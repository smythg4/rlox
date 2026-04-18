use std::collections::HashMap;

use crate::chunk::{Chunk, OpCode};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::value::{Obj, ObjKind, Value};
use crate::vm::InterpretResult;

use anyhow::{Result, bail};

#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

impl std::ops::Add<usize> for Precedence {
    type Output = Precedence;
    fn add(self, rhs: usize) -> Precedence {
        // it would be safer to implement a `.next_higher()` on `Precedence`
        // to avoid catastrophe
        unsafe { std::mem::transmute(self as usize + rhs) }
    }
}

impl From<TokenKind> for Precedence {
    fn from(kind: TokenKind) -> Self {
        match kind {
            TokenKind::Plus | TokenKind::Minus => Precedence::Term,
            TokenKind::Star | TokenKind::Slash => Precedence::Factor,
            TokenKind::BangEqual | TokenKind::EqualEqual => Precedence::Equality,
            TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::Less
            | TokenKind::LessEqual => Precedence::Comparison,
            _ => Precedence::None,
        }
    }
}

pub struct Compiler<'a> {
    lexer: Lexer<'a>,
    current: Token,
    previous: Token,

    compiling_chunk: &'a mut Chunk,

    objects: &'a mut *mut Obj,
    strings: &'a mut HashMap<String, *mut Obj>,

    had_error: bool,
    panic_mode: bool,
    debug_print: bool,
}

impl<'a> Compiler<'a> {
    pub fn with_debug(mut self) -> Self {
        self.debug_print = true;
        self
    }

    pub fn from(
        lexer: Lexer<'a>,
        chunk: &'a mut Chunk,
        objects: &'a mut *mut Obj,
        strings: &'a mut HashMap<String, *mut Obj>,
    ) -> Self {
        Compiler {
            lexer,
            current: Token {
                kind: TokenKind::Nil,
                lexeme: "".into(),
                line: 0,
            },
            previous: Token {
                kind: TokenKind::Nil,
                lexeme: "".into(),
                line: 0,
            },
            compiling_chunk: chunk,
            objects,
            strings,

            had_error: false,
            panic_mode: false,
            debug_print: false,
        }
    }

    pub fn compile(&mut self) -> Result<InterpretResult> {
        self.advance();

        while !self.peek_match(TokenKind::Eof) {
            self.declaration();
        }

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
        if self.panic_mode {
            return;
        }
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

    fn peek_match(&mut self, kind: TokenKind) -> bool {
        if self.current.kind == kind {
            self.advance();
            return true;
        }
        false
    }

    fn emit_byte(&mut self, byte: u8) {
        self.compiling_chunk.write_chunk(byte, self.previous.line);
    }

    fn emit_bytes(&mut self, byte1: u8, byte2: u8) {
        self.emit_byte(byte1);
        self.emit_byte(byte2);
    }

    fn end_compiler(&mut self) {
        if !self.had_error && self.debug_print {
            self.compiling_chunk.disassemble_chunk("code");
        }
        self.emit_return();
    }

    fn expression(&mut self) {
        self.parse_precedence(Precedence::Assignment)
    }

    fn expression_statement(&mut self) {
        self.expression();
        self.consume(TokenKind::Semicolon, "Expect ';' after expression.");
        self.emit_byte(OpCode::Pop as u8);
    }

    fn declaration(&mut self) {
        if self.peek_match(TokenKind::Var) {
            self.var_declaration();
        } else {
            self.statement();
        }

        if self.panic_mode {
            self.synchronize();
        }
    }

    fn var_declaration(&mut self) {
        let global = self.parse_variable("Expect variable name.");

        if self.peek_match(TokenKind::Equal) {
            self.expression();
        } else {
            self.emit_byte(OpCode::Nil as u8);
        }

        self.consume(
            TokenKind::Semicolon,
            "Expect ';' after variable declaration",
        );

        self.define_variable(global);
    }

    fn statement(&mut self) {
        if self.peek_match(TokenKind::Print) {
            self.print_statement();
        } else {
            self.expression();
        }
    }

    fn print_statement(&mut self) {
        self.expression();
        self.consume(TokenKind::Semicolon, "Expect ';' after value.");
        self.emit_byte(OpCode::Print as u8);
    }

    fn synchronize(&mut self) {
        self.panic_mode = false;

        while self.current.kind != TokenKind::Eof {
            if self.previous.kind == TokenKind::Semicolon {
                return;
            }
            match self.current.kind {
                TokenKind::Class
                | TokenKind::Fun
                | TokenKind::Var
                | TokenKind::For
                | TokenKind::If
                | TokenKind::While
                | TokenKind::Print
                | TokenKind::Return => {
                    return;
                }
                _ => {}
            }
            self.advance();
        }
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

    fn string(&mut self) {
        let stripped = self.previous.lexeme.trim_matches('"');
        if let Some(&ptr) = self.strings.get(stripped) {
            self.emit_constant(Value::Obj(ptr));
            return;
        }
        let new_obj = Box::new(Obj {
            kind: ObjKind::String(stripped.to_string()),
            next: *self.objects,
            marked: false,
        });

        let ptr = Box::into_raw(new_obj);

        self.strings.insert(stripped.to_string(), ptr); // intern the string
        *self.objects = ptr; // update the GC linked list
        self.emit_constant(Value::Obj(ptr)); // emit the opcode
    }

    fn variable(&mut self) {
        self.named_variable(self.previous.clone())
    }

    fn named_variable(&mut self, name: Token) {
        let arg = self.identifier_constant(name);
        self.emit_bytes(OpCode::GetGlobal as u8, arg);
    }

    fn literal(&mut self) {
        match self.previous.kind {
            TokenKind::False => self.emit_byte(OpCode::False as u8),
            TokenKind::True => self.emit_byte(OpCode::True as u8),
            TokenKind::Nil => self.emit_byte(OpCode::Nil as u8),
            _ => unreachable!(),
        }
    }

    fn unary(&mut self) {
        let kind = self.previous.kind;

        self.parse_precedence(Precedence::Unary);

        match kind {
            TokenKind::Minus => self.emit_byte(OpCode::Negate as u8),
            TokenKind::Bang => self.emit_byte(OpCode::Not as u8),
            _ => unreachable!(),
        }
    }

    fn binary(&mut self) {
        let kind = self.previous.kind;
        self.parse_precedence(Precedence::from(kind) + 1);
        match kind {
            TokenKind::Plus => self.emit_byte(OpCode::Add as u8),
            TokenKind::Minus => self.emit_byte(OpCode::Subtract as u8),
            TokenKind::Star => self.emit_byte(OpCode::Multiply as u8),
            TokenKind::Slash => self.emit_byte(OpCode::Divide as u8),
            TokenKind::BangEqual => self.emit_bytes(OpCode::Equal as u8, OpCode::Not as u8),
            TokenKind::EqualEqual => self.emit_byte(OpCode::Equal as u8),
            TokenKind::Greater => self.emit_byte(OpCode::Greater as u8),
            TokenKind::GreaterEqual => self.emit_bytes(OpCode::Less as u8, OpCode::Not as u8),
            TokenKind::Less => self.emit_byte(OpCode::Less as u8),
            TokenKind::LessEqual => self.emit_bytes(OpCode::Greater as u8, OpCode::Not as u8),
            _ => unreachable!(),
        }
    }

    fn call_prefix(&mut self, kind: TokenKind) {
        match kind {
            TokenKind::LeftParen => self.grouping(),
            TokenKind::Minus | TokenKind::Bang => self.unary(),
            TokenKind::Number => self.number(),
            TokenKind::String => self.string(),
            TokenKind::Identifier => self.variable(),
            TokenKind::False | TokenKind::True | TokenKind::Nil => self.literal(),
            _ => self.error("Expect expression".into()),
        }
    }

    fn call_infix(&mut self, kind: TokenKind) {
        match kind {
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::EqualEqual
            | TokenKind::BangEqual
            | TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::Less
            | TokenKind::LessEqual => self.binary(),
            _ => {}
        }
    }

    fn parse_precedence(&mut self, precedence: Precedence) {
        self.advance();
        self.call_prefix(self.previous.kind);

        while precedence <= Precedence::from(self.current.kind) {
            self.advance();
            self.call_infix(self.previous.kind);
        }
    }

    fn parse_variable(&mut self, msg: &str) -> u8 {
        self.consume(TokenKind::Identifier, msg);
        self.identifier_constant(self.previous.clone())
    }
    
    fn define_variable(&mut self, global: u8) {
        self.emit_bytes(OpCode::DefineGlobal as u8, global);
    }

    fn identifier_constant(&mut self, token: Token) -> u8 {
        if let Some(&ptr) = self.strings.get(&token.lexeme) {
            return self.make_constant(Value::Obj(ptr));
        }
        let new_obj = Box::new(Obj {
            kind: ObjKind::String(token.lexeme.clone()),
            next: *self.objects,
            marked: false,
        });
        let ptr = Box::into_raw(new_obj);
        self.strings.insert(token.lexeme, ptr);
        *self.objects = ptr;
        self.make_constant(Value::Obj(ptr))
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
