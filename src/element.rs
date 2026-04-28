use crate::error::Result;
use windows::Win32::UI::Accessibility::{
    IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
};

#[derive(Clone)]
pub struct UiElement {
    pub(crate) raw: IUIAutomationElement,
    pub(crate) automation: IUIAutomation,
}

impl UiElement {
    pub fn new(raw: IUIAutomationElement, automation: IUIAutomation) -> Self {
        Self { raw, automation }
    }

    /// Get the underlying IUIAutomationElement for integration with other systems.
    pub fn raw_element(&self) -> &IUIAutomationElement {
        &self.raw
    }

    /// Get a clone of the underlying IUIAutomationElement.
    pub fn raw_element_clone(&self) -> IUIAutomationElement {
        self.raw.clone()
    }

    pub fn name(&self) -> String {
        unsafe { self.raw.CurrentName().map(|s| s.to_string()).unwrap_or_default() }
    }
    pub fn class_name(&self) -> String {
        unsafe { self.raw.CurrentClassName().map(|s| s.to_string()).unwrap_or_default() }
    }
    pub fn automation_id(&self) -> String {
        unsafe { self.raw.CurrentAutomationId().map(|s| s.to_string()).unwrap_or_default() }
    }
    pub fn control_type_name(&self) -> String {
        unsafe {
            self.raw.CurrentLocalizedControlType()
                .map(|s| s.to_string()).unwrap_or_default()
        }
    }
    pub fn control_type_id(&self) -> i32 {
        unsafe { self.raw.CurrentControlType().map(|c| c.0).unwrap_or(0) }
    }
    pub fn is_enabled(&self) -> bool {
        unsafe { self.raw.CurrentIsEnabled().map(|b| b.as_bool()).unwrap_or(false) }
    }
    pub fn is_offscreen(&self) -> bool {
        unsafe { self.raw.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(false) }
    }
    pub fn process_id(&self) -> i32 {
        unsafe { self.raw.CurrentProcessId().unwrap_or(0) }
    }
    pub fn help_text(&self) -> String {
        unsafe { self.raw.CurrentHelpText().map(|s| s.to_string()).unwrap_or_default() }
    }
    pub fn framework_id(&self) -> String {
        unsafe { self.raw.CurrentFrameworkId().map(|s| s.to_string()).unwrap_or_default() }
    }

    /// 获取属性的字符串表示，用于 XPath 属性匹配 (@xxx)
    pub fn get_property(&self, name: &str) -> Option<String> {
        match name.to_ascii_lowercase().as_str() {
            "name" => Some(self.name()),
            "classname" | "class" => Some(self.class_name()),
            "automationid" | "id" => Some(self.automation_id()),
            "controltype" | "type" => Some(self.control_type_name()),
            "controltypeid" => Some(self.control_type_id().to_string()),
            "enabled" | "isenabled" => Some(self.is_enabled().to_string()),
            "offscreen" | "isoffscreen" => Some(self.is_offscreen().to_string()),
            "processid" | "pid" => Some(self.process_id().to_string()),
            "helptext" => Some(self.help_text()),
            "frameworkid" => Some(self.framework_id()),
            _ => None,
        }
    }

    /// 节点名（XPath 节点测试）：使用控件类型本地化名 (Button, Edit, ...)
    pub fn node_name(&self) -> String {
        let s = self.control_type_name();
        if s.is_empty() { "Element".to_string() } else { s.replace(' ', "") }
    }

    pub fn children(&self) -> Result<Vec<UiElement>> {
        unsafe {
            let walker: IUIAutomationTreeWalker = self.automation.ControlViewWalker()?;
            let mut out = Vec::new();
            let mut child = walker.GetFirstChildElement(&self.raw).ok();
            while let Some(c) = child {
                out.push(UiElement::new(c.clone(), self.automation.clone()));
                child = walker.GetNextSiblingElement(&c).ok();
            }
            Ok(out)
        }
    }

    pub fn parent(&self) -> Option<UiElement> {
        unsafe {
            let walker = self.automation.ControlViewWalker().ok()?;
            walker.GetParentElement(&self.raw).ok()
                .map(|e| UiElement::new(e, self.automation.clone()))
        }
    }

    pub fn descendants(&self) -> Result<Vec<UiElement>> {
        let mut out = Vec::new();
        let mut stack = self.children()?;
        while let Some(e) = stack.pop() {
            for c in e.children()? { stack.push(c); }
            out.push(e);
        }
        Ok(out)
    }

    pub fn ancestors(&self) -> Vec<UiElement> {
        let mut out = Vec::new();
        let mut cur = self.parent();
        while let Some(e) = cur {
            cur = e.parent();
            out.push(e);
        }
        out
    }

    pub fn following_siblings(&self) -> Result<Vec<UiElement>> {
        unsafe {
            let walker = self.automation.ControlViewWalker()?;
            let mut out = Vec::new();
            let mut s = walker.GetNextSiblingElement(&self.raw).ok();
            while let Some(c) = s {
                out.push(UiElement::new(c.clone(), self.automation.clone()));
                s = walker.GetNextSiblingElement(&c).ok();
            }
            Ok(out)
        }
    }

    pub fn preceding_siblings(&self) -> Result<Vec<UiElement>> {
        unsafe {
            let walker = self.automation.ControlViewWalker()?;
            let mut out = Vec::new();
            let mut s = walker.GetPreviousSiblingElement(&self.raw).ok();
            while let Some(c) = s {
                out.push(UiElement::new(c.clone(), self.automation.clone()));
                s = walker.GetPreviousSiblingElement(&c).ok();
            }
            Ok(out)
        }
    }

    pub fn equals(&self, other: &UiElement) -> bool {
        unsafe {
            self.automation.CompareElements(&self.raw, &other.raw)
                .map(|b| b.as_bool()).unwrap_or(false)
        }
    }
}

impl std::fmt::Debug for UiElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiElement")
            .field("name", &self.name())
            .field("type", &self.control_type_name())
            .field("id", &self.automation_id())
            .finish()
    }
}
