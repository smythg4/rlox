#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    // single char tokens
    LeftBrace,
    RightBrace,
    LeftParen,
    RightParen,
    Comma,
    Dot,
    Minus,
    Slash,
    Plus,
    Semicolon,
    Star,

    // one or two char tokens
    Bang,
    BangEqual,
    Equal,
    EqualEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,

    // literals
    Identifier,
    String,
    Number,

    // keywords
    And,
    Class,
    Else,
    False,
    For,
    Fun,
    If,
    Nil,
    Or,
    Print,
    Return,
    Super,
    This,
    True,
    Var,
    While,

    //other
    Error,
    Eof,

    // sentinel for maintaining a count of possibilities
    Count,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub line: usize,
}

pub struct Lexer<'a> {
    source: &'a str,
    start: usize,   // index of starting char of current lexeme
    current: usize, // current character under examination
    line: usize,
}

impl<'a> Lexer<'a> {
    pub fn from(source: &'a str) -> Self {
        Lexer {
            source,
            start: 0,
            current: 0,
            line: 1,
        }
    }

    pub fn scan_token(&mut self) -> Token {
        loop {
            self.skip_whitespace();
            self.start = self.current;

            if self.start >= self.source.len() {
                return self.make_token(TokenKind::Eof);
            }
            let c = self.advance();

            match c {
                '(' => return self.make_token(TokenKind::LeftParen),
                ')' => return self.make_token(TokenKind::RightParen),
                '{' => return self.make_token(TokenKind::LeftBrace),
                '}' => return self.make_token(TokenKind::RightBrace),
                ';' => return self.make_token(TokenKind::Semicolon),
                ',' => return self.make_token(TokenKind::Comma),
                '.' => return self.make_token(TokenKind::Dot),
                '-' => return self.make_token(TokenKind::Minus),
                '+' => return self.make_token(TokenKind::Plus),
                '/' => {
                    if self.peek() == '/' {
                        // a comment goes until the end of the line.
                        while self.peek() != '\n' && self.current < self.source.len() {
                            self.advance();
                        }
                        continue;
                    } else {
                        return self.make_token(TokenKind::Slash);
                    }
                }
                '*' => return self.make_token(TokenKind::Star),
                '!' => {
                    return if self.expect_match('=') {
                        self.make_token(TokenKind::BangEqual)
                    } else {
                        self.make_token(TokenKind::Bang)
                    };
                }
                '=' => {
                    return if self.expect_match('=') {
                        self.make_token(TokenKind::EqualEqual)
                    } else {
                        self.make_token(TokenKind::Equal)
                    };
                }
                '<' => {
                    return if self.expect_match('=') {
                        self.make_token(TokenKind::LessEqual)
                    } else {
                        self.make_token(TokenKind::Less)
                    };
                }
                '>' => {
                    return if self.expect_match('=') {
                        self.make_token(TokenKind::GreaterEqual)
                    } else {
                        self.make_token(TokenKind::Greater)
                    };
                }
                '"' => return self.string_token(),
                '0'..='9' => return self.number_token(),
                'a'..='z' | 'A'..='Z' | '_' => return self.ident_token(),
                _ => return self.error_token("Unrecognized character"),
            }
        }
    }

    fn advance(&mut self) -> char {
        if self.current >= self.source.len() {
            return '\0';
        }
        self.current += 1;
        self.source.as_bytes()[self.current - 1] as char
    }

    fn expect_match(&mut self, expected: char) -> bool {
        if self.current >= self.source.len() || self.peek() != expected {
            return false;
        }
        self.current += 1;
        true
    }

    fn skip_whitespace(&mut self) {
        loop {
            match self.peek() {
                ' ' | '\r' | '\t' => {
                    self.advance();
                    continue;
                }
                '\n' => {
                    self.advance();
                    self.line += 1;
                    continue;
                }
                _ => {
                    return;
                }
            }
        }
    }

    pub fn peek(&self) -> char {
        if self.current >= self.source.len() {
            return '\0';
        }
        self.source.as_bytes()[self.current] as char
    }

    fn peek_next(&self) -> char {
        if self.current + 1 >= self.source.len() {
            return '\0';
        }
        self.source.as_bytes()[self.current + 1] as char
    }

    fn string_token(&mut self) -> Token {
        while self.peek() != '"' && self.current < self.source.len() {
            if self.peek() == '\n' {
                self.line += 1
            }
            self.advance();
        }
        if self.current >= self.source.len() {
            return self.error_token("Unterminated string");
        }
        self.advance(); // consume closing quote
        self.make_token(TokenKind::String)
    }

    fn number_token(&mut self) -> Token {
        while self.peek().is_ascii_digit() {
            self.advance();
        }
        if self.peek() == '.' && self.peek_next().is_ascii_digit() {
            // consume the '.'
            self.advance();
            while self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        self.make_token(TokenKind::Number)
    }

    fn ident_token(&mut self) -> Token {
        while self.peek().is_ascii_alphanumeric() || self.peek() == '_' {
            self.advance();
        }
        self.make_token(self.ident_type())
    }

    fn ident_type(&self) -> TokenKind {
        match self.source.as_bytes()[self.start] as char {
            'a' => self.check_keyword(1, 2, "nd", TokenKind::And),
            'c' => self.check_keyword(1, 4, "lass", TokenKind::Class),
            'e' => self.check_keyword(1, 3, "lse", TokenKind::Else),
            'f' => {
                if self.current - self.start > 1 {
                    match self.source.as_bytes()[self.start + 1] as char {
                        'a' => self.check_keyword(2, 3, "lse", TokenKind::False),
                        'o' => self.check_keyword(2, 1, "r", TokenKind::For),
                        'u' => self.check_keyword(2, 1, "n", TokenKind::Fun),
                        _ => TokenKind::Identifier,
                    }
                } else {
                    TokenKind::Identifier
                }
            }
            'i' => self.check_keyword(1, 1, "f", TokenKind::If),
            'n' => self.check_keyword(1, 2, "il", TokenKind::Nil),
            'o' => self.check_keyword(1, 1, "r", TokenKind::Or),
            'p' => self.check_keyword(1, 4, "rint", TokenKind::Print),
            'r' => self.check_keyword(1, 5, "eturn", TokenKind::Return),
            's' => self.check_keyword(1, 4, "uper", TokenKind::Super),
            't' => {
                if self.current - self.start > 1 {
                    match self.source.as_bytes()[self.start + 1] as char {
                        'h' => self.check_keyword(2, 2, "is", TokenKind::This),
                        'r' => self.check_keyword(2, 2, "ue", TokenKind::True),
                        _ => TokenKind::Identifier,
                    }
                } else {
                    TokenKind::Identifier
                }
            }
            'v' => self.check_keyword(1, 2, "ar", TokenKind::Var),
            'w' => self.check_keyword(1, 4, "hile", TokenKind::While),
            _ => TokenKind::Identifier,
        }
    }

    fn check_keyword(&self, start: usize, length: usize, rest: &str, kind: TokenKind) -> TokenKind {
        if self.current - self.start == start + length
            && &self.source[self.start + start..self.start + start + length] == rest
        {
            return kind;
        }
        TokenKind::Identifier
    }

    fn error_token(&self, lexeme: &str) -> Token {
        Token {
            kind: TokenKind::Error,
            lexeme: lexeme.to_string(),
            line: self.line,
        }
    }

    fn make_token(&self, kind: TokenKind) -> Token {
        Token {
            kind,
            lexeme: self.source[self.start..self.current].to_string(),
            line: self.line,
        }
    }
}
