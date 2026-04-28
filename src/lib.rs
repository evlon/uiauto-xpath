pub mod automation;
pub mod element;
pub mod error;
pub mod xpath;

pub use automation::UiAutomation;
pub use element::UiElement;
pub use error::{XPathError, Result};
pub use xpath::{XPath, evaluator::XPathResult};
