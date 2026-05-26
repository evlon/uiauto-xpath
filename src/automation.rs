use crate::element::UiElement;
use crate::error::Result;
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

pub struct UiAutomation {
    pub(crate) inner: IUIAutomation,
}

impl UiAutomation {
    pub fn new() -> Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let inner: IUIAutomation =
                windows::Win32::System::Com::CoCreateInstance(&CUIAutomation, None,
                    windows::Win32::System::Com::CLSCTX_INPROC_SERVER)?;
            Ok(Self { inner })
        }
    }

    pub fn root(&self) -> Result<UiElement> {
        unsafe {
            let elem = self.inner.GetRootElement()?;
            Ok(UiElement::new(elem, self.inner.clone()))
        }
    }

    pub fn from_handle(&self, hwnd: isize) -> Result<UiElement> {
        unsafe {
            let elem = self.inner.ElementFromHandle(windows::Win32::Foundation::HWND(hwnd as _))?;
            Ok(UiElement::new(elem, self.inner.clone()))
        }
    }

    /// Get UI Automation element at the given screen coordinates.
    pub fn from_point(&self, x: i32, y: i32) -> Result<UiElement> {
        unsafe {
            let elem = self.inner.ElementFromPoint(POINT { x, y })?;
            Ok(UiElement::new(elem, self.inner.clone()))
        }
    }
}
