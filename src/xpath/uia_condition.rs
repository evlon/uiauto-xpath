use super::ast::*;
use super::context::Context;
use super::evaluator::eval_with_attrs;
use crate::element::UiElement;
use crate::error::{Result, XPathError};
use log::{debug, warn};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::System::Variant::*;
use windows::core::BSTR;

/// 谓词分析结果
#[derive(Debug, Clone)]
pub struct PredicateAnalysis {
    /// 可以用 UIA Condition 表达的简单谓词索引
    pub simple_indices: Vec<usize>,
    /// 不能用 Condition 表达的复杂谓词索引
    pub complex_indices: Vec<usize>,
    /// 是否值得使用 FindAll 优化（至少有一个简单谓词）
    pub can_optimize: bool,
    /// 优化预期收益评估（0.0-1.0）
    pub expected_benefit: f64,
}

/// 分析 Step 的谓词，判断是否适合用 FindAll 优化
pub fn analyze_predicates(predicates: &[Expr]) -> PredicateAnalysis {
    let mut simple_indices = Vec::new();
    let mut complex_indices = Vec::new();
    
    for (i, pred) in predicates.iter().enumerate() {
        // 递归提取简单条件和复杂条件
        let (simple_conds, has_complex) = extract_conditions_from_expr(pred);
        
        if !simple_conds.is_empty() {
            // 有简单条件，可以优化
            simple_indices.push(i);
            log::debug!("[UIA Condition] Predicate {} has {} simple conditions", i, simple_conds.len());
        }
        
        if has_complex {
            // 包含复杂条件，需要二次过滤
            complex_indices.push(i);
            log::debug!("[UIA Condition] Predicate {} has complex conditions", i);
        }
    }
    
    let can_optimize = !simple_indices.is_empty();
    
    log::debug!("[UIA Condition] Analysis result: simple={}, complex={}, can_optimize={}", 
        simple_indices.len(), complex_indices.len(), can_optimize);
    
    // 计算预期收益：简单谓词越多，收益越高
    let expected_benefit = if predicates.is_empty() {
        0.0
    } else {
        simple_indices.len() as f64 / predicates.len() as f64
    };
    
    PredicateAnalysis {
        simple_indices,
        complex_indices,
        can_optimize,
        expected_benefit,
    }
}

/// 从表达式中递归提取简单条件，并检测是否有复杂条件
/// 返回: (简单条件列表, 是否包含复杂条件)
fn extract_conditions_from_expr(expr: &Expr) -> (Vec<&Expr>, bool) {
    match expr {
        // 如果是简单的 @attr = 'value'，直接返回
        Expr::BinaryOp(BinOp::Eq, left, right) 
            if is_attr_ref(left) && is_string_literal(right) => {
            (vec![expr], false)
        },
        // 如果是 AND 表达式，递归提取两边的条件
        Expr::BinaryOp(BinOp::And, left, right) => {
            let (left_simple, left_complex) = extract_conditions_from_expr(left);
            let (right_simple, right_complex) = extract_conditions_from_expr(right);
            
            let mut result = left_simple;
            result.extend(right_simple);
            
            let has_complex = left_complex || right_complex;
            (result, has_complex)
        },
        // ★ 特殊处理：starts-with() 等函数调用
        // 虽然不能直接用 UIA Condition，但应该允许其他简单条件使用 FindAll
        Expr::FunctionCall { name, args: _ } => {
            // 函数调用本身是复杂条件，但不阻止其他条件的优化
            log::debug!("[UIA Condition] Detected function call (complex predicate): {}", name);
            (vec![], true)
        },
        // 其他情况（比较运算符等），视为复杂条件
        _ => {
            log::debug!("[UIA Condition] Detected complex expression: {:?}", expr);
            (vec![], true)
        }
    }
}

/// 判断是否是属性引用：@attr
fn is_attr_ref(expr: &Expr) -> bool {
    matches!(expr, Expr::Path(p) 
        if !p.absolute 
        && p.steps.len() == 1 
        && p.steps[0].axis == Axis::Attribute
        && matches!(p.steps[0].test, NodeTest::Name(_)))
}

/// 判断是否是字符串字面量
fn is_string_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::String(_))
}

/// 从简单谓词构建 UIA Condition
pub fn build_condition_from_analysis(
    auto: &IUIAutomation,
    predicates: &[Expr],
    analysis: &PredicateAnalysis
) -> Result<IUIAutomationCondition> {
    let mut conditions = Vec::new();
    
    log::debug!("[UIA Condition] Building condition from {} simple predicates", analysis.simple_indices.len());
    
    for &idx in &analysis.simple_indices {
        let pred = &predicates[idx];
        // 递归提取并构建条件
        collect_conditions_from_expr(auto, pred, &mut conditions)?;
    }
    
    log::debug!("[UIA Condition] Built {} conditions total", conditions.len());
    
    // 合并条件
    if conditions.is_empty() {
        Err(XPathError::EvalError("No valid conditions to build".into()))
    } else if conditions.len() == 1 {
        Ok(conditions.remove(0))
    } else {
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = unsafe {
                auto.CreateAndCondition(&combined, &cond)
                    .map_err(|e| {
                        XPathError::EvalError(format!("CreateAndCondition failed: {:?}", e))
                    })?
            };
        }
        Ok(combined)
    }
}

/// 递归从表达式中收集条件
fn collect_conditions_from_expr(
    auto: &IUIAutomation,
    expr: &Expr,
    conditions: &mut Vec<IUIAutomationCondition>
) -> Result<()> {
    match expr {
        // 简单的 @attr = 'value'
        Expr::BinaryOp(BinOp::Eq, attr_expr, value_expr) 
            if is_attr_ref(attr_expr) && is_string_literal(value_expr) => {
            // 提取属性名
            let attr_name = if let Expr::Path(p) = attr_expr.as_ref() {
                if let NodeTest::Name(name) = &p.steps[0].test {
                    name
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            };
            
            // 提取字符串值
            let value = if let Expr::String(s) = value_expr.as_ref() {
                s
            } else {
                return Ok(());
            };
            
            log::debug!("[UIA Condition] Adding predicate: @{} = '{}'", attr_name, value);
            
            // 映射到 UIA Property ID
            if let Some(prop_id) = map_property_name_to_id(attr_name) {
                unsafe {
                    // 特殊处理：ControlType 需要整数类型，不是字符串
                    let variant = if prop_id == UIA_ControlTypePropertyId {
                        // 将字符串 'Pane' 等转换为 UIA_ControlType_ID
                        match map_control_type_name_to_id(value) {
                            Some(control_type_id) => {
                                log::debug!("[UIA Condition] Mapping ControlType '{}' to ID {}", value, control_type_id);
                                VARIANT::from(control_type_id as i32)
                            },
                            None => {
                                warn!("[UIA Condition] Unknown ControlType '{}', skipping", value);
                                return Ok(());
                            }
                        }
                    } else {
                        // 其他属性使用字符串
                        VARIANT::from(BSTR::from(value.as_str()))
                    };
                    
                    let cond = auto.CreatePropertyCondition(prop_id, &variant)
                        .map_err(|e| {
                            XPathError::EvalError(format!(
                                "CreatePropertyCondition for {} failed: {:?}", 
                                attr_name, e
                            ))
                        })?;
                    conditions.push(cond);
                    log::debug!("[UIA Condition] Successfully created condition for @{}", attr_name);
                }
            } else {
                warn!("[UIA Condition] Unsupported property: {}", attr_name);
            }
        },
        // AND 表达式，递归处理两边
        Expr::BinaryOp(BinOp::And, left, right) => {
            collect_conditions_from_expr(auto, left, conditions)?;
            collect_conditions_from_expr(auto, right, conditions)?;
        },
        // 其他情况，忽略
        _ => {}
    }
    Ok(())
}

/// 将 XPath 属性名映射到 UIA Property ID
fn map_property_name_to_id(name: &str) -> Option<UIA_PROPERTY_ID> {
    match name.to_ascii_lowercase().as_str() {
        "controltype" | "type" => Some(UIA_ControlTypePropertyId),
        "automationid" | "id" => Some(UIA_AutomationIdPropertyId),
        "classname" | "class" => Some(UIA_ClassNamePropertyId),
        "name" => Some(UIA_NamePropertyId),
        "frameworkid" => Some(UIA_FrameworkIdPropertyId),
        "localizedcontroltype" => Some(UIA_LocalizedControlTypePropertyId),
        "helptext" => Some(UIA_HelpTextPropertyId),
        "acceleratorkey" => Some(UIA_AcceleratorKeyPropertyId),
        "accesskey" => Some(UIA_AccessKeyPropertyId),
        "itemtype" => Some(UIA_ItemTypePropertyId),
        "itemstatus" => Some(UIA_ItemStatusPropertyId),
        _ => {
            debug!("[UIA Condition] Unsupported property '{}', skipping", name);
            None
        },
    }
}

/// 将 ControlType 字符串名称映射到 UIA_ControlType_ID
fn map_control_type_name_to_id(name: &str) -> Option<u32> {
    // UIA_ControlType_ID 定义在 windows::Win32::UI::Accessibility 中
    // 参考：https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-controltype-ids
    match name {
        "Button" => Some(0xC350),       // UIA_ButtonControlTypeId
        "Calendar" => Some(0xC351),     // UIA_CalendarControlTypeId
        "CheckBox" => Some(0xC352),     // UIA_CheckBoxControlTypeId
        "ComboBox" => Some(0xC353),     // UIA_ComboBoxControlTypeId
        "Edit" => Some(0xC354),         // UIA_EditControlTypeId
        "Hyperlink" => Some(0xC355),    // UIA_HyperlinkControlTypeId
        "Image" => Some(0xC356),        // UIA_ImageControlTypeId
        "ListItem" => Some(0xC357),     // UIA_ListItemControlTypeId
        "List" => Some(0xC358),         // UIA_ListControlTypeId
        "Menu" => Some(0xC359),         // UIA_MenuControlTypeId
        "MenuBar" => Some(0xC35A),      // UIA_MenuBarControlTypeId
        "MenuItem" => Some(0xC35B),     // UIA_MenuItemControlTypeId
        "ProgressBar" => Some(0xC35C),  // UIA_ProgressBarControlTypeId
        "RadioButton" => Some(0xC35D),  // UIA_RadioButtonControlTypeId
        "ScrollBar" => Some(0xC35E),    // UIA_ScrollBarControlTypeId
        "Slider" => Some(0xC35F),       // UIA_SliderControlTypeId
        "Spinner" => Some(0xC360),      // UIA_SpinnerControlTypeId
        "StatusBar" => Some(0xC361),    // UIA_StatusBarControlTypeId
        "Tab" => Some(0xC362),          // UIA_TabControlTypeId
        "TabItem" => Some(0xC363),      // UIA_TabItemControlTypeId
        "Text" => Some(0xC364),         // UIA_TextControlTypeId
        "ToolBar" => Some(0xC365),      // UIA_ToolBarControlTypeId
        "ToolTip" => Some(0xC366),      // UIA_ToolTipControlTypeId
        "Tree" => Some(0xC367),         // UIA_TreeControlTypeId
        "TreeItem" => Some(0xC368),     // UIA_TreeItemControlTypeId
        "Custom" => Some(0xC369),       // UIA_CustomControlTypeId
        "Group" => Some(0xC36A),        // UIA_GroupControlTypeId
        "Thumb" => Some(0xC36B),        // UIA_ThumbControlTypeId
        "DataGrid" => Some(0xC36C),     // UIA_DataGridControlTypeId
        "DataItem" => Some(0xC36D),     // UIA_DataItemControlTypeId
        "Document" => Some(0xC36E),     // UIA_DocumentControlTypeId
        "SplitButton" => Some(0xC36F),  // UIA_SplitButtonControlTypeId
        "Window" => Some(0xC370),       // UIA_WindowControlTypeId
        "Pane" => Some(0xC371),         // UIA_PaneControlTypeId
        "Header" => Some(0xC372),       // UIA_HeaderControlTypeId
        "HeaderItem" => Some(0xC373),   // UIA_HeaderItemControlTypeId
        "Table" => Some(0xC374),        // UIA_TableControlTypeId
        "TitleBar" => Some(0xC375),     // UIA_TitleBarControlTypeId
        "Separator" => Some(0xC376),    // UIA_SeparatorControlTypeId
        "SemanticZoom" => Some(0xC377), // UIA_SemanticZoomControlTypeId
        "AppBar" => Some(0xC378),       // UIA_AppBarControlTypeId
        _ => None,
    }
}

/// 在 Rust 层应用复杂谓词过滤
pub fn apply_complex_predicates(
    candidates: Vec<UiElement>,
    predicates: &[Expr],
    complex_indices: &[usize],
    ctx: &Context
) -> Result<Vec<UiElement>> {
    if complex_indices.is_empty() {
        return Ok(candidates);
    }
    
    let mut result = Vec::new();
    for elem in candidates {
        let mut keep = true;
        for &idx in complex_indices {
            let pred = &predicates[idx];
            let sub_ctx = ctx.with_node(elem.clone(), 1, 1);
            let eval_result = eval_with_attrs(pred, &sub_ctx)?;
            if !eval_result.to_boolean() {
                keep = false;
                debug!(
                    "[UIA Condition] Element filtered out by complex predicate [{}]: {:?}",
                    idx, pred
                );
                break;
            }
        }
        if keep {
            result.push(elem);
        }
    }
    Ok(result)
}

/// 诊断报告：当 FindAll 返回空结果时的分析
pub fn diagnose_empty_result(
    node: &UiElement,
    axis: Axis,
    predicates: &[Expr],
    analysis: &PredicateAnalysis
) {
    debug!("[UIA Condition] FindAll returned empty result, diagnosing...");
    debug!("  Node: {} class='{}' name='{}'", 
        node.node_name(), node.class_name(), node.name());
    debug!("  Axis: {:?}", axis);
    debug!("  Predicates: {} total, {} simple, {} complex",
        predicates.len(), analysis.simple_indices.len(), analysis.complex_indices.len());
    
    // 检查每个简单谓词对应的属性值
    for &idx in &analysis.simple_indices {
        if let Expr::BinaryOp(BinOp::Eq, attr_expr, value_expr) = &predicates[idx] {
            if let Expr::Path(p) = attr_expr.as_ref() {
                if let NodeTest::Name(attr_name) = &p.steps[0].test {
                    if let Expr::String(expected_value) = value_expr.as_ref() {
                        let actual_value = node.get_property(attr_name).unwrap_or_default();
                        debug!("  Predicate [{}]: @{} = '{}' (actual: '{}')",
                            idx, attr_name, expected_value, actual_value);
                    }
                }
            }
        }
    }
    
    // 建议：是否需要调整策略
    if analysis.simple_indices.len() == predicates.len() {
        debug!("  Suggestion: All predicates are simple, but no match found.");
        debug!("  Possible causes:");
        debug!("    1. Wrong axis (should use Descendant instead of Child?)");
        debug!("    2. Element doesn't exist in the tree");
        debug!("    3. Property values changed dynamically");
    } else {
        debug!("  Suggestion: Complex predicates may be too restrictive.");
    }
}
