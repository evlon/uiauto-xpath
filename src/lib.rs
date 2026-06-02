pub mod automation;
pub mod control_type;
pub mod element;
pub mod error;
pub mod xpath;

pub use control_type::{id_to_name as control_type_id_to_name, name_to_id as control_type_name_to_id};

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

// Re-export TreeScope and CacheRequest-related types for BuildCache usage
pub use windows::Win32::UI::Accessibility::TreeScope;
pub use windows::Win32::UI::Accessibility::IUIAutomationCacheRequest;
pub use element::{create_default_cache_request, DEFAULT_CACHE_PROPERTIES};
