use crate::element::UiElement;
use std::collections::HashMap;
use super::value::Value;

#[derive(Clone)]
pub struct Context {
    pub root: UiElement,
    pub node: UiElement,
    pub position: usize,
    pub size: usize,
    pub vars: HashMap<String, Value>,
}

impl Context {
    pub fn new(root: UiElement) -> Self {
        Self {
            node: root.clone(),
            root,
            position: 1,
            size: 1,
            vars: HashMap::new(),
        }
    }

    pub fn with_node(&self, node: UiElement, pos: usize, size: usize) -> Self {
        let mut c = self.clone();
        c.node = node; c.position = pos; c.size = size;
        c
    }
}
