use super::context::Context;
use super::value::{Value, string_value_of_node};
use crate::error::{Result, XPathError};
use regex::Regex;

pub fn call(name: &str, args: Vec<Value>, ctx: &Context) -> Result<Value> {
    macro_rules! arity {
        ($n:expr) => { if args.len() != $n {
            return Err(XPathError::Arity { name: name.into(), expected: $n.to_string(), got: args.len() });
        }};
        ($lo:expr, $hi:expr) => { #[allow(unused_comparisons)] if args.len() < $lo || args.len() > $hi {
            return Err(XPathError::Arity { name: name.into(),
                expected: format!("{}..{}", $lo, $hi), got: args.len() });
        }};
    }

    Ok(match name {
        // ===== Node Set Functions =====
        "last" => { arity!(0); Value::Number(ctx.size as f64) }
        "position" => { arity!(0); Value::Number(ctx.position as f64) }
        "count" => {
            arity!(1);
            match &args[0] {
                Value::NodeSet(ns) => Value::Number(ns.len() as f64),
                _ => return Err(XPathError::TypeError("count() expects node-set".into())),
            }
        }
        "id" => {
            arity!(1);
            // 在 UIA 上下文中按 AutomationId 查找
            let id_str = args[0].to_string_value();
            
            // ★ 优化：直接使用 FindFirst 查找 AutomationId
            use windows::Win32::UI::Accessibility::UIA_AutomationIdPropertyId;
            use windows::Win32::System::Variant::VARIANT;
            use windows::core::BSTR;
            
            unsafe {
                let condition = ctx.node.automation.CreatePropertyCondition(
                    UIA_AutomationIdPropertyId,
                    &VARIANT::from(BSTR::from(id_str.as_str()))
                );
                
                if let Ok(cond) = condition {
                    // 先尝试在当前节点的子元素中查找
                    match ctx.node.find_first_child_with_condition(&cond) {
                        Ok(Some(elem)) => {
                            log::debug!("[id()] Found element in children with AutomationId='{}'", id_str);
                            // 应用可见性过滤
                            if ctx.visibility_filter == super::context::VisibilityFilter::All {
                                return Ok(Value::NodeSet(vec![elem]));
                            } else {
                                let is_offscreen = elem.is_offscreen();
                                let visible_only = ctx.visibility_filter == super::context::VisibilityFilter::VisibleOnly;
                                let offscreen_only = ctx.visibility_filter == super::context::VisibilityFilter::OffscreenOnly;
                                
                                if (visible_only && !is_offscreen) || (offscreen_only && is_offscreen) {
                                    return Ok(Value::NodeSet(vec![elem]));
                                } else {
                                    log::debug!("[id()] Element filtered by visibility: is_offscreen={}, filter={:?}", is_offscreen, ctx.visibility_filter);
                                    return Ok(Value::NodeSet(vec![]));
                                }
                            }
                        },
                        Ok(None) => {
                            log::debug!("[id()] Not found in children, trying descendants...");
                        },
                        Err(e) => {
                            log::warn!("[id()] FindFirst on children failed: {:?}", e);
                        }
                    }
                    
                    // 再尝试在后代元素中查找
                    match ctx.node.find_first_descendant_with_condition(&cond) {
                        Ok(Some(elem)) => {
                            log::debug!("[id()] Found element in descendants with AutomationId='{}'", id_str);
                            // 应用可见性过滤
                            if ctx.visibility_filter == super::context::VisibilityFilter::All {
                                return Ok(Value::NodeSet(vec![elem]));
                            } else {
                                let is_offscreen = elem.is_offscreen();
                                let visible_only = ctx.visibility_filter == super::context::VisibilityFilter::VisibleOnly;
                                let offscreen_only = ctx.visibility_filter == super::context::VisibilityFilter::OffscreenOnly;
                                
                                if (visible_only && !is_offscreen) || (offscreen_only && is_offscreen) {
                                    return Ok(Value::NodeSet(vec![elem]));
                                } else {
                                    log::debug!("[id()] Element filtered by visibility: is_offscreen={}, filter={:?}", is_offscreen, ctx.visibility_filter);
                                    return Ok(Value::NodeSet(vec![]));
                                }
                            }
                        },
                        Ok(None) => {
                            log::debug!("[id()] No element found with AutomationId='{}'", id_str);
                            return Ok(Value::NodeSet(vec![]));
                        },
                        Err(e) => {
                            log::warn!("[id()] FindFirst on descendants failed: {:?}", e);
                            return Ok(Value::NodeSet(vec![]));
                        }
                    }
                } else {
                    log::warn!("[id()] Failed to create condition for AutomationId='{}'", id_str);
                    return Ok(Value::NodeSet(vec![]));
                }
            }
        }
        "local-name" | "name" => {
            arity!(0, 1);
            let node = match args.first() {
                Some(Value::NodeSet(ns)) => ns.first().cloned(),
                None => Some(ctx.node.clone()),
                _ => return Err(XPathError::TypeError(format!("{}() expects node-set", name))),
            };
            Value::String(node.map(|n| n.node_name()).unwrap_or_default())
        }
        "namespace-uri" => {
            arity!(0, 1);
            Value::String(String::new())
        }

        // ===== String Functions =====
        "string" => {
            arity!(0, 1);
            let v = args.into_iter().next().unwrap_or_else(|| Value::NodeSet(vec![ctx.node.clone()]));
            Value::String(v.to_string_value())
        }
        "concat" => {
            if args.is_empty() {
                return Err(XPathError::Arity { name: name.into(), expected: ">=1".into(), got: 0 });
            }
            Value::String(args.iter().map(|a| a.to_string_value()).collect::<String>())
        }
        "starts-with" => {
            arity!(2);
            Value::Boolean(args[0].to_string_value().starts_with(&args[1].to_string_value()))
        }
        "ends-with" => { // XPath 2.0
            arity!(2);
            Value::Boolean(args[0].to_string_value().ends_with(&args[1].to_string_value()))
        }
        "contains" => {
            arity!(2);
            Value::Boolean(args[0].to_string_value().contains(&args[1].to_string_value()))
        }
        "substring-before" => {
            arity!(2);
            let s = args[0].to_string_value();
            let p = args[1].to_string_value();
            Value::String(match s.find(&p) {
                Some(i) => s[..i].to_string(), None => String::new(),
            })
        }
        "substring-after" => {
            arity!(2);
            let s = args[0].to_string_value();
            let p = args[1].to_string_value();
            Value::String(match s.find(&p) {
                Some(i) => s[i + p.len()..].to_string(), None => String::new(),
            })
        }
        "substring" => {
            arity!(2, 3);
            let s = args[0].to_string_value();
            let chars: Vec<char> = s.chars().collect();
            let start = args[1].to_number().round() as i64;
            let end = if args.len() == 3 {
                start + args[2].to_number().round() as i64
            } else { chars.len() as i64 + 1 };
            let s1 = (start.max(1) - 1) as usize;
            let e1 = (end.max(1) - 1).min(chars.len() as i64) as usize;
            if s1 >= chars.len() || s1 >= e1 { Value::String(String::new()) }
            else { Value::String(chars[s1..e1].iter().collect()) }
        }
        "string-length" => {
            arity!(0, 1);
            let s = if args.is_empty() { string_value_of_node(&ctx.node) }
                    else { args[0].to_string_value() };
            Value::Number(s.chars().count() as f64)
        }
        "normalize-space" => {
            arity!(0, 1);
            let s = if args.is_empty() { string_value_of_node(&ctx.node) }
                    else { args[0].to_string_value() };
            Value::String(s.split_whitespace().collect::<Vec<_>>().join(" "))
        }
        "translate" => {
            arity!(3);
            let s = args[0].to_string_value();
            let from: Vec<char> = args[1].to_string_value().chars().collect();
            let to: Vec<char> = args[2].to_string_value().chars().collect();
            let result: String = s.chars().filter_map(|c| {
                match from.iter().position(|x| *x == c) {
                    Some(i) => to.get(i).copied(),
                    None => Some(c),
                }
            }).collect();
            Value::String(result)
        }
        "upper-case" => { arity!(1); Value::String(args[0].to_string_value().to_uppercase()) }
        "lower-case" => { arity!(1); Value::String(args[0].to_string_value().to_lowercase()) }
        "matches" | "match" => { // XPath 2.0 + SDK alias
            arity!(2, 3);
            let s = args[0].to_string_value();
            let p = args[1].to_string_value();
            let re = Regex::new(&p).map_err(|e| XPathError::EvalError(e.to_string()))?;
            Value::Boolean(re.is_match(&s))
        }
        "replace" => { // XPath 2.0
            arity!(3, 4);
            let s = args[0].to_string_value();
            let p = args[1].to_string_value();
            let r = args[2].to_string_value();
            let re = Regex::new(&p).map_err(|e| XPathError::EvalError(e.to_string()))?;
            Value::String(re.replace_all(&s, r.as_str()).to_string())
        }

        // ===== Boolean Functions =====
        "boolean" => { arity!(1); Value::Boolean(args[0].to_boolean()) }
        "not" => { arity!(1); Value::Boolean(!args[0].to_boolean()) }
        "true" => { arity!(0); Value::Boolean(true) }
        "false" => { arity!(0); Value::Boolean(false) }
        "lang" => {
            arity!(1);
            // UIA 无标准 xml:lang; 返回 false
            Value::Boolean(false)
        }

        // ===== Number Functions =====
        "number" => {
            arity!(0, 1);
            let v = args.into_iter().next()
                .unwrap_or_else(|| Value::NodeSet(vec![ctx.node.clone()]));
            Value::Number(v.to_number())
        }
        "sum" => {
            arity!(1);
            match &args[0] {
                Value::NodeSet(ns) => {
                    let s: f64 = ns.iter().map(|n| {
                        Value::String(string_value_of_node(n)).to_number()
                    }).sum();
                    Value::Number(s)
                }
                _ => return Err(XPathError::TypeError("sum() expects node-set".into())),
            }
        }
        "floor" => { arity!(1); Value::Number(args[0].to_number().floor()) }
        "ceiling" => { arity!(1); Value::Number(args[0].to_number().ceil()) }
        "round" => {
            arity!(1);
            let n = args[0].to_number();
            Value::Number(if n.is_nan() { f64::NAN } else { (n + 0.5).floor() })
        }
        "abs" => { arity!(1); Value::Number(args[0].to_number().abs()) }

        _ => return Err(XPathError::UnknownFunction(name.into())),
    })
}
