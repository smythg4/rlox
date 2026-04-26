use crate::value::{ObjKind, Value};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    Constant,
    Nil,
    True,
    False,
    Pop,
    GetLocal,
    SetLocal,
    GetGlobal,
    DefineGlobal,
    SetGlobal,
    GetUpValue,
    SetUpValue,
    SetProperty,
    GetProperty,
    GetSuper,
    Equal,
    Greater,
    Less,
    Add,
    Subtract,
    Multiply,
    Divide,
    Not,
    Negate,
    Print,
    Jump,
    JumpIfFalse,
    Loop,
    Call,
    Invoke,
    SuperInvoke,
    Closure,
    CloseUpvalue,
    Return,
    Class,
    Inherit,
    Method,
}

impl std::fmt::Display for OpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let upper = format!("{:?}", self).to_uppercase();
        write!(f, "OP_{}", upper)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Chunk {
    pub codes: Vec<u8>,
    pub constants: Vec<Value>,
    pub lines: Vec<usize>,
}

impl From<OpCode> for u8 {
    fn from(value: OpCode) -> Self {
        value as u8
    }
}

impl From<u8> for OpCode {
    fn from(value: u8) -> Self {
        unsafe { std::mem::transmute::<u8, OpCode>(value) }
    }
}

impl Chunk {
    pub fn write_chunk(&mut self, code: u8, line: usize) {
        self.lines.push(line);
        self.codes.push(code)
    }

    pub fn disassemble_chunk(&self, label: &str) {
        println!("== {} ==", label);
        let mut offset = 0;
        while offset < self.codes.len() {
            offset = self.disassemble_instruction(offset);
        }
    }

    pub fn disassemble_instruction(&self, offset: usize) -> usize {
        print!("{:04} ", offset);
        if offset > 0 && self.lines[offset] == self.lines[offset - 1] {
            print!("   | ");
        } else {
            print!("{:4} ", self.lines[offset]);
        }
        let op_code = OpCode::from(self.codes[offset]);
        match op_code {
            OpCode::Constant
            | OpCode::DefineGlobal
            | OpCode::GetGlobal
            | OpCode::SetGlobal
            | OpCode::GetProperty
            | OpCode::SetProperty
            | OpCode::Class
            | OpCode::Method
            | OpCode::GetSuper => offset + self.constant_instruction(offset),
            OpCode::Return
            | OpCode::False
            | OpCode::True
            | OpCode::Nil
            | OpCode::Not
            | OpCode::Equal
            | OpCode::Greater
            | OpCode::Less
            | OpCode::Print
            | OpCode::Pop
            | OpCode::Negate
            | OpCode::Divide
            | OpCode::Multiply
            | OpCode::Subtract
            | OpCode::Add
            | OpCode::CloseUpvalue
            | OpCode::Inherit => offset + self.simple_instruction(offset),
            OpCode::GetLocal
            | OpCode::SetLocal
            | OpCode::Call
            | OpCode::GetUpValue
            | OpCode::SetUpValue => offset + self.byte_instruction(offset),
            OpCode::Jump | OpCode::JumpIfFalse => offset + self.jump_instruction(1, offset),
            OpCode::Loop => offset + self.jump_instruction(-1, offset),
            OpCode::Invoke | OpCode::SuperInvoke => offset + self.invoke_instruction(offset),
            OpCode::Closure => {
                let const_idx = self.codes[offset + 1] as usize;
                print!("{:<16} {} ", op_code, const_idx);
                println!("{}", self.constants[const_idx]);

                // SAFETY: the complier always places an ObjKind::Function as the Closure
                // constant operand; the pointer is live for the VM's lifetime
                let upvalue_count = if self.constants[const_idx].is_obj() {
                    let ptr = self.constants[const_idx].as_obj();
                    unsafe {
                        if let ObjKind::Function { upvalue_count, .. } = &(*ptr).kind {
                            *upvalue_count
                        } else {
                            0
                        }
                    }
                } else {
                    0
                };

                for i in 0..upvalue_count {
                    let base = offset + 2 + i * 2;
                    let is_local = self.codes[base] != 0;
                    let index = self.codes[base + 1];
                    let kind = if is_local { "local" } else { "upvalue" };
                    println!("{:04}      |                     {} {}", base, kind, index);
                }
                offset + 2 + upvalue_count * 2
            }
        }
    }

    fn simple_instruction(&self, offset: usize) -> usize {
        let op_code = OpCode::from(self.codes[offset]);
        println!("{}", op_code);
        1 // bytes consumed
    }

    fn constant_instruction(&self, offset: usize) -> usize {
        let op_code = OpCode::from(self.codes[offset]);
        let const_idx = self.codes[offset + 1] as usize;
        println!(
            "{:<16} {} '{}'",
            op_code, const_idx, self.constants[const_idx]
        );
        2 // bytes consumed
    }

    pub fn add_constant(&mut self, constant: Value) -> u8 {
        assert!(
            self.constants.len() <= u8::MAX as usize,
            "constant overflow! Max constant count is {}",
            u8::MAX
        );
        let i = self.constants.len() as u8;
        self.constants.push(constant);
        i
    }

    fn byte_instruction(&self, offset: usize) -> usize {
        let op_code = OpCode::from(self.codes[offset]);
        let slot = self.codes[offset + 1] as usize;
        println!("{:<16} {}", op_code, slot);
        2
    }

    fn jump_instruction(&self, sign: isize, offset: usize) -> usize {
        assert!(offset < self.codes.len() + 2);
        let op_code = OpCode::from(self.codes[offset]);
        let bh = (self.codes[offset + 1] as u16) << 8;
        let bl = self.codes[offset + 2] as u16;
        let jump = (bh | bl) as isize;
        println!(
            "{:<16} {:4} -> {}",
            op_code,
            offset,
            offset as isize + 3 + sign * jump
        );
        3
    }

    fn invoke_instruction(&self, offset: usize) -> usize {
        let op_code = OpCode::from(self.codes[offset]);
        let const_idx = self.codes[offset + 1] as usize;
        let arg_count = self.codes[offset + 2];
        println!(
            "{:<16} ({} args) {:04} '{}'",
            op_code, arg_count, const_idx, self.constants[const_idx]
        );
        3
    }
}
