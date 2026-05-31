use crate::control_type::id_to_name;
use crate::error::Result;
use windows::Win32::UI::Accessibility::{
    IUIAutomation, IUIAutomationCondition, IUIAutomationElement, IUIAutomationTreeWalker,
    TreeScope, TreeScope_Children, TreeScope_Descendants,
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

    /// Reconstruct a UiElement from raw IUIAutomationElement and IUIAutomation.
    /// Useful for integration with external systems that already have raw elements.
    pub fn from_raw(raw: IUIAutomationElement, automation: IUIAutomation) -> Self {
        Self { raw, automation }
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
    pub fn control_type_id(&self) -> i32 {
        unsafe { self.raw.CurrentControlType().map(|c| c.0).unwrap_or(0) }
    }
    pub fn is_enabled(&self) -> bool {
        unsafe { self.raw.CurrentIsEnabled().map(|b| b.as_bool()).unwrap_or(false) }
    }
    pub fn is_offscreen(&self) -> bool {
        unsafe { self.raw.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(false) }
    }

    /// Get RuntimeId as a Vec<i32>, for deduplication purposes.
    pub fn runtime_id(&self) -> Option<Vec<i32>> {
        unsafe {
            let variant = self.raw.GetRuntimeId().ok()?;
            let len = (*variant).rgsabound[0].cElements as usize;
            let ptr = (*variant).pvData as *const i32;
            if ptr.is_null() || len == 0 {
                return None;
            }
            Some(std::slice::from_raw_parts(ptr, len).to_vec())
        }
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
    pub fn is_password(&self) -> bool {
        unsafe { self.raw.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false) }
    }
    pub fn accelerator_key(&self) -> String {
        unsafe { self.raw.CurrentAcceleratorKey().map(|s| s.to_string()).unwrap_or_default() }
    }
    pub fn access_key(&self) -> String {
        unsafe { self.raw.CurrentAccessKey().map(|s| s.to_string()).unwrap_or_default() }
    }
    pub fn item_type(&self) -> String {
        unsafe { self.raw.CurrentItemType().map(|s| s.to_string()).unwrap_or_default() }
    }
    pub fn item_status(&self) -> String {
        unsafe { self.raw.CurrentItemStatus().map(|s| s.to_string()).unwrap_or_default() }
    }
    pub fn localized_control_type(&self) -> String {
        unsafe { self.raw.CurrentLocalizedControlType().map(|s| s.to_string()).unwrap_or_default() }
    }

    /// 获取属性的字符串表示，用于 XPath 属性匹配 (@xxx)
    pub fn get_property(&self, name: &str) -> Option<String> {
        match name.to_ascii_lowercase().as_str() {
            "name" => Some(self.name()),
            "classname" | "class" => Some(self.class_name()),
            "automationid" | "id" => Some(self.automation_id()),
            // ControlType: 返回标准英文名称（与 element-selector 一致）
            // 整数映射：UIA50000=Button, UIA50003=Pane, UIA50004=Document, etc.
            "controltype" | "type" => Some(id_to_name(self.control_type_id()).to_string()),
            "controltypeid" => Some(self.control_type_id().to_string()),
            "enabled" | "isenabled" => Some(self.is_enabled().to_string()),
            "offscreen" | "isoffscreen" => Some(self.is_offscreen().to_string()),
            "processid" | "pid" => Some(self.process_id().to_string()),
            "helptext" => Some(self.help_text()),
            "frameworkid" => Some(self.framework_id()),
            "ispassword" | "password" => Some(self.is_password().to_string()),
            "acceleratorkey" => Some(self.accelerator_key()),
            "accesskey" => Some(self.access_key()),
            "itemtype" => Some(self.item_type()),
            "itemstatus" => Some(self.item_status()),
            "localizedcontroltype" => Some(self.localized_control_type()),
            _ => None,
        }
    }

    /// 节点名（XPath 节点测试）：使用控件类型标准英文名称 (Button, Pane, Document, ...)
    /// 与 element-selector 的 control_type_name 保持一致
    pub fn node_name(&self) -> String {
        id_to_name(self.control_type_id()).to_string()
    }

    /// Get children using ControlViewWalker (standard UIA control view).
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

    /// Get children using RawViewWalker (full UIA raw tree, includes elements
    /// filtered out of the control view such as Qt/Chrome intermediate layers).
    /// Falls back to ControlViewWalker if RawViewWalker is unavailable.
    pub fn raw_children(&self) -> Result<Vec<UiElement>> {
        unsafe {
            let walker = match self.automation.RawViewWalker() {
                Ok(w) => w,
                Err(_) => {
                    log::warn!("[UiElement] RawViewWalker unavailable, falling back to ControlViewWalker");
                    self.automation.ControlViewWalker()?
                }
            };
            let mut out = Vec::new();
            let mut child = walker.GetFirstChildElement(&self.raw).ok();
            while let Some(c) = child {
                out.push(UiElement::new(c.clone(), self.automation.clone()));
                child = walker.GetNextSiblingElement(&c).ok();
            }
            Ok(out)
        }
    }

    /// Get parent using RawViewWalker (full UIA raw tree, consistent with capture-side).
    /// Falls back to ControlViewWalker if RawViewWalker is unavailable.
    pub fn parent(&self) -> Option<UiElement> {
        unsafe {
            let walker = match self.automation.RawViewWalker() {
                Ok(w) => w,
                Err(_) => self.automation.ControlViewWalker().ok()?,
            };
            walker.GetParentElement(&self.raw).ok()
                .map(|e| UiElement::new(e, self.automation.clone()))
        }
    }

    pub fn descendants(&self) -> Result<Vec<UiElement>> {
        let mut out = Vec::new();
        let mut stack: Vec<(UiElement, usize)> = self.children()?
            .into_iter().map(|c| (c, 1)).collect();
        const MAX_DEPTH: usize = 32;
        while let Some((e, depth)) = stack.pop() {
            if depth < MAX_DEPTH {
                for c in e.children()? {
                    stack.push((c, depth + 1));
                }
            }
            out.push(e);
        }
        Ok(out)
    }

    /// Get all descendants using RawViewWalker (full UIA raw tree).
    /// This includes elements that are filtered out of the control view,
    /// such as intermediate Group/Pane layers in Qt applications.
    /// Depth-limited to 32 levels to prevent runaway traversal.
    pub fn raw_descendants(&self) -> Result<Vec<UiElement>> {
        let mut out = Vec::new();
        let mut stack: Vec<(UiElement, usize)> = self.raw_children()?
            .into_iter().map(|c| (c, 1)).collect();
        const MAX_DEPTH: usize = 32;
        while let Some((e, depth)) = stack.pop() {
            if depth < MAX_DEPTH {
                for c in e.raw_children()? {
                    stack.push((c, depth + 1));
                }
            }
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

    /// Get following siblings using RawViewWalker (full UIA raw tree, consistent with capture-side).
    /// Falls back to ControlViewWalker if RawViewWalker is unavailable.
    pub fn following_siblings(&self) -> Result<Vec<UiElement>> {
        unsafe {
            let walker = match self.automation.RawViewWalker() {
                Ok(w) => w,
                Err(_) => self.automation.ControlViewWalker()?,
            };
            let mut out = Vec::new();
            let mut s = walker.GetNextSiblingElement(&self.raw).ok();
            while let Some(c) = s {
                out.push(UiElement::new(c.clone(), self.automation.clone()));
                s = walker.GetNextSiblingElement(&c).ok();
            }
            Ok(out)
        }
    }

    /// Get preceding siblings using RawViewWalker (full UIA raw tree, consistent with capture-side).
    /// Falls back to ControlViewWalker if RawViewWalker is unavailable.
    pub fn preceding_siblings(&self) -> Result<Vec<UiElement>> {
        unsafe {
            let walker = match self.automation.RawViewWalker() {
                Ok(w) => w,
                Err(_) => self.automation.ControlViewWalker()?,
            };
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

    /// 使用 UIA FindAll 快速查找子元素（带条件过滤）
    pub fn find_children_with_condition(
        &self,
        condition: &IUIAutomationCondition
    ) -> Result<Vec<UiElement>> {
        unsafe {
            let elements = self.raw.FindAll(TreeScope_Children, condition)?;
            let count = elements.Length()?;
            let mut result = Vec::with_capacity(count as usize);
            for i in 0..count {
                let elem = elements.GetElement(i)?;
                result.push(UiElement::new(elem, self.automation.clone()));
            }
            Ok(result)
        }
    }
    
    /// 使用 UIA FindAll 快速查找后代元素（带条件过滤）
    pub fn find_descendants_with_condition(
        &self,
        condition: &IUIAutomationCondition
    ) -> Result<Vec<UiElement>> {
        unsafe {
            let elements = self.raw.FindAll(TreeScope_Descendants, condition)?;
            let count = elements.Length()?;
            let mut result = Vec::with_capacity(count as usize);
            for i in 0..count {
                let elem = elements.GetElement(i)?;
                result.push(UiElement::new(elem, self.automation.clone()));
            }
            Ok(result)
        }
    }
    
    /// 使用 UIA FindFirst 查找第一个匹配的子元素
    pub fn find_first_child_with_condition(
        &self,
        condition: &IUIAutomationCondition
    ) -> Result<Option<UiElement>> {
        unsafe {
            match self.raw.FindFirst(TreeScope_Children, condition) {
                Ok(elem) => Ok(Some(UiElement::new(elem, self.automation.clone()))),
                Err(_) => Ok(None),
            }
        }
    }
    
    /// 使用 UIA FindFirst 查找第一个匹配的后代元素
    pub fn find_first_descendant_with_condition(
        &self,
        condition: &IUIAutomationCondition
    ) -> Result<Option<UiElement>> {
        unsafe {
            match self.raw.FindFirst(TreeScope_Descendants, condition) {
                Ok(elem) => Ok(Some(UiElement::new(elem, self.automation.clone()))),
                Err(_) => Ok(None),
            }
        }
    }

    /// Use UIA FindAll with a custom TreeScope to find elements.
    /// Supports: Children, Descendants, Subtree (Element|Descendants), Ancestors, Parent.
    pub fn find_all(
        &self,
        scope: TreeScope,
        condition: &IUIAutomationCondition,
    ) -> Result<Vec<UiElement>> {
        unsafe {
            let elements = self.raw.FindAll(scope, condition)?;
            let count = elements.Length()?;
            let mut result = Vec::with_capacity(count as usize);
            for i in 0..count {
                let elem = elements.GetElement(i)?;
                result.push(UiElement::new(elem, self.automation.clone()));
            }
            Ok(result)
        }
    }
}

impl std::fmt::Debug for UiElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiElement")
            .field("name", &self.name())
            .field("type", &self.localized_control_type())
            .field("id", &self.automation_id())
            .finish()
    }
}
