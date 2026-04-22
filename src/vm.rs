use std::collections::HashMap;

use crate::chunk::{Chunk, OpCode};
use crate::compiler::Compiler;
use crate::lexer::Lexer;
use crate::value::{Obj, ObjKind, Value};

use anyhow::{Result, bail};

const FRAMES_MAX: usize = 64;
const STACK_MAX: usize = FRAMES_MAX * u8::MAX as usize;

pub enum InterpretResult {
    Ok,
    CompileError,
    RuntimeError,
}

struct CallFrame {
    function: *mut Obj,
    ip: usize,
    base_pointer: usize, // index into vm.stack where this frame's locals start
}

pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<CallFrame>,

    objects: *mut Obj,
    strings: HashMap<String, *mut Obj>,
    globals: HashMap<String, Value>,
    // consider adding
    // natives: HashMap<String, Arc<dyn Fn(&[Value]) -> Result<Value>>>
    // this would add the ability to use closures as native functions
    toggle_tracing: bool,
    toggle_debug_print: bool,
}

// =============================================================================
// Lifecycle
// =============================================================================

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        let mut vm = Vm {
            stack: Vec::with_capacity(STACK_MAX),
            frames: Vec::with_capacity(FRAMES_MAX),

            objects: std::ptr::null_mut(),
            strings: HashMap::new(),
            globals: HashMap::new(),

            toggle_tracing: false,
            toggle_debug_print: false,
        };

        vm.define_native("clock", |_args| {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();
            Ok(Value::Number(secs))
        });

        vm.define_native("sqrt", |args| {
            if args.len() != 1 {
                bail!("'sqrt' only accepts 1 argument, got: {}", args.len());
            }
            let Value::Number(x) = args[0] else {
                bail!("'sqrt' only accepts Numbers")
            };
            Ok(Value::Number(x.sqrt()))
        });
        vm
    }

    pub fn with_tracing(mut self) -> Self {
        self.toggle_tracing = true;
        self
    }

    pub fn with_debug(mut self) -> Self {
        self.toggle_debug_print = true;
        self
    }

    pub fn interpret(&mut self, source: &str) -> InterpretResult {
        let lexer = Lexer::from(source);
        let mut compiler = Compiler::new(lexer, &mut self.objects, &mut self.strings);
        if self.toggle_debug_print {
            compiler = compiler.with_debug();
        }

        match compiler.compile() {
            Ok(func_obj) => {
                self.stack.push(Value::Obj(func_obj));
                self.call(func_obj, 0);
                self.run()
            }
            Err(_) => InterpretResult::CompileError,
        }
    }

    fn reset_stack(&mut self) {
        self.frames.clear();
        self.stack.clear();
    }

    fn define_native(&mut self, name: &str, native_func: fn(&[Value]) -> Result<Value>) {
        self.globals
            .insert(name.to_string(), Value::NativeFunction(native_func));
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        let mut obj = self.objects;
        while !obj.is_null() {
            let next = unsafe { (*obj).next };
            drop(unsafe { Box::from_raw(obj) });
            obj = next;
        }
    }
}

// =============================================================================
// Frame
// =============================================================================

impl Vm {
    fn resolve_function(ptr: *mut Obj) -> *mut Obj {
        unsafe {
            match &(*ptr).kind {
                ObjKind::Function { .. } => ptr,
                ObjKind::Closure { function } => *function,
                _ => unreachable!(),
            }
        }
    }

    fn current_func(&self) -> &Obj {
        unsafe { &*Self::resolve_function(self.frames.last().unwrap().function) }
    }

    fn current_chunk(&self) -> &Chunk {
        let ObjKind::Function { chunk, .. } = &self.current_func().kind else {
            unreachable!()
        };
        chunk
    }

    fn current_ip(&self) -> usize {
        self.frames.last().unwrap().ip
    }

    fn current_ip_mut(&mut self) -> &mut usize {
        &mut self.frames.last_mut().unwrap().ip
    }

    fn peek_stack(&self, depth: usize) -> &Value {
        &self.stack[self.stack.len() - 1 - depth]
    }

    fn read_byte(&mut self) -> u8 {
        let b = self.current_chunk().codes[self.current_ip()];
        *self.current_ip_mut() += 1;
        b
    }

    fn read_short(&mut self) -> u16 {
        let chunk = self.current_chunk();
        assert!(
            self.current_ip() < chunk.codes.len() - 1,
            "can't read two bytes when ip is so high"
        );
        let bh = chunk.codes[self.current_ip()] as u16;
        let bl = chunk.codes[self.current_ip() + 1] as u16;
        let val = bh << 8 | bl;
        *self.current_ip_mut() += 2;
        val
    }

    fn read_constant(&mut self) -> Value {
        let offset = self.read_byte() as usize;
        self.current_chunk().constants[offset]
    }
}

// =============================================================================
// Call
// =============================================================================

impl Vm {
    fn call(&mut self, func_ptr: *mut Obj, arg_count: u8) -> bool {
        let arity = unsafe {
            match &(*func_ptr).kind {
                ObjKind::Function { arity, .. } => *arity,
                ObjKind::Closure { function } => {
                    let ObjKind::Function { arity, .. } = &(**function).kind else {
                        unreachable!()
                    };
                    *arity
                }
                _ => unreachable!(),
            }
        };
        if arg_count as usize != arity {
            self.runtime_error(&format!(
                "Expected {} arguments but got {}.",
                arity, arg_count
            ));
            return false;
        }
        if self.frames.len() == FRAMES_MAX {
            self.runtime_error("Stack overflow.");
            return false;
        }
        let base_pointer = self.stack.len() - arg_count as usize - 1;
        self.frames.push(CallFrame {
            function: func_ptr,
            ip: 0,
            base_pointer,
        });
        true
    }

    fn call_value(&mut self, callee: Value, arg_count: u8) -> bool {
        if let Value::Obj(ptr) = callee
            && matches!(unsafe { &(*ptr).kind }, ObjKind::Function { .. })
        {
            return self.call(ptr, arg_count);
        }
        if let Value::NativeFunction(native) = callee {
            let args = &self.stack[self.stack.len() - arg_count as usize..];
            match native(args) {
                Ok(val) => {
                    self.stack
                        .truncate(self.stack.len() - arg_count as usize - 1);
                    self.stack.push(val);
                    return true;
                }
                Err(e) => {
                    self.runtime_error(&format!("Error in native function: {e}"));
                    return false;
                }
            }
        }
        if let Value::Obj(ptr) = callee
            && let ObjKind::Closure { .. } = unsafe { &(*ptr).kind }
        {
            return self.call(ptr, arg_count);
        }
        self.runtime_error("Can only call functions, closures, and classes.");
        false
    }
}

// =============================================================================
// Execution
// =============================================================================

impl Vm {
    pub fn run(&mut self) -> InterpretResult {
        loop {
            if self.toggle_tracing {
                print!("      ");
                self.stack.iter().for_each(|val| print!("[{val}]"));
                println!();
                self.current_chunk()
                    .disassemble_instruction(self.current_ip());
            }
            match OpCode::from(self.read_byte()) {
                OpCode::Jump => {
                    let offset = self.read_short() as usize;
                    *self.current_ip_mut() += offset;
                }
                OpCode::JumpIfFalse => {
                    let offset = self.read_short() as usize;
                    if is_falsey(*self.peek_stack(0)) {
                        *self.current_ip_mut() += offset;
                    }
                }
                OpCode::Loop => {
                    let offset = self.read_short() as usize;
                    *self.current_ip_mut() -= offset;
                }
                OpCode::Call => {
                    let arg_count = self.read_byte();
                    let callee = *self.peek_stack(arg_count as usize);
                    if !self.call_value(callee, arg_count) {
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::Closure => {
                    let Value::Obj(ptr) = self.read_constant() else {
                        unreachable!()
                    };
                    assert!(matches!(&unsafe { &*ptr }.kind, ObjKind::Function { .. }));
                    let closure = Box::into_raw(Box::new(Obj {
                        kind: ObjKind::Closure { function: ptr },
                        next: self.objects,
                        marked: false,
                    }));
                    self.objects = closure;
                    self.stack.push(Value::Obj(closure));
                }
                OpCode::Return => {
                    let result = self.stack.pop().unwrap();
                    let bp = self.frames.pop().unwrap().base_pointer;
                    if self.frames.is_empty() {
                        self.stack.pop(); // pop the script function
                        return InterpretResult::Ok;
                    }
                    self.stack.truncate(bp);
                    self.stack.push(result);
                }
                OpCode::Constant => {
                    let constant = self.read_constant();
                    self.stack.push(constant);
                }
                OpCode::Nil => self.stack.push(Value::Nil),
                OpCode::True => self.stack.push(Value::Boolean(true)),
                OpCode::False => self.stack.push(Value::Boolean(false)),
                OpCode::Pop => {
                    self.stack.pop();
                }
                OpCode::GetLocal => {
                    let slot = self.read_byte() as usize;
                    let base = self.frames.last().unwrap().base_pointer;
                    self.stack.push(self.stack[base + slot]);
                }
                OpCode::SetLocal => {
                    let slot = self.read_byte() as usize;
                    let base = self.frames.last().unwrap().base_pointer;
                    self.stack[base + slot] = *self.peek_stack(0);
                }
                OpCode::DefineGlobal => {
                    let name = self.read_constant();
                    let key = unsafe { name.as_string() }.unwrap().to_string();
                    self.globals.insert(key, *self.peek_stack(0));
                    self.stack.pop();
                }
                OpCode::GetGlobal => {
                    let name = self.read_constant();
                    let key = unsafe { name.as_string() }.unwrap();
                    if let Some(val) = self.globals.get(key) {
                        self.stack.push(*val);
                    } else {
                        self.runtime_error(&format!("Undefined variable '{}'", name));
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::SetGlobal => {
                    let name = self.read_constant();
                    let key = unsafe { name.as_string() }.unwrap();
                    if self.globals.contains_key(key) {
                        self.globals.insert(key.to_string(), *self.peek_stack(0));
                    } else {
                        self.runtime_error(&format!("Undefined variable '{}'", name));
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::Equal => {
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(Value::Boolean(x == y));
                }
                OpCode::Greater => {
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(Value::Boolean(x > y));
                }
                OpCode::Less => {
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(Value::Boolean(x < y));
                }
                OpCode::Negate => {
                    if !matches!(self.peek_stack(0), Value::Number(_)) {
                        self.runtime_error("operand must be a number");
                        return InterpretResult::RuntimeError;
                    }
                    let popped = self.stack.pop().unwrap();
                    self.stack.push(-popped);
                }
                OpCode::Add => {
                    let both_numbers = matches!(
                        (self.peek_stack(0), self.peek_stack(1)),
                        (Value::Number(_), Value::Number(_))
                    );
                    let both_objects = matches!(
                        (self.peek_stack(0), self.peek_stack(1)),
                        (Value::Obj(_), Value::Obj(_))
                    );
                    if both_numbers {
                        let y = self.stack.pop().unwrap();
                        let x = self.stack.pop().unwrap();
                        self.stack.push(x + y);
                    } else if both_objects {
                        if !self.concatenate() {
                            self.runtime_error("operands must be two strings");
                            return InterpretResult::RuntimeError;
                        }
                    } else {
                        self.runtime_error("operands must be numbers or strings");
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::Subtract => {
                    if !matches!(
                        (self.peek_stack(0), self.peek_stack(1)),
                        (Value::Number(_), Value::Number(_))
                    ) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(x - y);
                }
                OpCode::Multiply => {
                    if !matches!(
                        (self.peek_stack(0), self.peek_stack(1)),
                        (Value::Number(_), Value::Number(_))
                    ) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(x * y);
                }
                OpCode::Divide => {
                    if !matches!(
                        (self.peek_stack(0), self.peek_stack(1)),
                        (Value::Number(_), Value::Number(_))
                    ) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(x / y);
                }
                OpCode::Not => {
                    let top = self.stack.pop().unwrap();
                    self.stack.push(Value::Boolean(is_falsey(top)));
                }
                OpCode::Print => {
                    println!("{}", self.stack.pop().unwrap());
                }
            }
        }
    }

    fn concatenate(&mut self) -> bool {
        let (p1, p2) = match (self.peek_stack(0), self.peek_stack(1)) {
            (Value::Obj(p1), Value::Obj(p2)) => (*p1, *p2),
            _ => return false,
        };
        let is_strings = unsafe {
            matches!(
                (&(*p1).kind, &(*p2).kind),
                (ObjKind::String(_), ObjKind::String(_))
            )
        };
        if !is_strings {
            return false;
        }

        let Value::Obj(ptr2) = self.stack.pop().unwrap() else {
            panic!("invalid obj type")
        };
        let Value::Obj(ptr1) = self.stack.pop().unwrap() else {
            panic!("invalid obj type")
        };
        let (obj1, obj2) = unsafe { (&*ptr1, &*ptr2) };
        match (&obj1.kind, &obj2.kind) {
            (ObjKind::String(s1), ObjKind::String(s2)) => {
                let result = s1.clone() + s2.as_str();
                if let Some(&ptr) = self.strings.get(&result) {
                    self.stack.push(Value::Obj(ptr));
                    return true;
                }
                let ptr = Box::into_raw(Box::new(Obj {
                    kind: ObjKind::String(result.clone()),
                    next: self.objects,
                    marked: false,
                }));
                self.strings.insert(result, ptr);
                self.objects = ptr;
                self.stack.push(Value::Obj(ptr));
            }
            _ => panic!("invalid object types!"),
        }
        true
    }

    fn runtime_error(&mut self, msg: &str) {
        eprintln!("{msg}");
        for frame in self.frames.iter().rev() {
            let func_ptr = Self::resolve_function(frame.function);
            let ObjKind::Function { name, chunk, .. } = &unsafe { &*func_ptr }.kind else {
                unreachable!()
            };
            let line = chunk.lines[frame.ip - 1];
            if name.is_empty() {
                eprintln!("[line {line}] in script");
            } else {
                eprintln!("[line {line}] in {name}()");
            }
        }
        self.reset_stack();
    }
}

fn is_falsey(val: Value) -> bool {
    matches!(val, Value::Nil | Value::Boolean(false))
}
