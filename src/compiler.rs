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

struct Local(Token, Option<usize>); // name and depth -- `None` means uninitialized

pub struct Compiler<'a> {
    lexer: Lexer<'a>,
    current: Token,
    previous: Token,

    compiling_chunk: &'a mut Chunk,

    objects: &'a mut *mut Obj,
    strings: &'a mut HashMap<String, *mut Obj>,

    locals: Vec<Local>,
    local_count: usize,
    scope_depth: usize,

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

            locals: Vec::new(),
            local_count: 0,
            scope_depth: 0,

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

    fn emit_jump(&mut self, op: OpCode) -> usize {
        self.emit_byte(op as u8);
        self.emit_bytes(0xff, 0xff);
        // Nystrom had this twice: `self.emit_byte(0xff);`, I'm trying the emit_bytes instead.
        // return the index of where these dummy bytes were stored for later
        self.compiling_chunk.codes.len() - 2
    }

    fn patch_jump(&mut self, offset: usize) {
        // -2 to adjust for the bytecode for the jump offset itself
        let jump = self.compiling_chunk.codes.len() - offset - 2;
        if jump > u16::MAX as usize {
            self.error("Too much code to jump over.".to_string());
        }
        self.compiling_chunk.codes[offset] = (jump >> 8) as u8;
        self.compiling_chunk.codes[offset+1] = jump as u8;
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

    fn if_statement(&mut self) {
        self.consume(TokenKind::LeftParen, "Expect a '(' after 'if'.");
        self.expression();
        self.consume(TokenKind::RightParen, "Expect a ')' after condition.");

        let then_jump = self.emit_jump(OpCode::JumpIfFalse);
        self.emit_byte(OpCode::Pop as u8); // if the condition was truthy, we pop it off the stack right before the then branch
        self.statement();

        let else_jump = self.emit_jump(OpCode::Jump);

        self.patch_jump(then_jump);
        self.emit_byte(OpCode::Pop as u8); // if the condition was falsey, we pop it off the stack at beginning of the else branch

        if self.peek_match(TokenKind::Else) {
            self.statement();
        }
        self.patch_jump(else_jump);
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
        // TODO: Eliminate `peek_match` unless you find a good use case now that we're shifting
        // to a match paradigm.
        match self.current.kind {
            TokenKind::Print => {
                self.advance();
                self.print_statement();
            }
            TokenKind::If => {
                self.advance();
                self.if_statement();
            }
            TokenKind::LeftBrace => {
                self.advance();
                self.begin_scope();
                self.block();
                self.end_scope();
            }
            _ => {
                self.expression_statement();
            }
        }
    }

    fn block(&mut self) {
        while self.current.kind != TokenKind::RightBrace && self.current.kind != TokenKind::Eof {
            // '\0' is EOF
            // Nystrom uses `check(TOKEN_RIGHT_BRACE) && check(TOKEN_EOF)`
            self.declaration();
        }
        self.consume(TokenKind::RightBrace, "Expect a '}' after a block.");
    }

    fn begin_scope(&mut self) {
        self.scope_depth += 1;
    }

    fn end_scope(&mut self) {
        self.scope_depth -= 1;

        // put locals to rest when a scope exists
        while self.local_count > 0
            && let Some(depth) = self.locals[self.local_count - 1].1
            && depth > self.scope_depth
        {
            self.emit_byte(OpCode::Pop as u8);
            self.local_count -= 1;
        }
        self.locals.truncate(self.local_count);
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

    fn variable(&mut self, can_assign: bool) {
        self.named_variable(self.previous.clone(), can_assign)
    }

    fn named_variable(&mut self, name: Token, can_assign: bool) {
        let get_op;
        let set_op;
        let arg;
        let local = self.resolve_local(&name); // needs to return a sentinel in some cases...
        if let Some(larg) = local {
            get_op = OpCode::GetLocal as u8;
            set_op = OpCode::SetLocal as u8;
            arg = larg;
        } else {
            get_op = OpCode::GetGlobal as u8;
            set_op = OpCode::SetGlobal as u8;
            arg = self.identifier_constant(name);
        }

        if can_assign && self.peek_match(TokenKind::Equal) {
            self.expression();
            self.emit_bytes(set_op, arg);
        } else {
            self.emit_bytes(get_op, arg);
        }
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

    fn call_prefix(&mut self, kind: TokenKind, can_assign: bool) {
        match kind {
            TokenKind::LeftParen => self.grouping(),
            TokenKind::Minus | TokenKind::Bang => self.unary(),
            TokenKind::Number => self.number(),
            TokenKind::String => self.string(),
            TokenKind::Identifier => self.variable(can_assign),
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
        let can_assign = precedence <= Precedence::Assignment;
        self.advance();
        self.call_prefix(self.previous.kind, can_assign);

        while precedence <= Precedence::from(self.current.kind) {
            self.advance();
            self.call_infix(self.previous.kind);
        }

        if can_assign && self.current.kind == TokenKind::Equal {
            self.error("Invalid assignment target.".into());
        }
    }

    fn parse_variable(&mut self, msg: &str) -> u8 {
        self.consume(TokenKind::Identifier, msg);

        self.declare_variable(); // `declareVariable()` in Nystrom
        if self.scope_depth > 0 {
            return 0;
        }

        self.identifier_constant(self.previous.clone())
    }

    fn mark_initialized(&mut self) {
        self.locals[self.local_count - 1].1 = Some(self.scope_depth);
    }

    fn define_variable(&mut self, global: u8) {
        // determine if it's local or global (global = 0)
        if self.scope_depth > 0 {
            self.mark_initialized();
            return;
        }
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

    fn declare_variable(&mut self) {
        if self.scope_depth == 0 {
            return;
        }

        let name = self.previous.clone();
        // check to make sure we're not redeclaring in the same scope
        for i in (0..self.local_count).rev() {
            let local = &self.locals[i];
            if let Some(depth) = local.1
                && depth < self.scope_depth
            {
                break;
            }

            if Self::identifiers_equal(&name, &local.0) {
                self.error("Already a variable with this name in this scope.".into());
            }
        }
        self.add_local(name);
    }

    fn identifiers_equal(a: &Token, b: &Token) -> bool {
        a.lexeme == b.lexeme
    }

    fn resolve_local(&mut self, name: &Token) -> Option<u8> {
        for i in (0..self.local_count).rev() {
            let local = &self.locals[i];
            if Self::identifiers_equal(name, &local.0) {
                if local.1.is_none() {
                    self.error("Can't read local variable in its own initializer.".into());
                }
                return Some(i as u8);
            }
        }
        None
    }

    fn add_local(&mut self, local_token: Token) {
        if self.local_count >= u8::MAX as usize {
            self.error("Too many local variables in function".into());
            return;
        }
        self.locals.push(Local(local_token, None));
        self.local_count += 1;
    }

    fn emit_constant(&mut self, value: Value) {
        let const_i = self.make_constant(value);
        self.emit_bytes(OpCode::Constant as u8, const_i);
    }

    fn make_constant(&mut self, value: Value) -> u8 {
        let const_i = self.compiling_chunk.add_constant(value);
        if self.compiling_chunk.constants.len() > u8::MAX as usize {
            self.error("too many constants for one chunk".into());
            return 0;
        }
        const_i
    }
}
