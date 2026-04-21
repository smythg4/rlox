use crate::chunk::Chunk;
use std::cmp::{PartialEq, PartialOrd};
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy)]
pub enum Value {
    Number(f64),
    Boolean(bool),
    Obj(*mut Obj),
    Nil,
}

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

pub enum ObjKind {
    String(String),
    Function {
        arity: usize,
        name: String,
        chunk: Chunk,
    },
}

impl Value {
    pub unsafe fn as_string(&self) -> Option<&str> {
        unsafe {
            match self {
                Value::Obj(ptr) => (*(*ptr)).as_string(),
                _ => None,
            }
        }
    }
}
impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{n}"),
            Value::Boolean(b) => write!(f, "{b}"),
            Value::Obj(ptr) => {
                let obj = unsafe { &**ptr };
                match &obj.kind {
                    ObjKind::String(s) => write!(f, "{s}"),
                    ObjKind::Function { name, .. } if name.is_empty() => write!(f, "<script>"),
                    ObjKind::Function { name, .. } => write!(f, "<fn {name}>"),
                }
            }
            Value::Nil => write!(f, "nil"),
        }
    }
}

impl Neg for Value {
    type Output = Value;
    fn neg(self) -> Self::Output {
        match self {
            Value::Number(n) => Value::Number(-n),
            _ => panic!("can't negate a non-number"),
        }
    }
}

impl Add<Value> for Value {
    type Output = Value;
    fn add(self, rhs: Value) -> Self::Output {
        match (self, rhs) {
            (Value::Number(x), Value::Number(y)) => Value::Number(x + y),
            _ => panic!("can't add non-numbers or strings"),
        }
    }
}

impl Sub<Value> for Value {
    type Output = Value;
    fn sub(self, rhs: Value) -> Self::Output {
        match (self, rhs) {
            (Value::Number(x), Value::Number(y)) => Value::Number(x - y),
            _ => panic!("can't subtract non-numbers"),
        }
    }
}

impl Mul<Value> for Value {
    type Output = Value;
    fn mul(self, rhs: Value) -> Self::Output {
        match (self, rhs) {
            (Value::Number(x), Value::Number(y)) => Value::Number(x * y),
            _ => panic!("can't multiply non-numbers"),
        }
    }
}

impl Div<Value> for Value {
    type Output = Value;
    fn div(self, rhs: Value) -> Self::Output {
        match (self, rhs) {
            (Value::Number(x), Value::Number(y)) => Value::Number(x / y),
            _ => panic!("can't divide non-numbers"),
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Value::Number(x), Value::Number(y)) => x.partial_cmp(y),
            _ => None,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Number(x), Value::Number(y)) => x.eq(y),
            (Value::Boolean(x), Value::Boolean(y)) => x.eq(y),
            (Value::Obj(o1), Value::Obj(o2)) => o1.eq(o2),
            _ => false,
        }
    }
}
