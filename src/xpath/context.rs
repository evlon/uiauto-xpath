use crate::element::UiElement;
use std::collections::HashMap;
use super::value::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VisibilityFilter {
    All,
    VisibleOnly,
    OffscreenOnly,
}

#[derive(Clone)]
pub struct Context {
    pub root: UiElement,
    pub node: UiElement,
    pub position: usize,
    pub size: usize,
    pub vars: HashMap<String, Value>,
    pub visibility_filter: VisibilityFilter,
}

impl Context {
    pub fn new(root: UiElement) -> Self {
        Self {
            node: root.clone(),
            root,
            position: 1,
            size: 1,
            vars: HashMap::new(),
            visibility_filter: VisibilityFilter::All,
        }
    }

    pub fn with_node(&self, node: UiElement, pos: usize, size: usize) -> Self {
        let mut c = self.clone();
        c.node = node; c.position = pos; c.size = size;
        c
    }

    pub fn with_visibility_filter(&self, filter: VisibilityFilter) -> Self {
        let mut c = self.clone();
        c.visibility_filter = filter;
        c
    }
}
