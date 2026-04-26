use crate::vecmap::VecMap;

use crate::chunk::{Chunk, OpCode};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::value::{Obj, ObjKind, Value};

use anyhow::{Result, bail};

#[derive(Default)]
struct ClassContext {
    has_super_class: bool,
}

struct CompilerContext {
    function: *mut Obj,          // ObjKind::Function with its own chunk
    function_kind: FunctionKind, // top-level or user-defined function
    locals: Vec<Local>,
    scope_depth: usize,
    upvalues: Vec<UpValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FunctionKind {
    Script, // top-level execution
    Function,
    Method,
    Initializer,
}

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
    //Primary, // I wasn't using this...
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
            TokenKind::And => Precedence::And,
            TokenKind::Or => Precedence::Or,
            TokenKind::LeftParen | TokenKind::Dot => Precedence::Call,
            _ => Precedence::None,
        }
    }
}

struct Local {
    name: Token,          // name
    depth: Option<usize>, // `None` means uninitialized
    is_captured: bool,
}

#[derive(Debug, Clone, Copy)]
struct UpValue {
    index: usize,
    is_local: bool,
}

pub struct Compiler<'a> {
    lexer: Lexer<'a>,
    current: Token,
    previous: Token,

    contexts: Vec<CompilerContext>,
    class_contexts: Vec<ClassContext>,

    objects: &'a mut *mut Obj,
    strings: &'a mut VecMap<String, *mut Obj>,

    had_error: bool,
    panic_mode: bool,
    debug_print: bool,
}

// =============================================================================
// Context Management
// =============================================================================

impl<'a> Compiler<'a> {
    pub fn with_debug(mut self) -> Self {
        self.debug_print = true;
        self
    }

    pub fn new(
        lexer: Lexer<'a>,
        objects: &'a mut *mut Obj,
        strings: &'a mut VecMap<String, *mut Obj>,
    ) -> Self {
        let mut c = Compiler {
            lexer,
            current: Token {
                kind: TokenKind::Nil,
                lexeme: String::new(),
                line: 0,
            },
            previous: Token {
                kind: TokenKind::Nil,
                lexeme: String::new(),
                line: 0,
            },
            contexts: Vec::new(),
            class_contexts: Vec::new(),
            objects,
            strings,

            had_error: false,
            panic_mode: false,
            debug_print: false,
        };
        c.push_context(FunctionKind::Script);
        c
    }

    pub fn compile(&mut self) -> Result<*mut Obj> {
        self.advance();

        while !self.peek_match(TokenKind::Eof) {
            self.declaration();
        }

        self.consume(TokenKind::Eof, "Expect end of expression");
        if self.had_error {
            bail!("compiler error");
        }
        Ok(self.end_compiler())
    }

    /// Initializes a new compiler context for a function or the top-level script.
    ///
    /// Each context owns its `ObjFunction` (with a fresh `Chunk`) and an independent
    /// locals stack. Slot 0 is reserved via a dummy local — the VM places the function
    /// object there at call time, so user-declared locals start at slot 1.
    ///
    /// Mirrors Nystrom's `initCompiler`. Paired with `end_compiler`, which pops the
    /// context and returns the finished `ObjFunction`.
    fn push_context(&mut self, kind: FunctionKind) {
        let name = match kind {
            FunctionKind::Script => String::new(),
            _ => self.previous.lexeme.clone(),
        };
        let func_obj = Box::into_raw(Box::new(Obj {
            kind: ObjKind::Function {
                arity: 0,
                name,
                chunk: Chunk::default(),
                upvalue_count: 0,
            },
            next: *self.objects,
            marked: false,
        }));
        *self.objects = func_obj;
        let dummy_name = match kind {
            FunctionKind::Method | FunctionKind::Initializer => "this".to_string(),
            _ => String::new(),
        };
        let dummy = Local {
            name: Token {
                kind: TokenKind::Nil,
                lexeme: dummy_name,
                line: 0,
            },
            depth: Some(0),
            is_captured: false,
        };
        self.contexts.push(CompilerContext {
            function: func_obj,
            function_kind: kind,
            locals: vec![dummy],
            scope_depth: 0,
            upvalues: Vec::new(),
        });
    }

    fn end_compiler(&mut self) -> *mut Obj {
        self.emit_return();
        let context = self.contexts.pop().unwrap();
        // SAFETY: context.function was allocated via Box::into_raw in push_context
        // and is still live — end_compiler is the only site that consumes the context.
        let object = unsafe { &(*context.function).kind };
        let ObjKind::Function { chunk, .. } = object else {
            panic!("Attempt to return a non-function object from compiler")
        };
        if !self.had_error && self.debug_print {
            let name = match object {
                ObjKind::Function { name, .. } if !name.is_empty() => name.as_ref(),
                _ => "<script>",
            };
            chunk.disassemble_chunk(name);
        }
        context.function
    }

    fn current_chunk(&self) -> &Chunk {
        let context = self.contexts.last().unwrap();
        // SAFETY: context.function is a live Box::into_raw allocation for the duration
        // of this CompilerContext's presence in self.contexts.
        let object = unsafe { &(*context.function).kind };
        let ObjKind::Function { chunk, .. } = object else {
            unreachable!()
        };
        chunk
    }

    fn current_chunk_mut(&mut self) -> &mut Chunk {
        let context = self.contexts.last_mut().unwrap();
        // SAFETY: same as current_chunk; &mut self ensures exclusive access.
        let object = unsafe { &mut (*context.function).kind };
        let ObjKind::Function { chunk, .. } = object else {
            unreachable!()
        };
        chunk
    }

    // fn current_function(&self) -> &ObjKind {
    //     let context = self.contexts.last().unwrap();
    //     // SAFETY: same as current_chunk.
    //     unsafe { &(*context.function).kind }
    // }

    fn current_function_mut(&mut self) -> &mut ObjKind {
        let context = self.contexts.last().unwrap();
        // SAFETY: same as current_chunk; &mut self ensures exclusive access.
        unsafe { &mut (*context.function).kind }
    }

    fn current_locals(&self) -> &[Local] {
        &self.contexts.last().unwrap().locals
    }

    fn current_locals_mut(&mut self) -> &mut Vec<Local> {
        &mut self.contexts.last_mut().unwrap().locals
    }

    fn current_local_count(&self) -> usize {
        self.contexts.last().unwrap().locals.len()
    }

    fn current_scope_depth(&self) -> usize {
        self.contexts.last().unwrap().scope_depth
    }

    fn current_scope_depth_mut(&mut self) -> &mut usize {
        &mut self.contexts.last_mut().unwrap().scope_depth
    }
}

// =============================================================================
// Emitter
// =============================================================================

impl<'a> Compiler<'a> {
    fn emit_byte(&mut self, byte: u8) {
        let prev_line = self.previous.line;
        self.current_chunk_mut().write_chunk(byte, prev_line);
    }

    fn emit_bytes(&mut self, byte1: u8, byte2: u8) {
        self.emit_byte(byte1);
        self.emit_byte(byte2);
    }

    fn emit_constant(&mut self, value: Value) {
        let const_i = self.make_constant(value);
        self.emit_bytes(OpCode::Constant as u8, const_i);
    }

    fn emit_jump(&mut self, op: OpCode) -> usize {
        self.emit_byte(op as u8);
        self.emit_bytes(0xff, 0xff);
        self.current_chunk().codes.len() - 2
    }

    fn patch_jump(&mut self, offset: usize) {
        // -2 to adjust for the bytecode for the jump offset itself
        let jump = self.current_chunk().codes.len() - offset - 2;
        if jump > u16::MAX as usize {
            self.error("Too much code to jump over.");
        }
        self.current_chunk_mut().codes[offset] = (jump >> 8) as u8;
        self.current_chunk_mut().codes[offset + 1] = jump as u8;
    }

    fn emit_loop(&mut self, loop_start: usize) {
        self.emit_byte(OpCode::Loop as u8);

        let offset = self.current_chunk().codes.len() - loop_start + 2;
        if offset > u16::MAX as usize {
            self.error("Loop body too large");
        }

        self.emit_bytes((offset >> 8) as u8, offset as u8);
    }

    fn emit_return(&mut self) {
        match self.contexts.last().unwrap().function_kind {
            FunctionKind::Initializer => self.emit_bytes(OpCode::GetLocal as u8, 0),
            _ => self.emit_byte(OpCode::Nil as u8),
        }
        self.emit_byte(OpCode::Return as u8);
    }

    fn make_constant(&mut self, value: Value) -> u8 {
        let const_i = self.current_chunk_mut().add_constant(value);
        if self.current_chunk().constants.len() > u8::MAX as usize {
            self.error("too many constants for one chunk");
            return 0;
        }
        const_i
    }
}

// =============================================================================
// Scope & Locals
// =============================================================================

impl<'a> Compiler<'a> {
    fn begin_scope(&mut self) {
        *self.current_scope_depth_mut() += 1;
    }

    fn end_scope(&mut self) {
        // couldn't use the helpers here due to borrow conflicts
        self.contexts.last_mut().unwrap().scope_depth -= 1;

        loop {
            let ctx = self.contexts.last_mut().unwrap();
            let should_pop = !ctx.locals.is_empty()
                && ctx.locals[ctx.locals.len() - 1]
                    .depth
                    .is_some_and(|d| d > ctx.scope_depth);
            if !should_pop {
                break;
            }
            let is_captured = ctx.locals.last().unwrap().is_captured;
            ctx.locals.pop();
            if is_captured {
                self.emit_byte(OpCode::CloseUpvalue as u8);
            } else {
                self.emit_byte(OpCode::Pop as u8);
            }
        }
    }

    fn declare_variable(&mut self) {
        if self.current_scope_depth() == 0 {
            return;
        }

        let name = self.previous.clone();
        for i in (0..self.current_local_count()).rev() {
            let local = &self.current_locals()[i];
            if let Some(depth) = local.depth
                && depth < self.current_scope_depth()
            {
                break;
            }
            if Self::identifiers_equal(&name, &local.name) {
                self.error("Already a variable with this name in this scope.");
            }
        }
        self.add_local(name);
    }

    fn define_variable(&mut self, global: u8) {
        if self.current_scope_depth() > 0 {
            self.mark_initialized();
            return;
        }
        self.emit_bytes(OpCode::DefineGlobal as u8, global);
    }

    fn mark_initialized(&mut self) {
        let count = self.current_local_count();
        let depth = self.current_scope_depth();
        if depth == 0 {
            return;
        }
        self.current_locals_mut()[count - 1].depth = Some(depth);
    }

    fn add_local(&mut self, local_token: Token) {
        if self.current_local_count() >= u8::MAX as usize {
            self.error("Too many local variables in function");
            return;
        }
        self.current_locals_mut().push(Local {
            name: local_token,
            depth: None,
            is_captured: false,
        });
    }

    fn resolve_local_in(&mut self, ctx_idx: usize, name: &Token) -> Option<u8> {
        let locals = &self.contexts[ctx_idx].locals;
        for i in (0..locals.len()).rev() {
            if Self::identifiers_equal(name, &locals[i].name) {
                return locals[i].depth.map(|_| i as u8);
            }
        }
        None
    }

    fn resolve_local(&mut self, name: &Token) -> Option<u8> {
        let idx = self.contexts.len() - 1;
        self.resolve_local_in(idx, name)
    }

    fn resolve_upvalue_in(&mut self, ctx_idx: usize, name: &Token) -> Option<u8> {
        if ctx_idx == 0 {
            return None;
        }
        if let Some(local) = self.resolve_local_in(ctx_idx - 1, name) {
            self.contexts[ctx_idx - 1].locals[local as usize].is_captured = true;
            return Some(self.add_upvalue_in(ctx_idx, local as usize, true));
        }
        if let Some(upvalue) = self.resolve_upvalue_in(ctx_idx - 1, name) {
            return Some(self.add_upvalue_in(ctx_idx, upvalue as usize, false));
        }
        None
    }

    fn resolve_upvalue(&mut self, name: &Token) -> Option<u8> {
        let idx = self.contexts.len() - 1;
        self.resolve_upvalue_in(idx, name)
    }

    // fn add_upvalue(&mut self, index: usize, is_local: bool) -> u8 {
    //     let idx = self.contexts.len() - 1;
    //     self.add_upvalue_in(idx, index, is_local)
    // }

    fn add_upvalue_in(&mut self, ctx_id: usize, index: usize, is_local: bool) -> u8 {
        let ctx = &mut self.contexts[ctx_id];
        // SAFETY: same as current_func_mut
        let ObjKind::Function { upvalue_count, .. } = (unsafe { &mut (*ctx.function).kind }) else {
            unreachable!()
        };
        for i in 0..*upvalue_count {
            let uv = ctx.upvalues[i];
            if uv.index == index && uv.is_local == is_local {
                return i as u8;
            }
        }
        if *upvalue_count >= u8::MAX as usize {
            self.error("too many closure variables in function.");
            return 0;
        }
        ctx.upvalues.push(UpValue { index, is_local });
        *upvalue_count += 1;
        (*upvalue_count - 1) as u8
    }

    fn parse_variable(&mut self, msg: &str) -> u8 {
        self.consume(TokenKind::Identifier, msg);
        self.declare_variable();
        if self.current_scope_depth() > 0 {
            return 0;
        }
        self.identifier_constant(self.previous.clone())
    }

    fn identifier_constant(&mut self, token: Token) -> u8 {
        let ptr = self.intern_string(token.lexeme);
        self.make_constant(Value::from_obj(ptr))
    }

    fn intern_string(&mut self, s: String) -> *mut Obj {
        if let Some(&ptr) = self.strings.get(&s) {
            return ptr;
        }
        let new_obj = Box::new(Obj {
            kind: ObjKind::String(s.clone()),
            next: *self.objects,
            marked: false,
        });
        let ptr = Box::into_raw(new_obj);
        self.strings.insert(s, ptr);
        *self.objects = ptr;
        ptr
    }

    fn identifiers_equal(a: &Token, b: &Token) -> bool {
        a.lexeme == b.lexeme
    }
}

// =============================================================================
// Parser
// =============================================================================

impl<'a> Compiler<'a> {
    fn advance(&mut self) {
        std::mem::swap(&mut self.previous, &mut self.current);
        loop {
            self.current = self.lexer.scan_token();
            if self.current.kind == TokenKind::Error {
                let msg = self.current.lexeme.clone();
                self.error_at_current(&msg);
            } else {
                break;
            }
        }
    }

    fn consume(&mut self, kind: TokenKind, msg: &str) {
        if self.current.kind == kind {
            self.advance();
            return;
        }
        self.error_at_current(msg);
    }

    fn peek_match(&mut self, kind: TokenKind) -> bool {
        if self.current.kind == kind {
            self.advance();
            return true;
        }
        false
    }

    fn error_at_current(&mut self, msg: &str) {
        let token = self.current.clone();
        self.error_at(&token, msg)
    }

    fn error(&mut self, msg: &str) {
        let token = self.previous.clone();
        self.error_at(&token, msg)
    }

    fn error_at(&mut self, token: &Token, msg: &str) {
        if self.panic_mode {
            return;
        }
        self.panic_mode = true;
        eprint!("[line {}] Error", token.line);

        if token.kind == TokenKind::Eof {
            eprint!(" at end");
        } else if token.kind != TokenKind::Error {
            eprint!(" at '{}'", token.lexeme);
        }

        eprintln!(": {msg}");
        self.had_error = true;
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
                | TokenKind::Return => return,
                _ => {}
            }
            self.advance();
        }
    }

    fn parse_precedence(&mut self, precedence: Precedence) {
        let can_assign = precedence <= Precedence::Assignment;
        self.advance();
        self.call_prefix(self.previous.kind, can_assign);

        while precedence <= Precedence::from(self.current.kind) {
            self.advance();
            self.call_infix(self.previous.kind, can_assign);
        }

        if can_assign && self.current.kind == TokenKind::Equal {
            self.error("Invalid assignment target.");
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
            TokenKind::This => {
                if self.class_contexts.is_empty() {
                    self.error("Can't use `this` outside of a class.");
                    return;
                }
                self.this_expr()
            }
            TokenKind::Super => self.super_expr(),
            _ => self.error("Expect expression"),
        }
    }

    fn call_infix(&mut self, kind: TokenKind, can_assign: bool) {
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
            TokenKind::And => self.and_expr(),
            TokenKind::Or => self.or_expr(),
            TokenKind::LeftParen => self.call(),
            TokenKind::Dot => self.dot_expr(can_assign),
            _ => {}
        }
    }

    // --- declarations ---

    fn declaration(&mut self) {
        if self.peek_match(TokenKind::Var) {
            self.var_declaration();
        } else if self.peek_match(TokenKind::Fun) {
            self.fun_declaration();
        } else if self.peek_match(TokenKind::Class) {
            self.class_declaration();
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

    fn fun_declaration(&mut self) {
        let global = self.parse_variable("Expect function name.");
        self.mark_initialized();
        self.function(FunctionKind::Function);
        self.define_variable(global);
    }

    fn function(&mut self, kind: FunctionKind) {
        self.push_context(kind);
        self.begin_scope();

        self.consume(TokenKind::LeftParen, "Expect '(' after function name.");

        if self.current.kind != TokenKind::RightParen {
            loop {
                let ObjKind::Function { arity, .. } = self.current_function_mut() else {
                    unreachable!()
                };
                *arity += 1;
                if *arity > u8::MAX as usize {
                    self.error_at_current("can't have more than 255 parameters.");
                }
                let constant = self.parse_variable("expect parameter name.");
                self.define_variable(constant);
                if !self.peek_match(TokenKind::Comma) {
                    break;
                }
            }
        }

        self.consume(TokenKind::RightParen, "Expect ')' after parameters.");
        self.consume(TokenKind::LeftBrace, "Expect '{' before function body.");
        self.block();

        let upvalues = std::mem::take(&mut self.contexts.last_mut().unwrap().upvalues);
        let function = self.end_compiler();
        let constant = self.make_constant(Value::from_obj(function));
        self.emit_bytes(OpCode::Closure as u8, constant);

        let ObjKind::Function { upvalue_count, .. } = unsafe { &*function }.kind else {
            unreachable!()
        };
        for uv in upvalues.iter().take(upvalue_count) {
            self.emit_byte(uv.is_local as u8);
            self.emit_byte(uv.index as u8);
        }
    }

    fn class_declaration(&mut self) {
        self.consume(TokenKind::Identifier, "expect a class name.");
        let class_name = self.previous.clone();
        let name_constant = self.identifier_constant(self.previous.clone());
        self.declare_variable();

        self.emit_bytes(OpCode::Class as u8, name_constant);
        self.define_variable(name_constant);

        self.class_contexts.push(ClassContext::default());
        // check for inheritance - do this before setting methods up
        if self.peek_match(TokenKind::Less) {
            self.consume(TokenKind::Identifier, "expect superclass name");
            self.variable(false);

            // make sure it's not self-inheritance
            if Self::identifiers_equal(&class_name, &self.previous) {
                self.error("a class can't inherit from itself.");
            }

            self.begin_scope();
            self.add_local(synthetic_token("super"));
            self.define_variable(0);

            self.named_variable(class_name.clone(), false);
            self.emit_byte(OpCode::Inherit as u8);
            // mark this as having a super class
            self.class_contexts.last_mut().unwrap().has_super_class = true;
        }

        self.named_variable(class_name, false);
        self.consume(TokenKind::LeftBrace, "Expect '{' before class body.");

        while self.current.kind != TokenKind::RightBrace && self.current.kind != TokenKind::Eof {
            self.method();
        }

        let had_super = self
            .class_contexts
            .last()
            .map(|cc| cc.has_super_class)
            .unwrap_or(false);
        let _ = self.class_contexts.pop();
        self.consume(TokenKind::RightBrace, "Expect '}' after class body.");
        self.emit_byte(OpCode::Pop as u8); // class gone first
        if had_super {
            self.end_scope(); // now super is on top, CloseUpvalue works
        }
    }

    fn method(&mut self) {
        self.consume(TokenKind::Identifier, "expect method name");
        let constant = self.identifier_constant(self.previous.clone());

        let func_kind = match self.previous.lexeme.as_str() {
            "init" => FunctionKind::Initializer,
            _ => FunctionKind::Method,
        };
        self.function(func_kind);

        self.emit_bytes(OpCode::Method as u8, constant);
    }

    // --- statements ---

    fn statement(&mut self) {
        match self.current.kind {
            TokenKind::Print => {
                self.advance();
                self.print_statement();
            }
            TokenKind::If => {
                self.advance();
                self.if_statement();
            }
            TokenKind::While => {
                self.advance();
                self.while_statement();
            }
            TokenKind::For => {
                self.advance();
                self.for_statement();
            }
            TokenKind::Return => {
                self.advance();
                self.return_statement();
            }
            TokenKind::LeftBrace => {
                self.advance();
                self.begin_scope();
                self.block();
                self.end_scope();
            }
            _ => self.expression_statement(),
        }
    }

    fn block(&mut self) {
        while self.current.kind != TokenKind::RightBrace && self.current.kind != TokenKind::Eof {
            self.declaration();
        }
        self.consume(TokenKind::RightBrace, "Expect '}' after block.");
    }

    fn print_statement(&mut self) {
        self.expression();
        self.consume(TokenKind::Semicolon, "Expect ';' after value.");
        self.emit_byte(OpCode::Print as u8);
    }

    fn return_statement(&mut self) {
        if self.contexts.last().unwrap().function_kind == FunctionKind::Script {
            self.error("can't return from a top-level script.");
        }
        if self.peek_match(TokenKind::Semicolon) {
            self.emit_return();
        } else {
            if self.contexts.last().unwrap().function_kind == FunctionKind::Initializer {
                self.error("can't return a value from an initializer.");
            }
            self.expression();
            self.consume(TokenKind::Semicolon, "Expect ';' after return value.");
            self.emit_byte(OpCode::Return as u8);
        }
    }

    fn if_statement(&mut self) {
        self.consume(TokenKind::LeftParen, "Expect '(' after 'if'.");
        self.expression();
        self.consume(TokenKind::RightParen, "Expect ')' after condition.");

        let then_jump = self.emit_jump(OpCode::JumpIfFalse);
        self.emit_byte(OpCode::Pop as u8);
        self.statement();

        let else_jump = self.emit_jump(OpCode::Jump);
        self.patch_jump(then_jump);
        self.emit_byte(OpCode::Pop as u8);

        if self.peek_match(TokenKind::Else) {
            self.statement();
        }
        self.patch_jump(else_jump);
    }

    fn while_statement(&mut self) {
        let loop_start = self.current_chunk().codes.len();
        self.consume(TokenKind::LeftParen, "Expect '(' after 'while'.");
        self.expression();
        self.consume(TokenKind::RightParen, "Expect ')' after condition.");

        let exit_jump = self.emit_jump(OpCode::JumpIfFalse);
        self.emit_byte(OpCode::Pop as u8);
        self.statement();
        self.emit_loop(loop_start);

        self.patch_jump(exit_jump);
        self.emit_byte(OpCode::Pop as u8);
    }

    fn for_statement(&mut self) {
        self.begin_scope();
        self.consume(TokenKind::LeftParen, "Expect '(' after 'for'.");

        if self.peek_match(TokenKind::Semicolon) {
            // no initializer
        } else if self.peek_match(TokenKind::Var) {
            self.var_declaration();
        } else {
            self.expression_statement();
        }

        let mut loop_start = self.current_chunk().codes.len();
        let mut exit_jump = None;

        if !self.peek_match(TokenKind::Semicolon) {
            self.expression();
            self.consume(TokenKind::Semicolon, "Expect ';' after loop condition.");
            exit_jump = Some(self.emit_jump(OpCode::JumpIfFalse));
            self.emit_byte(OpCode::Pop as u8);
        }

        if !self.peek_match(TokenKind::RightParen) {
            let body_jump = self.emit_jump(OpCode::Jump);
            let increment_start = self.current_chunk().codes.len();
            self.expression();
            self.emit_byte(OpCode::Pop as u8);
            self.consume(TokenKind::RightParen, "Expect ')' after for clauses.");
            self.emit_loop(loop_start);
            loop_start = increment_start;
            self.patch_jump(body_jump);
        }

        self.statement();
        self.emit_loop(loop_start);

        if let Some(pos) = exit_jump {
            self.patch_jump(pos);
            self.emit_byte(OpCode::Pop as u8);
        }

        self.end_scope();
    }

    fn expression_statement(&mut self) {
        self.expression();
        self.consume(TokenKind::Semicolon, "Expect ';' after expression.");
        self.emit_byte(OpCode::Pop as u8);
    }

    // --- expressions ---

    fn expression(&mut self) {
        self.parse_precedence(Precedence::Assignment)
    }

    fn grouping(&mut self) {
        self.expression();
        self.consume(TokenKind::RightParen, "Expect ')' after expression.");
    }

    fn number(&mut self) {
        if let Ok(value) = self.previous.lexeme.parse::<f64>() {
            self.emit_constant(Value::from_number(value));
        } else {
            self.error(&format!(
                "invalid number literal '{}'",
                self.previous.lexeme
            ));
        }
    }

    fn string(&mut self) {
        let stripped = self.previous.lexeme.trim_matches('"').to_string();
        let ptr = self.intern_string(stripped);
        self.emit_constant(Value::from_obj(ptr));
    }

    fn variable(&mut self, can_assign: bool) {
        self.named_variable(self.previous.clone(), can_assign)
    }

    fn this_expr(&mut self) {
        self.variable(false);
    }

    fn super_expr(&mut self) {
        if self.class_contexts.is_empty() {
            self.error("Can't use 'super' outside of a class.");
        } else if let Some(class_context) = self.class_contexts.last()
            && !class_context.has_super_class
        {
            self.error("Can't use 'super' in a class with no superclass.");
        }

        self.consume(TokenKind::Dot, "Expect a '.' after 'super'.");
        self.consume(TokenKind::Identifier, "Expect a superclass method name");
        let name = self.identifier_constant(self.previous.clone());

        // load the instance on the vm stack
        self.named_variable(synthetic_token("this"), false);
        // check if this is an immediate method call
        if self.peek_match(TokenKind::LeftParen) {
            let arg_count = self.argument_list();
            self.named_variable(synthetic_token("super"), false);
            // this will combine behavior of GetSuper and Call to save an allocation
            self.emit_bytes(OpCode::SuperInvoke as u8, name);
            self.emit_byte(arg_count);
        } else {
            // load the super class where the method is resolved on the stack
            self.named_variable(synthetic_token("super"), false);
            // encode the name of the method to access as an operand
            self.emit_bytes(OpCode::GetSuper as u8, name);
        }
    }

    fn named_variable(&mut self, name: Token, can_assign: bool) {
        let (get_op, set_op, arg) = if let Some(slot) = self.resolve_local(&name) {
            (OpCode::GetLocal as u8, OpCode::SetLocal as u8, slot)
        } else if let Some(upvalue) = self.resolve_upvalue(&name) {
            (OpCode::GetUpValue as u8, OpCode::SetUpValue as u8, upvalue)
        } else {
            let arg = self.identifier_constant(name);
            (OpCode::GetGlobal as u8, OpCode::SetGlobal as u8, arg)
        };

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

    fn and_expr(&mut self) {
        let end_jump = self.emit_jump(OpCode::JumpIfFalse);
        self.emit_byte(OpCode::Pop as u8);
        self.parse_precedence(Precedence::And);
        self.patch_jump(end_jump);
    }

    fn or_expr(&mut self) {
        let else_jump = self.emit_jump(OpCode::JumpIfFalse);
        let end_jump = self.emit_jump(OpCode::Jump);
        self.patch_jump(else_jump);
        self.emit_byte(OpCode::Pop as u8);
        self.parse_precedence(Precedence::Or);
        self.patch_jump(end_jump);
    }

    fn dot_expr(&mut self, can_assign: bool) {
        self.consume(TokenKind::Identifier, "expect property name after '.'");
        let name = self.identifier_constant(self.previous.clone());

        if can_assign && self.peek_match(TokenKind::Equal) {
            self.expression();
            self.emit_bytes(OpCode::SetProperty as u8, name);
        } else if self.peek_match(TokenKind::LeftParen) {
            let arg_count = self.argument_list();
            self.emit_bytes(OpCode::Invoke as u8, name);
            self.emit_byte(arg_count);
        } else {
            self.emit_bytes(OpCode::GetProperty as u8, name);
        }
    }

    fn call(&mut self) {
        let arg_count = self.argument_list();
        self.emit_bytes(OpCode::Call as u8, arg_count);
    }

    fn argument_list(&mut self) -> u8 {
        let mut count = 0u8;
        if self.current.kind != TokenKind::RightParen {
            loop {
                self.expression();
                if count == u8::MAX {
                    self.error("Can't have more than 255 arguments.");
                }
                count += 1;
                if !self.peek_match(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume(TokenKind::RightParen, "Expect ')' after arguments.");
        count
    }
}

fn synthetic_token(name: &str) -> Token {
    Token {
        kind: TokenKind::Nil,
        lexeme: name.to_string(),
        line: 0,
    }
}
