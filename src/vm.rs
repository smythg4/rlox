use crate::chunk::{Chunk, OpCode};
use crate::compiler::Compiler;
use crate::lexer::Lexer;
use crate::value::Value;

pub enum InterpretResult {
    Ok,
    CompileError,
    RuntimeError,
}

pub struct Vm {
    chunk: Chunk,
    ip: usize,
    stack: Vec<Value>,
    toggle_tracing: bool,
}

impl Vm {
    pub fn new() -> Self {
        Vm {
            chunk: Chunk::default(),
            ip: 0,
            stack: Vec::new(),
            toggle_tracing: false,
        }
    }

    pub fn with_tracing(mut self) -> Self {
        self.toggle_tracing = true;
        self
    }

    pub fn interpret(&mut self, source: &str) -> InterpretResult {
        let lexer = Lexer::from(source);
        let mut compiler = Compiler::from(lexer);

        if let Ok(_chunk) = compiler.compile() {
            return InterpretResult::Ok;
        }

        InterpretResult::CompileError
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
                self.chunk
                    .disassemble_instruction(self.ip);
            }
            match OpCode::from(self.read_byte()) {
                OpCode::Return => {
                    let popped = self.stack.pop().unwrap();
                    println!("{popped}");
                    return InterpretResult::Ok;
                },
                OpCode::Constant => {
                    let constant = self.read_constant();
                    self.stack.push(constant);
                },
                OpCode::Negate => {
                    let popped = self.stack.pop().unwrap();
                    self.stack.push(-popped);
                },
                OpCode::Add=> {
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
                    self.stack.push(x + y);
                },
                OpCode::Subtract => {
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
                    self.stack.push(x - y);
                },
                OpCode::Multiply => {
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
                    self.stack.push(x * y);
                },
                OpCode::Divide => {
                    let x = self.stack.pop().unwrap();
                    let y = self.stack.pop().unwrap();
                    self.stack.push(x / y);
                },
                _ => return InterpretResult::CompileError,
            }
        }
    }
}
