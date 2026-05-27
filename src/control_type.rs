/// Unified UIA ControlType ID ↔ name mapping.
///
/// All projects should use these constants to stay in sync.
/// IDs are defined by Microsoft: https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-controltype-ids
/// The numeric values correspond to `UIA_*ControlTypeId` from windows::Win32::UI::Accessibility.

/// Map a ControlType integer ID to its standard English name.
/// Returns `"Element"` for unknown IDs.
pub fn id_to_name(id: i32) -> &'static str {
    // IDs 50000–50040 map to the standard control type names.
    // Keep this table in numeric order for maintainability.
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
        _ => "Element",
    }
}

/// Map a standard English ControlType name back to its UIA integer ID.
/// Returns `None` for unrecognized names.
pub fn name_to_id(name: &str) -> Option<i32> {
    match name {
        "Button"       => Some(50000),
        "Calendar"     => Some(50001),
        "CheckBox"     => Some(50002),
        "ComboBox"     => Some(50003),
        "Edit"         => Some(50004),
        "Hyperlink"    => Some(50005),
        "Image"        => Some(50006),
        "ListItem"     => Some(50007),
        "List"         => Some(50008),
        "Menu"         => Some(50009),
        "MenuBar"      => Some(50010),
        "MenuItem"     => Some(50011),
        "ProgressBar"  => Some(50012),
        "RadioButton"  => Some(50013),
        "ScrollBar"    => Some(50014),
        "Slider"       => Some(50015),
        "Spinner"      => Some(50016),
        "StatusBar"    => Some(50017),
        "Tab"          => Some(50018),
        "TabItem"      => Some(50019),
        "Text"         => Some(50020),
        "ToolBar"      => Some(50021),
        "ToolTip"      => Some(50022),
        "Tree"         => Some(50023),
        "TreeItem"     => Some(50024),
        "Custom"       => Some(50025),
        "Group"        => Some(50026),
        "Thumb"        => Some(50027),
        "DataGrid"     => Some(50028),
        "DataItem"     => Some(50029),
        "Document"     => Some(50030),
        "SplitButton"  => Some(50031),
        "Window"       => Some(50032),
        "Pane"         => Some(50033),
        "Header"       => Some(50034),
        "HeaderItem"   => Some(50035),
        "Table"        => Some(50036),
        "TitleBar"     => Some(50037),
        "Separator"    => Some(50038),
        "SemanticZoom" => Some(50039),
        "AppBar"       => Some(50040),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all() {
        for id in 50000..=50040 {
            let name = id_to_name(id);
            assert_eq!(name_to_id(name), Some(id), "roundtrip failed for id={}", id);
        }
    }

    #[test]
    fn unknown_id_returns_element() {
        assert_eq!(id_to_name(0), "Element");
        assert_eq!(id_to_name(99999), "Element");
    }

    #[test]
    fn unknown_name_returns_none() {
        assert!(name_to_id("Bogus").is_none());
        assert!(name_to_id("").is_none());
    }
}
