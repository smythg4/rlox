use crate::value::Value;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    Constant,
    Add,
    Subtract,
    Multiply,
    Divide,
    Negate,
    Return,
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
            OpCode::Return => offset + self.simple_instruction(offset),
            OpCode::Constant => offset + self.constant_instruction(offset),
            OpCode::Negate | OpCode::Divide | OpCode::Multiply | OpCode::Subtract | OpCode::Add => {
                offset + self.simple_instruction(offset)
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
        assert!(self.constants.len() <= u8::MAX as usize, "constant overflow! Max constant count is {}", u8::MAX);
        let i = self.constants.len() as u8;
        self.constants.push(constant);
        i
    }
}
