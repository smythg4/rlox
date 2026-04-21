use std::collections::HashMap;

use crate::chunk::{Chunk, OpCode};
use crate::compiler::Compiler;
use crate::lexer::Lexer;
use crate::value::{Obj, ObjKind, Value};

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

    toggle_tracing: bool,
    toggle_debug_print: bool,
}

impl Vm {
    pub fn new() -> Self {
        Vm {
            stack: Vec::with_capacity(STACK_MAX),
            frames: Vec::with_capacity(FRAMES_MAX),

            objects: std::ptr::null_mut(),
            strings: HashMap::new(),
            globals: HashMap::new(),

            toggle_tracing: false,
            toggle_debug_print: false,
        }
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

    fn peek_stack(&self, depth: usize) -> &Value {
        &self.stack[self.stack.len() - 1 - depth]
    }

    fn call_value(&mut self, callee: Value, arg_count: u8) -> bool {
        if let Value::Obj(ptr) = callee
            && matches!(unsafe { &(*ptr).kind }, ObjKind::Function { .. })
        {
            return self.call(ptr, arg_count);
        }
        self.runtime_error("Can only call functions and classes.");
        false
    }

    fn call(&mut self, func_ptr: *mut Obj, arg_count: u8) -> bool {
        let arity = unsafe {
            let ObjKind::Function { arity, .. } = &(*func_ptr).kind else {
                unreachable!()
            };
            *arity
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

    fn reset_stack(&mut self) {
        self.frames.clear();
        self.stack.clear();
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

    fn current_chunk(&self) -> &Chunk {
        let object = &self.current_func().kind;
        let ObjKind::Function { chunk, .. } = object else {
            unreachable!()
        };
        chunk
    }

    fn current_func(&self) -> &Obj {
        let frame = self.frames.last().unwrap();
        unsafe { &*(frame.function) }
    }

    fn current_func_mut(&mut self) -> &mut Obj {
        let frame = self.frames.last_mut().unwrap();
        unsafe { &mut *(frame.function) }
    }

    fn current_ip(&self) -> usize {
        let frame = self.frames.last().unwrap();
        frame.ip
    }

    fn current_ip_mut(&mut self) -> &mut usize {
        let frame = self.frames.last_mut().unwrap();
        &mut frame.ip
    }

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
                    // unconditional jump
                    let offset = self.read_short() as usize;
                    *self.current_ip_mut() += offset;
                }
                OpCode::JumpIfFalse => {
                    let offset = self.read_short() as usize;
                    if self.is_falsey(*self.peek_stack(0)) {
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
                OpCode::Return => {
                    let result = self.stack.pop().unwrap();
                    let bp = self.frames.pop().unwrap().base_pointer;
                    if self.frames.is_empty() {
                        let _ = self.stack.pop(); // pop off the script function
                        return InterpretResult::Ok;
                    }
                    self.stack.truncate(bp); // chop off parts of the stack called function was using.
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
                    let _ = self.stack.pop();
                }
                OpCode::GetLocal => {
                    let slot = self.read_byte() as usize;
                    let base = self.frames.last().unwrap().base_pointer;
                    self.stack.push(self.stack[base + slot]);
                }
                OpCode::DefineGlobal => {
                    // read the string `ObjString* name = READ_STRING();`
                    let name = self.read_constant();
                    let value = self.peek_stack(0);
                    //set the global table `tableSet(&vm.globals, name, peek(0));`
                    let key = unsafe { name.as_string() }.unwrap().to_string();
                    self.globals.insert(key, *value);
                    let _ = self.stack.pop(); // pop the stack
                }
                OpCode::SetGlobal => {
                    let name = self.read_constant();

                    let key = unsafe { name.as_string() }.unwrap();
                    if self.globals.contains_key(key) {
                        let value = self.peek_stack(0);
                        self.globals.insert(key.to_string(), *value);
                    } else {
                        self.runtime_error(&format!("Undefined variable '{}'", name));
                        return InterpretResult::RuntimeError;
                    }
                }
                OpCode::SetLocal => {
                    let slot = self.read_byte() as usize;
                    let base = self.frames.last().unwrap().base_pointer;
                    self.stack[base + slot] = *self.peek_stack(0);
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
                    let peeks = (self.peek_stack(0), self.peek_stack(1));
                    if !matches!(peeks, (Value::Number(_), Value::Number(_))) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(x - y);
                }
                OpCode::Multiply => {
                    let peeks = (self.peek_stack(0), self.peek_stack(1));
                    if !matches!(peeks, (Value::Number(_), Value::Number(_))) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(x * y);
                }
                OpCode::Divide => {
                    let peeks = (self.peek_stack(0), self.peek_stack(1));
                    if !matches!(peeks, (Value::Number(_), Value::Number(_))) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let y = self.stack.pop().unwrap();
                    let x = self.stack.pop().unwrap();
                    self.stack.push(x / y);
                }
                OpCode::Not => {
                    let top = self.stack.pop().unwrap();
                    let value = self.is_falsey(top);
                    self.stack.push(Value::Boolean(value));
                }
                OpCode::Print => {
                    let top = self.stack.pop().unwrap();
                    println!("{top}");
                } //_ => return InterpretResult::CompileError,
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
        let obj1 = unsafe { &*ptr1 };
        let obj2 = unsafe { &*ptr2 };
        match (&obj1.kind, &obj2.kind) {
            (ObjKind::String(s1), ObjKind::String(s2)) => {
                let result = s1.clone() + s2.as_str();
                if let Some(&ptr) = self.strings.get(&result) {
                    self.stack.push(Value::Obj(ptr)); // push the result on the stack
                    return true;
                }
                let new_obj = Box::new(Obj {
                    kind: ObjKind::String(result.clone()),
                    next: self.objects,
                    marked: false,
                });
                let ptr = Box::into_raw(new_obj);
                self.strings.insert(result, ptr); // intern the string
                self.objects = ptr; // update the GC linked list
                self.stack.push(Value::Obj(ptr)); // push the result on the stack
            }
            _ => panic!("invalid object types!"),
        }
        true
    }

    fn is_falsey(&self, val: Value) -> bool {
        match val {
            Value::Nil => true,
            Value::Boolean(b) => !b,
            _ => false,
        }
    }

    fn runtime_error(&mut self, msg: &str) {
        eprintln!("{msg}");
        for frame in self.frames.iter().rev() {
            let ObjKind::Function { name, chunk, .. } = &unsafe { &*frame.function }.kind else {
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

impl Drop for Vm {
    // not super necessary since this is when the process would
    // end anyhow
    fn drop(&mut self) {
        let mut obj = self.objects;
        while !obj.is_null() {
            let next = unsafe { (*obj).next };
            drop(unsafe { Box::from_raw(obj) });
            obj = next;
        }
    }
}
