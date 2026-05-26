pub mod automation;
pub mod element;
pub mod error;
pub mod xpath;

pub use automation::UiAutomation;
pub use element::UiElement;
pub use error::{XPathError, Result};
pub use xpath::{XPath, evaluator::XPathResult};
// 导出 optimizer 的核心算法函数供外部使用
pub use xpath::{
    is_dynamic_class,
    extract_stable_prefix,
    tag_uniqueness_bonus,
    is_generic_control_type,
    split_camel,
    OptimizeOptions,
    OptimizeResult,
};

// Re-export TreeScope for find_all usage
pub use windows::Win32::UI::Accessibility::TreeScope;
