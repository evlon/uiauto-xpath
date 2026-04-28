use thiserror::Error;

#[derive(Debug, Error)]
pub enum XPathError {
    #[error("Lex error at {pos}: {msg}")]
    LexError { pos: usize, msg: String },
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Evaluation error: {0}")]
    EvalError(String),
    #[error("Function '{0}' not found")]
    UnknownFunction(String),
    #[error("Wrong arity for function '{name}': expected {expected}, got {got}")]
    Arity { name: String, expected: String, got: usize },
    #[error("Type error: {0}")]
    TypeError(String),
    #[error("UIA error: {0}")]
    UiaError(String),
    #[error("Windows error: {0}")]
    Windows(#[from] windows::core::Error),
}

pub type Result<T> = std::result::Result<T, XPathError>;
