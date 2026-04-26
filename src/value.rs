use crate::chunk::Chunk;
use crate::vecmap::VecMap;
use std::cmp::{PartialEq, PartialOrd};
use std::ops::{Add, Div, Mul, Neg, Sub};

use anyhow::Result;

#[cfg(feature = "nan-boxing")]
#[repr(u64)]
enum TypeTag {
    Nil = 1,
    False = 2,
    True = 3,
}

#[cfg(feature = "nan-boxing")]
const QNAN: u64 = 0x7FFC_0000_0000_0000;
#[cfg(feature = "nan-boxing")]
const SIGN_BIT: u64 = 0x8000_0000_0000_0000;
#[cfg(feature = "nan-boxing")]
const NIL_VAL: u64 = QNAN | TypeTag::Nil as u64;
#[cfg(feature = "nan-boxing")]
const FALSE_VAL: u64 = QNAN | TypeTag::False as u64;
#[cfg(feature = "nan-boxing")]
const TRUE_VAL: u64 = QNAN | TypeTag::True as u64;

#[cfg(feature = "nan-boxing")]
#[derive(Debug, Clone, Copy)]
pub struct Value(pub u64);

#[cfg(not(feature = "nan-boxing"))]
#[derive(Debug, Clone, Copy)]
pub enum Value {
    Number(f64),
    Boolean(bool),
    Obj(*mut Obj),
    Nil,
}

#[derive(Debug)]
pub struct Obj {
    pub kind: ObjKind,
    pub next: *mut Obj,
    pub marked: bool,
}

impl Obj {
    pub fn as_string(&self) -> Option<&str> {
        match &self.kind {
            ObjKind::String(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum ObjKind {
    String(String),
    Function {
        arity: usize,
        name: String,
        chunk: Chunk,
        upvalue_count: usize,
    },
    Native(fn(&[Value]) -> Result<Value>),
    Closure {
        function: *mut Obj,
        upvalues: Vec<*mut Obj>,
    },
    UpValue {
        location: *mut Value,
        closed: Value,
    },
    Class {
        name: String,
        methods: VecMap<*mut Obj, Value>,
    },
    ClassInstance {
        klass: *mut Obj,
        fields: VecMap<*mut Obj, Value>,
    },
    BoundMethod {
        receiver: Value,
        method: *mut Obj,
    },
}

impl ObjKind {
    pub fn heap_size(&self) -> usize {
        match self {
            ObjKind::String(s) => s.capacity(),
            ObjKind::Function { chunk, .. } => {
                chunk.codes.capacity()
                    + chunk.constants.capacity() * size_of::<Value>()
                    + chunk.lines.capacity() * size_of::<usize>()
            }
            ObjKind::Closure { upvalues, .. } => upvalues.capacity() * size_of::<*mut Obj>(),
            ObjKind::UpValue { .. } => 0,
            ObjKind::Class { name, methods } => {
                name.capacity()
                    + methods
                        .keys()
                        .map(|_key| size_of::<*mut Obj>() + size_of::<Value>())
                        .sum::<usize>()
            }
            ObjKind::ClassInstance { fields, .. } => fields
                .keys()
                .map(|_key| size_of::<*mut Obj>() + size_of::<Value>())
                .sum(),
            ObjKind::BoundMethod { .. } => size_of::<Value>(),
            ObjKind::Native(_) => 0,
        }
    }
}

impl std::fmt::Display for ObjKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            ObjKind::String(s) => write!(f, "{s}"),
            ObjKind::Function { name, .. } if name.is_empty() => write!(f, "<script>"),
            ObjKind::Function { name, .. } => write!(f, "<fn {name}>"),
            ObjKind::Closure { function: ptr, .. } => {
                // SAFETY: ObjKind::Closure::function always points to an ObjKind::Function;
                // this invariant is enforced at construction in OpCode::Closure.
                let ObjKind::Function { name, .. } = (unsafe { &(**ptr).kind }) else {
                    unreachable!()
                };
                write!(f, "<closure {name}>")
            }
            ObjKind::UpValue { .. } => write!(f, "upvalue"),
            ObjKind::Class { name, .. } => write!(f, "{name}"),
            ObjKind::ClassInstance { klass, .. } => {
                write!(f, "{} instance", unsafe { &**klass })
            }
            ObjKind::BoundMethod { method, .. } => write!(f, "{}", unsafe { &**method }),
            ObjKind::Native(_) => write!(f, "<native fn>"),
        }
    }
}

impl std::fmt::Display for Obj {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)
    }
}

#[cfg(not(feature = "nan-boxing"))]
impl Value {
    // constructors
    pub fn from_number(n: f64) -> Self {
        Value::Number(n)
    }
    pub fn from_bool(b: bool) -> Self {
        Value::Boolean(b)
    }
    pub fn from_obj(obj_ptr: *mut Obj) -> Self {
        Value::Obj(obj_ptr)
    }
    pub fn nil() -> Self {
        Value::Nil
    }

    // type checks
    pub fn is_number(self) -> bool {
        matches!(self, Value::Number(_))
    }
    pub fn is_bool(self) -> bool {
        matches!(self, Value::Boolean(_))
    }
    pub fn is_nil(self) -> bool {
        matches!(self, Value::Nil)
    }
    pub fn is_obj(self) -> bool {
        matches!(self, Value::Obj(_))
    }
    pub fn is_falsey(self) -> bool {
        matches!(self, Value::Nil | Value::Boolean(false))
    }

    // type conversions
    pub fn as_number(self) -> f64 {
        debug_assert!(self.is_number(), "must be a Value::Number");
        let Value::Number(n) = self else {
            unreachable!()
        };
        n
    }
    pub fn as_bool(self) -> bool {
        debug_assert!(self.is_bool(), "must be a Value::Boolean");
        let Value::Boolean(b) = self else {
            unreachable!()
        };
        b
    }
    pub fn as_obj(self) -> *mut Obj {
        debug_assert!(self.is_obj(), "must be a Value::Obj");
        let Value::Obj(ptr) = self else {
            unreachable!()
        };
        ptr
    }
    /// # Safety
    /// `ptr` inside `Value::Obj` must be non-null and point to a live `Obj`
    /// allocated by the VM's GC linked list.
    pub unsafe fn as_string(&self) -> Option<&str> {
        match self {
            // SAFETY: caller guarantees ptr is non-null and points to a live Obj.
            Value::Obj(ptr) => unsafe { (*(*ptr)).as_string() },
            _ => None,
        }
    }
}

#[cfg(feature = "nan-boxing")]
impl Value {
    // constructors
    pub fn from_number(n: f64) -> Self {
        Value(n.to_bits())
    }
    pub fn from_bool(b: bool) -> Self {
        Value(if b { TRUE_VAL } else { FALSE_VAL })
    }
    pub fn from_obj(ptr: *mut Obj) -> Self {
        Value(SIGN_BIT | QNAN | ptr as u64)
    }
    pub fn nil() -> Self {
        Value(NIL_VAL)
    }

    // type checks
    pub fn is_number(self) -> bool {
        (self.0 & QNAN) != QNAN
    }
    pub fn is_bool(self) -> bool {
        (self.0 | 1) == TRUE_VAL
    }
    pub fn is_nil(self) -> bool {
        self.0 == NIL_VAL
    }
    pub fn is_obj(self) -> bool {
        self.0 & (QNAN | SIGN_BIT) == (QNAN | SIGN_BIT)
    }
    pub fn is_falsey(self) -> bool {
        self.is_nil() || self.0 == FALSE_VAL
    }

    // type conversions
    pub fn as_number(self) -> f64 {
        f64::from_bits(self.0)
    }
    pub fn as_bool(self) -> bool {
        self.0 == TRUE_VAL
    }
    pub fn as_obj(self) -> *mut Obj {
        (self.0 & !(SIGN_BIT | QNAN)) as *mut Obj
    }
    pub unsafe fn as_string<'a>(self) -> Option<&'a str> {
        unsafe { (*self.as_obj()).as_string() }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_number() {
            write!(f, "{}", self.as_number())
        } else if self.is_nil() {
            write!(f, "nil")
        } else if self.is_bool() {
            write!(f, "{}", self.as_bool())
        } else {
            write!(f, "{}", unsafe { &*self.as_obj() })
        }
    }
}

impl Neg for Value {
    type Output = Value;
    fn neg(self) -> Self::Output {
        Value::from_number(-self.as_number())
    }
}

impl Add<Value> for Value {
    type Output = Value;
    fn add(self, rhs: Value) -> Self::Output {
        Value::from_number(self.as_number() + rhs.as_number())
    }
}

impl Sub<Value> for Value {
    type Output = Value;
    fn sub(self, rhs: Value) -> Self::Output {
        Value::from_number(self.as_number() - rhs.as_number())
    }
}

impl Mul<Value> for Value {
    type Output = Value;
    fn mul(self, rhs: Value) -> Self::Output {
        Value::from_number(self.as_number() * rhs.as_number())
    }
}

impl Div<Value> for Value {
    type Output = Value;
    fn div(self, rhs: Value) -> Self::Output {
        Value::from_number(self.as_number() / rhs.as_number())
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.as_number().partial_cmp(&other.as_number())
    }
}

impl PartialEq for Value {
    fn eq(&self, rhs: &Self) -> bool {
        if self.is_number() {
            self.as_number().eq(&rhs.as_number())
        } else if self.is_bool() {
            rhs.is_bool() && self.as_bool() == rhs.as_bool()
        } else if self.is_obj() {
            rhs.is_obj() && self.as_obj() == rhs.as_obj()
        } else {
            self.is_nil() && rhs.is_nil()
        }
    }
}
