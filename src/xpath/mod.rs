pub mod ast;
pub mod axes;
pub mod context;
pub mod evaluator;
pub mod functions;
pub mod lexer;
pub mod parser;
pub mod value;
pub mod optimizer;
pub mod uia_condition;

// 导出 optimizer 的核心算法函数供外部使用
pub use optimizer::{
    is_dynamic_class,
    extract_stable_prefix,
    tag_uniqueness_bonus,
    is_generic_control_type,
    split_camel,
    OptimizeOptions,
    OptimizeResult,
    optimize_minimal_with_cancel,
};

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
    
    /// 极简优化：通过尝试验证移除所有非必要属性
    pub fn optimize_minimal<F, P>(
        xpath: &str,
        verify_callback: F,
        progress_callback: P,
    ) -> Result<Option<String>>
    where
        F: Fn(&str) -> Result<bool>,
        P: Fn(&str),
    {
        use std::sync::{Arc, atomic::AtomicBool};
        let cancel_flag = Arc::new(AtomicBool::new(false));
        optimize_minimal_with_cancel(xpath, verify_callback, progress_callback, cancel_flag)
    }
}
