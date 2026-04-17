use crate::chunk::{OpCode, Chunk};
use crate::value::Value;
use crate::lexer::{Lexer, Token, TokenKind};
use crate::vm::InterpretResult;

use anyhow::{bail, Result};

enum Precedence {
    None,
    Assignment,
    Or,
    And,
    Equality,
    Comparison,
    Term,
    Factor,
    Unary,
    Call,
    Primary,
}

pub struct Compiler<'a> {
    lexer: Lexer<'a>,
    current: Token,
    previous: Token,

    compiling_chunk: Chunk,

    had_error: bool,
    panic_mode: bool,
}

impl<'a> Compiler<'a> {
    pub fn compile(&mut self) -> Result<InterpretResult> {       
        self.advance();
        self.expression();
        self.consume(TokenKind::Eof, "Expect end of expression");
        self.end_compiler();
        if self.had_error {
            bail!("compiler error");
        }
        Ok(InterpretResult::Ok)
    }

    fn advance(&mut self) {
        //self.previous = self.current;
        std::mem::swap(&mut self.previous, &mut self.current);
        loop {
            self.current = self.lexer.scan_token();
            if self.current.kind != TokenKind::Error {
                break;
            } else {
                self.error_at_current(self.current.lexeme.to_string());
            }
        }
    }

    fn error_at_current(&mut self, msg: String) {
        self.error_at(self.current.clone(), msg.clone())
    }

    fn error(&mut self, msg: String) {
        self.error_at(self.previous.clone(), msg)
    }

    fn error_at(&mut self, token: Token, msg: String) {
        if self.panic_mode { return; }
        self.panic_mode = true;
        eprint!("[line {}] Error", token.line);

        if token.kind == TokenKind::Eof {
            eprint!(" at end");
        } else if token.kind == TokenKind::Error {

        } else {
            eprint!(" at '{}'", token.lexeme);
        }

        eprintln!(": {msg}");
        self.had_error = true;
    }

    fn consume(&mut self, kind: TokenKind, msg: &str) {
        if self.current.kind == kind {
            self.advance();
            return;
        }

        self.error_at_current(msg.into());
    }

    fn emit_byte(&mut self, byte: u8) {
        self.compiling_chunk.write_chunk(byte, self.previous.line);
    }

    fn emit_bytes(&mut self, byte1: u8, byte2: u8) {
        self.emit_byte(byte1);
        self.emit_byte(byte2);
    }

    fn end_compiler(&mut self) {
        self.emit_return();
    }

    fn expression(&mut self) {
        self.parse_precedence(Precedence::Assignment)
    }

    fn emit_return(&mut self) {
        self.emit_byte(OpCode::Return as u8);
    }

    fn grouping(&mut self) {
        self.expression();
        self.consume(TokenKind::RightParen, "Expect ')' after expression.");
    }

    fn number(&mut self) {
        let value = self.previous.lexeme.parse::<f64>().unwrap();
        self.emit_constant(Value::Number(value));
    }
    
    fn unary(&mut self) {
        let kind = self.previous.kind;

        self.parse_precedence(Precedence::Unary);

        match kind {
            TokenKind::Minus => self.emit_byte(OpCode::Negate as u8),
            _ => unreachable!(),
        }
    }

    fn emit_constant(&mut self, value: Value) {
        let const_i = self.make_constant(value);
        self.emit_bytes(OpCode::Constant as u8, const_i);
    }

    fn make_constant(&mut self, value: Value) -> u8 {
        let const_i = self.compiling_chunk.add_constant(value);
        if const_i > u8::MAX {
            self.error("too many constants for one chunk".into());
            return 0;
        }
        const_i
    }
}

impl<'a> From<Lexer<'a>> for Compiler<'a> {
    fn from(lexer: Lexer<'a>) -> Self {
        Compiler {
            lexer,
            current: Token { kind: TokenKind::Nil, lexeme: "".into(), line: 0},
            previous: Token { kind: TokenKind::Nil, lexeme: "".into(), line: 0},
            compiling_chunk: Chunk::default(),
            had_error: false,
            panic_mode: false,
        }
    }
}