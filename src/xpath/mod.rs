pub mod ast;
pub mod axes;
pub mod context;
pub mod evaluator;
pub mod functions;
pub mod lexer;
pub mod parser;
pub mod value;
pub mod optimizer;

use crate::element::UiElement;
use crate::error::Result;

pub struct XPath {
    pub(crate) expr: ast::Expr,
}

impl XPath {
    pub fn compile(expr: &str) -> Result<Self> {
        let tokens = lexer::tokenize(expr)?;
        let mut parser = parser::Parser::new(tokens);
        let expr = parser.parse_expr()?;
        Ok(Self { expr })
    }

    pub fn evaluate(&self, root: &UiElement) -> Result<value::Value> {
        let ctx = context::Context::new(root.clone());
        evaluator::eval(&self.expr, &ctx)
    }

    pub fn select_nodes(&self, root: &UiElement) -> Result<Vec<UiElement>> {
        match self.evaluate(root)? {
            value::Value::NodeSet(ns) => Ok(ns),
            other => Err(crate::error::XPathError::TypeError(
                format!("expected node-set, got {:?}", other.type_name())
            )),
        }
    }

    pub fn select_first(&self, root: &UiElement) -> Result<Option<UiElement>> {
        Ok(self.select_nodes(root)?.into_iter().next())
    }

    // src/xpath/mod.rs
    pub fn optimize(xpath: &str) -> Result<optimizer::OptimizeResult> {
        optimizer::optimize(xpath, &optimizer::OptimizeOptions::default())
    }
}
