use crate::error::{Result, XPathError};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // 字面量
    Number(f64),
    String(String),
    // 标识符
    Name(String),
    // 操作符
    Slash, DoubleSlash,
    LBracket, RBracket,
    LParen, RParen,
    At, Comma, Pipe,
    Dot, DoubleDot,
    DoubleColon,
    Star,
    Plus, Minus, Eq, NotEq, Lt, Le, Gt, Ge,
    // 关键字操作符
    And, Or, Mod, Div,
    // 变量 $name
    VarRef(String),
    // 函数前缀和命名空间分隔
    Colon,
    // 文本节点测试关键字会作为 Name
    Eof,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() { i += 1; continue; }

        match c {
            '/' => {
                if i + 1 < bytes.len() && bytes[i+1] == b'/' {
                    tokens.push(Token::DoubleSlash); i += 2;
                } else { tokens.push(Token::Slash); i += 1; }
            }
            '[' => { tokens.push(Token::LBracket); i += 1; }
            ']' => { tokens.push(Token::RBracket); i += 1; }
            '(' => { tokens.push(Token::LParen); i += 1; }
            ')' => { tokens.push(Token::RParen); i += 1; }
            '@' => { tokens.push(Token::At); i += 1; }
            ',' => { tokens.push(Token::Comma); i += 1; }
            '|' => { tokens.push(Token::Pipe); i += 1; }
            '+' => { tokens.push(Token::Plus); i += 1; }
            '-' => { tokens.push(Token::Minus); i += 1; }
            '=' => { tokens.push(Token::Eq); i += 1; }
            '*' => { tokens.push(Token::Star); i += 1; }
            ':' => {
                if i + 1 < bytes.len() && bytes[i+1] == b':' {
                    tokens.push(Token::DoubleColon); i += 2;
                } else { tokens.push(Token::Colon); i += 1; }
            }
            '.' => {
                if i + 1 < bytes.len() && bytes[i+1] == b'.' {
                    tokens.push(Token::DoubleDot); i += 2;
                } else if i + 1 < bytes.len() && (bytes[i+1] as char).is_ascii_digit() {
                    // .5 这种数字
                    let start = i;
                    i += 1;
                    while i < bytes.len() && (bytes[i] as char).is_ascii_digit() { i += 1; }
                    let s: f64 = input[start..i].parse().map_err(|_| XPathError::LexError {
                        pos: start, msg: "bad number".into() })?;
                    tokens.push(Token::Number(s));
                } else { tokens.push(Token::Dot); i += 1; }
            }
            '!' => {
                if i + 1 < bytes.len() && bytes[i+1] == b'=' {
                    tokens.push(Token::NotEq); i += 2;
                } else {
                    return Err(XPathError::LexError { pos: i, msg: "expected !=".into() });
                }
            }
            '<' => {
                if i + 1 < bytes.len() && bytes[i+1] == b'=' { tokens.push(Token::Le); i += 2; }
                else { tokens.push(Token::Lt); i += 1; }
            }
            '>' => {
                if i + 1 < bytes.len() && bytes[i+1] == b'=' { tokens.push(Token::Ge); i += 2; }
                else { tokens.push(Token::Gt); i += 1; }
            }
            '\'' | '"' => {
                let q = c;
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] as char != q { i += 1; }
                if i >= bytes.len() {
                    return Err(XPathError::LexError { pos: start, msg: "unterminated string".into() });
                }
                tokens.push(Token::String(input[start..i].to_string()));
                i += 1;
            }
            '$' => {
                i += 1;
                let start = i;
                while i < bytes.len() && is_name_char(bytes[i] as char) { i += 1; }
                tokens.push(Token::VarRef(input[start..i].to_string()));
            }
            d if d.is_ascii_digit() => {
                let start = i;
                while i < bytes.len() && (bytes[i] as char).is_ascii_digit() { i += 1; }
                if i < bytes.len() && bytes[i] == b'.' {
                    i += 1;
                    while i < bytes.len() && (bytes[i] as char).is_ascii_digit() { i += 1; }
                }
                let n: f64 = input[start..i].parse().map_err(|_| XPathError::LexError {
                    pos: start, msg: "bad number".into() })?;
                tokens.push(Token::Number(n));
            }
            n if is_name_start(n) => {
                let start = i;
                while i < bytes.len() && is_name_char(bytes[i] as char) { i += 1; }
                let name = &input[start..i];
                // 关键字判断（仅在合适上下文，简化为 token 直接产出）
                let prev = tokens.last();
                let is_op_ctx = matches!(prev,
                    Some(Token::Name(_)) | Some(Token::Number(_)) | Some(Token::String(_)) |
                    Some(Token::RParen) | Some(Token::RBracket) | Some(Token::Star) |
                    Some(Token::Dot) | Some(Token::DoubleDot) | Some(Token::VarRef(_))
                );
                let tok = match name {
                    "and" if is_op_ctx => Token::And,
                    "or"  if is_op_ctx => Token::Or,
                    "mod" if is_op_ctx => Token::Mod,
                    "div" if is_op_ctx => Token::Div,
                    _ => Token::Name(name.to_string()),
                };
                tokens.push(tok);
            }
            _ => return Err(XPathError::LexError { pos: i, msg: format!("unexpected '{}'", c) }),
        }
    }
    tokens.push(Token::Eof);
    Ok(tokens)
}

fn is_name_start(c: char) -> bool { c.is_ascii_alphabetic() || c == '_' }
fn is_name_char(c: char) -> bool { c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' }
