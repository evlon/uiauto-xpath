use crate::error::Result;
use windows::Win32::UI::Accessibility::{
    IUIAutomation, IUIAutomationCondition, IUIAutomationElement, IUIAutomationTreeWalker,
    TreeScope_Children, TreeScope_Descendants,
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
            "controltype" | "type" => Some(control_type_id_to_name(self.control_type_id())),
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
        control_type_id_to_name(self.control_type_id())
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
}

/// 将 UIA ControlType 整数 ID 转换为标准英文名称
fn control_type_id_to_name(id: i32) -> String {
    match id {
        50000 => "Button",
        50001 => "Calendar",
        50002 => "CheckBox",
        50003 => "ComboBox",
        50004 => "Edit",
        50005 => "Hyperlink",
        50006 => "Image",
        50007 => "ListItem",
        50008 => "List",
        50009 => "Menu",
        50010 => "MenuBar",
        50011 => "MenuItem",
        50012 => "ProgressBar",
        50013 => "RadioButton",
        50014 => "ScrollBar",
        50015 => "Slider",
        50016 => "Spinner",
        50017 => "StatusBar",
        50018 => "Tab",
        50019 => "TabItem",
        50020 => "Text",
        50021 => "ToolBar",
        50022 => "ToolTip",
        50023 => "Tree",
        50024 => "TreeItem",
        50025 => "Custom",
        50026 => "Group",
        50027 => "Thumb",
        50028 => "DataGrid",
        50029 => "DataItem",
        50030 => "Document",
        50031 => "SplitButton",
        50032 => "Window",
        50033 => "Pane",
        50034 => "Header",
        50035 => "HeaderItem",
        50036 => "Table",
        50037 => "TitleBar",
        50038 => "Separator",
        50039 => "SemanticZoom",
        50040 => "AppBar",
        50041 => "Pane",
        _ => "Element",
    }.to_string()
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
