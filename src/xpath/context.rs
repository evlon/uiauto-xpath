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
    /// 严格 ControlView 模式：禁用 RawViewWalker 回退。
    /// 当 FindAll (Control View) 返回空时直接返回空，不回退到 RawViewWalker 遍历。
    /// 用于 `[fast]` 前缀的 XPath 定位 —— 性能极致，找不到就是找不到。
    pub strict_control_view: bool,
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
            strict_control_view: false,
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

    /// 启用严格 ControlView 模式
    pub fn with_strict_control_view(&self) -> Self {
        let mut c = self.clone();
        c.strict_control_view = true;
        c
    }
}
