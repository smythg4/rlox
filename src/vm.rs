use std::collections::HashMap;

use crate::chunk::{Chunk, OpCode};
use crate::compiler::Compiler;
use crate::lexer::Lexer;
use crate::value::{Obj, ObjKind, Value};

pub enum InterpretResult {
    Ok,
    CompileError,
    RuntimeError,
}

pub struct Vm {
    chunk: Chunk,
    ip: usize,
    stack: Vec<Value>,

    objects: *mut Obj,
    strings: HashMap<String, *mut Obj>,
    globals: HashMap<String, Value>,

    toggle_tracing: bool,
    toggle_debug_print: bool,
}

impl Vm {
    pub fn new() -> Self {
        Vm {
            chunk: Chunk::default(),
            ip: 0,
            stack: Vec::new(),

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
        self.chunk = Chunk::default();
        self.ip = 0;
        let lexer = Lexer::from(source);
        let mut compiler =
            Compiler::from(lexer, &mut self.chunk, &mut self.objects, &mut self.strings);
        if self.toggle_debug_print {
            compiler = compiler.with_debug();
        }

        match compiler.compile() {
            Ok(_) => self.run(),
            Err(_) => InterpretResult::CompileError,
        }
    }

    fn peek_stack(&self, depth: usize) -> &Value {
        &self.stack[self.stack.len() - 1 - depth]
    }

    fn reset_stack(&mut self) {
        self.stack.clear();
    }

    fn read_byte(&mut self) -> u8 {
        let b = self.chunk.codes[self.ip];
        self.ip += 1;
        b
    }

    fn read_constant(&mut self) -> Value {
        let offset = self.read_byte() as usize;
        self.chunk.constants.get(offset).unwrap().clone()
    }

    pub fn run(&mut self) -> InterpretResult {
        loop {
            if self.toggle_tracing {
                print!("      ");
                self.stack.iter().for_each(|val| print!("[{val}]"));
                print!("\n");
                self.chunk.disassemble_instruction(self.ip);
            }
            match OpCode::from(self.read_byte()) {
                OpCode::Return => {
                    // let popped = self.stack.pop().unwrap();
                    // println!("{popped}");
                    return InterpretResult::Ok;
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
                },
                OpCode::DefineGlobal => {
                    // read the string `ObjString* name = READ_STRING();`
                    let name = self.read_constant();
                    let value = self.peek_stack(0).clone();
                    //set the global table `tableSet(&vm.globals, name, peek(0));`
                    let key = unsafe { name.as_string() }.unwrap().to_string();
                    self.globals.insert(key, value);
                    let _ = self.stack.pop(); // pop the stack
                },
                OpCode::GetGlobal => {
                    let name = self.read_constant();
                    let key = unsafe { name.as_string() }.unwrap();
                    if let Some(val) = self.globals.get(key) {
                        self.stack.push(val.clone());
                    } else {
                        self.runtime_error(&format!("Undefined variable '{}'", name));
                        return InterpretResult::RuntimeError;
                    }
                },
                OpCode::Equal => {
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
                    self.stack.push(Value::Boolean(x == y));
                }
                OpCode::Greater => {
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
                    self.stack.push(Value::Boolean(x > y));
                }
                OpCode::Less => {
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
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
                        let x = self.stack.pop().unwrap();
                        let y = self.stack.pop().unwrap();
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
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
                    self.stack.push(x - y);
                }
                OpCode::Multiply => {
                    let peeks = (self.peek_stack(0), self.peek_stack(1));
                    if !matches!(peeks, (Value::Number(_), Value::Number(_))) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
                    self.stack.push(x * y);
                }
                OpCode::Divide => {
                    let peeks = (self.peek_stack(0), self.peek_stack(1));
                    if !matches!(peeks, (Value::Number(_), Value::Number(_))) {
                        self.runtime_error("operands must be numbers");
                        return InterpretResult::RuntimeError;
                    }
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
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
        let line = self.chunk.lines[self.ip - 1];
        eprintln!("{msg}");
        eprintln!("[line {line}] in script");
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
