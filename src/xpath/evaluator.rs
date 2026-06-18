use super::ast::*;
use super::axes;
use super::context::Context;
use super::functions;
use super::value::Value;
use crate::element::UiElement;
use crate::error::{Result, XPathError};
use std::collections::HashSet;

pub type XPathResult = Value;

pub fn eval(expr: &Expr, ctx: &Context) -> Result<Value> {
    match expr {
        Expr::Number(n) => Ok(Value::Number(*n)),
        Expr::String(s) => Ok(Value::String(s.clone())),
        Expr::VarRef(n) => ctx.vars.get(n).cloned()
            .ok_or_else(|| XPathError::EvalError(format!("undefined variable ${}", n))),
        Expr::UnaryMinus(e) => Ok(Value::Number(-eval(e, ctx)?.to_number())),
        Expr::Union(parts) => {
            let mut out: Vec<UiElement> = Vec::new();
            let mut seen_ids: HashSet<Vec<i32>> = HashSet::new();
            for p in parts {
                match eval(p, ctx)? {
                    Value::NodeSet(ns) => {
                        for n in ns {
                            if let Some(rid) = n.runtime_id() {
                                if seen_ids.insert(rid) {
                                    out.push(n);
                                }
                            } else if !out.iter().any(|x| x.equals(&n)) {
                                out.push(n);
                            }
                        }
                    }
                    _ => return Err(XPathError::TypeError("union requires node-sets".into())),
                }
            }
            Ok(Value::NodeSet(out))
        }
        Expr::BinaryOp(op, l, r) => eval_binop(*op, l, r, ctx),
        Expr::FunctionCall { name, args } => {
            let mut vs = Vec::with_capacity(args.len());
            for a in args { vs.push(eval(a, ctx)?); }
            functions::call(name, vs, ctx)
        }
        Expr::Path(p) => eval_path(p, ctx),
        Expr::Filter { primary, predicates, steps } => {
            let v = eval(primary, ctx)?;
            let mut nodes = match v {
                Value::NodeSet(ns) => ns,
                _ if predicates.is_empty() && steps.is_empty() => return Ok(v),
                _ => return Err(XPathError::TypeError("filter requires node-set".into())),
            };
            for p in predicates {
                nodes = apply_predicate(&nodes, p, ctx)?;
            }
            if !steps.is_empty() {
                nodes = step_through(nodes, steps, ctx)?;
            }
            Ok(Value::NodeSet(nodes))
        }
    }
}

fn eval_binop(op: BinOp, l: &Expr, r: &Expr, ctx: &Context) -> Result<Value> {
    match op {
        BinOp::Or => Ok(Value::Boolean(eval(l, ctx)?.to_boolean() || eval(r, ctx)?.to_boolean())),
        BinOp::And => Ok(Value::Boolean(eval(l, ctx)?.to_boolean() && eval(r, ctx)?.to_boolean())),
        BinOp::Eq | BinOp::NotEq => {
            let lv = eval(l, ctx)?; let rv = eval(r, ctx)?;
            let eq = compare_eq(&lv, &rv);
            Ok(Value::Boolean(if op == BinOp::Eq { eq } else { !eq }))
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let lv = eval(l, ctx)?; let rv = eval(r, ctx)?;
            Ok(Value::Boolean(compare_rel(op, &lv, &rv)))
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            let a = eval(l, ctx)?.to_number();
            let b = eval(r, ctx)?.to_number();
            Ok(Value::Number(match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => a / b,
                BinOp::Mod => a % b,
                _ => unreachable!(),
            }))
        }
    }
}

fn compare_eq(l: &Value, r: &Value) -> bool {
    use Value::*;
    match (l, r) {
        (NodeSet(a), NodeSet(b)) => {
            for x in a { for y in b {
                if super::value::string_value_of_node(x) == super::value::string_value_of_node(y) { return true; }
            }} false
        }
        (NodeSet(a), Number(n)) | (Number(n), NodeSet(a)) => {
            a.iter().any(|x| Value::String(super::value::string_value_of_node(x)).to_number() == *n)
        }
        (NodeSet(a), String(s)) | (String(s), NodeSet(a)) => {
            a.iter().any(|x| super::value::string_value_of_node(x) == *s)
        }
        (NodeSet(a), Boolean(b)) | (Boolean(b), NodeSet(a)) => {
            (!a.is_empty()) == *b
        }
        (Boolean(_), _) | (_, Boolean(_)) => l.to_boolean() == r.to_boolean(),
        (Number(_), _) | (_, Number(_)) => l.to_number() == r.to_number(),
        (String(a), String(b)) => a == b,
    }
}

fn compare_rel(op: BinOp, l: &Value, r: &Value) -> bool {
    let a = l.to_number(); let b = r.to_number();
    match op {
        BinOp::Lt => a < b, BinOp::Le => a <= b,
        BinOp::Gt => a > b, BinOp::Ge => a >= b,
        _ => false,
    }
}

fn eval_path(p: &PathExpr, ctx: &Context) -> Result<Value> {
    let start = if p.absolute { ctx.root.clone() } else { ctx.node.clone() };
    let nodes = vec![start];
    let result = step_through(nodes, &p.steps, ctx)?;
    Ok(Value::NodeSet(result))
}

// ═══════════════════════════════════════════════════════════════════
// step_through: 分发到 ControlView / RawView 实现
// ═══════════════════════════════════════════════════════════════════

fn step_through(nodes: Vec<UiElement>, steps: &[Step], ctx: &Context) -> Result<Vec<UiElement>> {
    if ctx.strict_control_view {
        step_through_control(nodes, steps, ctx)
    } else {
        step_through_raw(nodes, steps, ctx)
    }
}

// ═══════════════════════════════════════════════════════════════════
// 共享工具函数
// ═══════════════════════════════════════════════════════════════════

/// 追加 IsOffscreen=false 条件到现有 condition。
/// 让 UIA 服务端过滤掉不可见元素，大幅减少返回的候选节点数
/// （如 Chrome WebView 的 Group 有 2257 个子节点，大部分 offscreen）。
/// 追加 IsOffscreen=false 条件，返回新 condition。
/// 始终追加，让 UIA 服务端过滤掉不可见元素，大幅减少候选节点数。
fn with_is_offscreen_false(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
    condition: windows::Win32::UI::Accessibility::IUIAutomationCondition,
) -> windows::Win32::UI::Accessibility::IUIAutomationCondition {
    let is_offscreen_false = unsafe {
        use windows::Win32::System::Variant::*;
        let mut variant = VARIANT::default();
        let var_ptr = &mut variant as *mut VARIANT;
        std::ptr::write(var_ptr as *mut VARENUM, VT_BOOL);
        let bool_ptr = (var_ptr as *mut u8).add(8) as *mut i16;
        std::ptr::write(bool_ptr, 0i16);
        automation.CreatePropertyCondition(
            windows::Win32::UI::Accessibility::UIA_IsOffscreenPropertyId, &variant
        )
    };
    match is_offscreen_false {
        Ok(offscreen_cond) => unsafe {
            automation.CreateAndCondition(&condition, &offscreen_cond)
                .unwrap_or(condition)
        },
        _ => condition,
    }
}

/// UIA FindFirst: 返回最多 1 个匹配元素（最快路径）。
fn uia_search_first(
    n: &UiElement,
    condition: &windows::Win32::UI::Accessibility::IUIAutomationCondition,
    axis: Axis,
    cache_request: Option<&windows::Win32::UI::Accessibility::IUIAutomationCacheRequest>,
) -> Option<UiElement> {
    match (axis, cache_request) {
        (Axis::Child, Some(cr)) => n.find_first_child_with_condition_cached(condition, cr).ok().flatten(),
        (Axis::Descendant | Axis::DescendantOrSelf, Some(cr)) => n.find_first_descendant_with_condition_cached(condition, cr).ok().flatten(),
        (Axis::Child, None) => n.find_first_child_with_condition(condition).ok().flatten(),
        (Axis::Descendant | Axis::DescendantOrSelf, None) => n.find_first_descendant_with_condition(condition).ok().flatten(),
        _ => None,
    }
}

/// UIA FindAll: 返回所有匹配元素（用于 enable_findall=true 的回退路径）。
fn uia_search_all(
    n: &UiElement,
    condition: &windows::Win32::UI::Accessibility::IUIAutomationCondition,
    axis: Axis,
    cache_request: Option<&windows::Win32::UI::Accessibility::IUIAutomationCacheRequest>,
) -> Vec<UiElement> {
    match (axis, cache_request) {
        (Axis::Child, Some(cr)) => n.find_children_with_condition_cached(condition, cr).unwrap_or_default(),
        (Axis::Descendant | Axis::DescendantOrSelf, Some(cr)) => n.find_descendants_with_condition_cached(condition, cr).unwrap_or_default(),
        (Axis::Child, None) => n.find_children_with_condition(condition).unwrap_or_default(),
        (Axis::Descendant | Axis::DescendantOrSelf, None) => n.find_descendants_with_condition(condition).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// 去重：使用 RuntimeId 做 O(1) 查重，替代 O(N²) 的 equals() 线性扫描。
fn dedup_into(existing: &[UiElement], incoming: Vec<UiElement>) -> Vec<UiElement> {
    let mut seen_ids: HashSet<Vec<i32>> = HashSet::new();
    for e in existing {
        if let Some(rid) = e.runtime_id() {
            seen_ids.insert(rid);
        }
    }
    let mut result = Vec::with_capacity(incoming.len());
    for c in incoming {
        if let Some(rid) = c.runtime_id() {
            if seen_ids.insert(rid) {
                result.push(c);
            }
        } else if !existing.iter().any(|x| x.equals(&c)) && !result.iter().any(|x| x.equals(&c)) {
            result.push(c);
        }
    }
    result
}

/// 可见性过滤。
fn apply_visibility_filter(nodes: Vec<UiElement>, filter: super::context::VisibilityFilter) -> Vec<UiElement> {
    if filter == super::context::VisibilityFilter::All {
        return nodes;
    }
    let before = nodes.len();
    let result: Vec<UiElement> = nodes.into_iter().filter(|elem| {
        let is_offscreen = elem.is_offscreen();
        match filter {
            super::context::VisibilityFilter::VisibleOnly => !is_offscreen,
            super::context::VisibilityFilter::OffscreenOnly => is_offscreen,
            super::context::VisibilityFilter::All => true,
        }
    }).collect();
    log::debug!("[XPath] Visibility filter: {} -> {} nodes", before, result.len());
    result
}

/// 计算 skip_step_0 优化标记。
/// XPath `//Group[@Name='x']` 解析为 Step0=DescendantOrSelf::Node + Step1=Child::Group[@Name='x']。
/// 优化：跳过 Step0，直接在 Step1 用 FindFirst/FindAll(Descendants)。
fn compute_skip_step_0(steps: &[Step]) -> bool {
    steps.len() >= 2
        && matches!(steps[0].axis, Axis::DescendantOrSelf)
        && matches!(steps[0].test, NodeTest::Node)
        && steps[0].predicates.is_empty()
        && !steps[1].predicates.is_empty()
}

/// 计算 should_optimize 标记。
fn compute_should_optimize(effective_axis: Axis, analysis: &crate::xpath::uia_condition::PredicateAnalysis, test: &NodeTest) -> bool {
    let axis_ok = matches!(effective_axis, Axis::Child | Axis::Descendant | Axis::DescendantOrSelf);
    (analysis.can_optimize && axis_ok && analysis.expected_benefit >= 0.5)
    || (axis_ok && analysis.expected_benefit >= 0.5 && matches!(test, NodeTest::Name(_)))
}

// ═══════════════════════════════════════════════════════════════════
// ControlView 实现（strict_control_view=true）
//
// 特点：
// - FindAll 空时不回退 RawView（空即空）
// - Walker 回退使用 ControlViewWalker
// ═══════════════════════════════════════════════════════════════════

fn step_through_control(mut nodes: Vec<UiElement>, steps: &[Step], ctx: &Context) -> Result<Vec<UiElement>> {
    let total_start = std::time::Instant::now();
    log::debug!("[XPath ControlView] Starting with {} nodes, {} steps", nodes.len(), steps.len());

    if nodes.is_empty() {
        return Ok(Vec::new());
    }

    let skip_step_0 = compute_skip_step_0(steps);
    if skip_step_0 {
        log::debug!("[XPath ControlView] ★ Skipping Step 0 (DescendantOrSelf/Node), merging with Step 1");
    }

    for (step_idx, step) in steps.iter().enumerate() {
        let step_start = std::time::Instant::now();
        if skip_step_0 && step_idx == 0 {
            continue;
        }

        let effective_axis = if skip_step_0 && step_idx == 1 {
            Axis::Descendant
        } else {
            step.axis
        };

        let mut next: Vec<UiElement> = Vec::new();

        use crate::xpath::uia_condition;
        let analysis = uia_condition::analyze_predicates(&step.predicates, Some(&step.test));
        let should_optimize = compute_should_optimize(effective_axis, &analysis, &step.test);

        let cache_request = if should_optimize && !nodes.is_empty() {
            crate::element::create_default_cache_request(&nodes[0].automation).ok()
        } else {
            None
        };

        for (node_idx, n) in nodes.iter().enumerate() {
            let node_start = std::time::Instant::now();
            let (candidates, predicates_fully_applied) = if should_optimize {
                let cond_start = std::time::Instant::now();
                match uia_condition::build_condition_from_analysis(
                    &n.automation, &step.predicates, &analysis, Some(&step.test),
                ) {
                    Ok(mut condition) => {
                        log::debug!("[PERF][Control] step={} node={} build_condition: {}ms",
                            step_idx, node_idx, cond_start.elapsed().as_millis());

                        // 始终追加 IsOffscreen=false
                        condition = with_is_offscreen_false(&n.automation, condition);

                        // Phase 1: FindFirst（默认快路径）
                        let search_start = std::time::Instant::now();
                        let mut tried_findall = false;

                        let mut candidates = match uia_search_first(n, &condition, effective_axis, cache_request.as_ref()) {
                            Some(elem) => {
                                log::debug!("[PERF][Control] step={} node={} FindFirst({:?}): {}ms, 1 result",
                                    step_idx, node_idx, effective_axis, search_start.elapsed().as_millis());
                                vec![elem]
                            }
                            None => {
                                // Phase 2: FindFirst 没找到，尝试 FindAll（如果启用）
                                if ctx.enable_findall {
                                    tried_findall = true;
                                    let all = uia_search_all(n, &condition, effective_axis, cache_request.as_ref());
                                    log::debug!("[PERF][Control] step={} node={} FindFirst=0, FindAll({:?}): {}ms, {} results",
                                        step_idx, node_idx, effective_axis, search_start.elapsed().as_millis(), all.len());
                                    all
                                } else {
                                    log::debug!("[PERF][Control] step={} node={} FindFirst({:?}): {}ms, 0 results (enable_findall=false)",
                                        step_idx, node_idx, effective_axis, search_start.elapsed().as_millis());
                                    Vec::new()
                                }
                            }
                        };

                        // 应用复杂谓词
                        if !analysis.complex_indices.is_empty() {
                            let complex_start = std::time::Instant::now();
                            candidates = uia_condition::apply_complex_predicates(
                                candidates, &step.predicates, &analysis.complex_indices, ctx
                            )?;
                            log::debug!("[PERF][Control] step={} node={} complex_predicates: {}ms, {} after",
                                step_idx, node_idx, complex_start.elapsed().as_millis(), candidates.len());
                        }

                        // 如果复杂谓词过滤后为空，且未尝试过 FindAll，且 enable_findall=true，再试 FindAll
                        if candidates.is_empty() && ctx.enable_findall && !tried_findall {
                            let all = uia_search_all(n, &condition, effective_axis, cache_request.as_ref());
                            log::debug!("[PERF][Control] step={} node={} complex predicates rejected FindFirst, trying FindAll: {} results",
                                step_idx, node_idx, all.len());
                            if !analysis.complex_indices.is_empty() {
                                candidates = uia_condition::apply_complex_predicates(
                                    all, &step.predicates, &analysis.complex_indices, ctx
                                )?;
                            } else {
                                candidates = all;
                            }
                        }

                        // 诊断
                        if candidates.is_empty() && !step.predicates.is_empty() {
                            uia_condition::diagnose_empty_result(n, step.axis, &step.predicates, &analysis);
                        }

                        (candidates, false)
                    }
                    Err(e) => {
                        log::warn!("[Control] Condition build failed: {:?}, falling back to ControlViewWalker", e);
                        let fb_start = std::time::Instant::now();
                        let result = axes::select_axis_strict(n, step.axis)?;
                        log::debug!("[PERF][Control] step={} node={} ControlViewWalker fallback({:?}): {}ms, {} results",
                            step_idx, node_idx, effective_axis, fb_start.elapsed().as_millis(), result.len());
                        (result, false)
                    }
                }
            } else {
                // 非优化路径：使用 ControlViewWalker
                let fb_start = std::time::Instant::now();
                let result = if step.axis == Axis::Attribute {
                    Vec::new()
                } else {
                    axes::select_axis_strict(n, step.axis)?
                };
                log::debug!("[PERF][Control] step={} node={} no_opt ControlViewWalker({:?}): {}ms, {} results",
                    step_idx, node_idx, effective_axis, fb_start.elapsed().as_millis(), result.len());
                (result, false)
            };

            // 节点测试
            let mut after_test: Vec<UiElement> = if should_optimize {
                candidates // UIA Condition 已过滤 ControlType
            } else {
                candidates.into_iter()
                    .filter(|c| node_test_match(c, &step.test, step.axis))
                    .collect()
            };

            // 剩余谓词
            if !predicates_fully_applied && !step.predicates.is_empty() {
                after_test = apply_all_predicates(after_test, &step.predicates, ctx)?;
            }

            // 去重
            let deduped = dedup_into(&next, after_test);
            next.extend(deduped);

            log::debug!("[PERF][Control] step={} node={} total: {}ms", step_idx, node_idx, node_start.elapsed().as_millis());
        }
        nodes = next;
        log::debug!("[PERF][Control] step={} done: {}ms, {} nodes", step_idx, step_start.elapsed().as_millis(), nodes.len());
    }
    log::debug!("[PERF][Control] step_through total: {}ms", total_start.elapsed().as_millis());

    nodes = apply_visibility_filter(nodes, ctx.visibility_filter);
    Ok(nodes)
}

// ═══════════════════════════════════════════════════════════════════
// RawView 实现（strict_control_view=false）
//
// 特点：
// - FindFirst/FindAll 空时回退到 RawViewWalker（Qt 等中间层可能不在 ControlView 中）
// - Walker 回退使用 RawViewWalker
// ═══════════════════════════════════════════════════════════════════

fn step_through_raw(mut nodes: Vec<UiElement>, steps: &[Step], ctx: &Context) -> Result<Vec<UiElement>> {
    let total_start = std::time::Instant::now();
    log::debug!("[XPath RawView] Starting with {} nodes, {} steps", nodes.len(), steps.len());

    if nodes.is_empty() {
        return Ok(Vec::new());
    }

    let skip_step_0 = compute_skip_step_0(steps);
    if skip_step_0 {
        log::debug!("[XPath RawView] ★ Skipping Step 0 (DescendantOrSelf/Node), merging with Step 1");
    }

    for (step_idx, step) in steps.iter().enumerate() {
        let step_start = std::time::Instant::now();
        if skip_step_0 && step_idx == 0 {
            continue;
        }

        let effective_axis = if skip_step_0 && step_idx == 1 {
            Axis::Descendant
        } else {
            step.axis
        };

        let mut next: Vec<UiElement> = Vec::new();

        use crate::xpath::uia_condition;
        let analysis = uia_condition::analyze_predicates(&step.predicates, Some(&step.test));
        let should_optimize = compute_should_optimize(effective_axis, &analysis, &step.test);

        let cache_request = if should_optimize && !nodes.is_empty() {
            crate::element::create_default_cache_request(&nodes[0].automation).ok()
        } else {
            None
        };

        for (node_idx, n) in nodes.iter().enumerate() {
            let node_start = std::time::Instant::now();
            let (candidates, predicates_fully_applied) = if should_optimize {
                let cond_start = std::time::Instant::now();
                match uia_condition::build_condition_from_analysis(
                    &n.automation, &step.predicates, &analysis, Some(&step.test),
                ) {
                    Ok(mut condition) => {
                        log::debug!("[PERF][Raw] step={} node={} build_condition: {}ms",
                            step_idx, node_idx, cond_start.elapsed().as_millis());

                        // 始终追加 IsOffscreen=false
                        condition = with_is_offscreen_false(&n.automation, condition);

                        // Phase 1: FindFirst（默认快路径）
                        let search_start = std::time::Instant::now();
                        let mut tried_findall = false;

                        let mut candidates = match uia_search_first(n, &condition, effective_axis, cache_request.as_ref()) {
                            Some(elem) => {
                                log::debug!("[PERF][Raw] step={} node={} FindFirst({:?}): {}ms, 1 result",
                                    step_idx, node_idx, effective_axis, search_start.elapsed().as_millis());
                                vec![elem]
                            }
                            None => {
                                // Phase 2: FindFirst 没找到，尝试 FindAll（如果启用）
                                if ctx.enable_findall {
                                    tried_findall = true;
                                    let all = uia_search_all(n, &condition, effective_axis, cache_request.as_ref());
                                    log::debug!("[PERF][Raw] step={} node={} FindFirst=0, FindAll({:?}): {}ms, {} results",
                                        step_idx, node_idx, effective_axis, search_start.elapsed().as_millis(), all.len());
                                    all
                                } else {
                                    log::debug!("[PERF][Raw] step={} node={} FindFirst({:?}): {}ms, 0 results (enable_findall=false)",
                                        step_idx, node_idx, effective_axis, search_start.elapsed().as_millis());
                                    Vec::new()
                                }
                            }
                        };

                        // 应用复杂谓词
                        if !analysis.complex_indices.is_empty() {
                            let complex_start = std::time::Instant::now();
                            candidates = uia_condition::apply_complex_predicates(
                                candidates, &step.predicates, &analysis.complex_indices, ctx
                            )?;
                            log::debug!("[PERF][Raw] step={} node={} complex_predicates: {}ms, {} after",
                                step_idx, node_idx, complex_start.elapsed().as_millis(), candidates.len());
                        }

                        // 如果复杂谓词过滤后为空，且未尝试过 FindAll，且 enable_findall=true，再试 FindAll
                        if candidates.is_empty() && ctx.enable_findall && !tried_findall {
                            let all = uia_search_all(n, &condition, effective_axis, cache_request.as_ref());
                            log::debug!("[PERF][Raw] step={} node={} complex predicates rejected FindFirst, trying FindAll: {} results",
                                step_idx, node_idx, all.len());
                            if !analysis.complex_indices.is_empty() {
                                candidates = uia_condition::apply_complex_predicates(
                                    all, &step.predicates, &analysis.complex_indices, ctx
                                )?;
                            } else {
                                candidates = all;
                            }
                        }

                        // ★ RawView 特有：UIA 搜索结果为空时回退到 RawViewWalker
                        // 原因：某些元素（如 Qt 中间层 Group）可能不在 ControlView 中，
                        // 只存在于 RawView。回退到 RawViewWalker 遍历 + Rust 层全谓词求值。
                        if candidates.is_empty() && !step.predicates.is_empty() {
                            let raw_start = std::time::Instant::now();
                            log::debug!("[PERF][Raw] step={} node={} UIA search returned 0, falling back to RawViewWalker", step_idx, node_idx);
                            let raw_candidates = match effective_axis {
                                Axis::Child => n.raw_children().unwrap_or_default(),
                                Axis::Descendant | Axis::DescendantOrSelf => n.raw_descendants().unwrap_or_default(),
                                _ => Vec::new(),
                            };
                            log::debug!("[PERF][Raw] step={} node={} raw_children/descendants: {}ms, {} candidates",
                                step_idx, node_idx, raw_start.elapsed().as_millis(), raw_candidates.len());
                            if !raw_candidates.is_empty() {
                                let after_test: Vec<UiElement> = raw_candidates
                                    .into_iter()
                                    .filter(|c| node_test_match(c, &step.test, effective_axis))
                                    .collect();
                                candidates = apply_all_predicates(after_test, &step.predicates, ctx)?;
                                // predicates_fully_applied = true — 但在此处直接返回最终结果，无需后续谓词处理
                                (candidates, true)
                            } else {
                                (candidates, false)
                            }
                        } else {
                            // 诊断
                            if candidates.is_empty() && !step.predicates.is_empty() {
                                uia_condition::diagnose_empty_result(n, step.axis, &step.predicates, &analysis);
                            }
                            (candidates, false)
                        }
                    }
                    Err(e) => {
                        log::warn!("[Raw] Condition build failed: {:?}, falling back to RawViewWalker", e);
                        let fb_start = std::time::Instant::now();
                        let result = axes::select_axis(n, step.axis)?;
                        log::debug!("[PERF][Raw] step={} node={} RawViewWalker fallback({:?}): {}ms, {} results",
                            step_idx, node_idx, effective_axis, fb_start.elapsed().as_millis(), result.len());
                        (result, false)
                    }
                }
            } else {
                // 非优化路径：使用 RawViewWalker
                let fb_start = std::time::Instant::now();
                let result = if step.axis == Axis::Attribute {
                    Vec::new()
                } else {
                    axes::select_axis(n, step.axis)?
                };
                log::debug!("[PERF][Raw] step={} node={} no_opt RawViewWalker({:?}): {}ms, {} results",
                    step_idx, node_idx, effective_axis, fb_start.elapsed().as_millis(), result.len());
                (result, false)
            };

            // 节点测试
            let mut after_test: Vec<UiElement> = if should_optimize {
                candidates // UIA Condition 已过滤 ControlType
            } else {
                candidates.into_iter()
                    .filter(|c| node_test_match(c, &step.test, step.axis))
                    .collect()
            };

            // 剩余谓词
            if !predicates_fully_applied && !step.predicates.is_empty() {
                after_test = apply_all_predicates(after_test, &step.predicates, ctx)?;
            }

            // 去重
            let deduped = dedup_into(&next, after_test);
            next.extend(deduped);

            log::debug!("[PERF][Raw] step={} node={} total: {}ms", step_idx, node_idx, node_start.elapsed().as_millis());
        }
        nodes = next;
        log::debug!("[PERF][Raw] step={} done: {}ms, {} nodes", step_idx, step_start.elapsed().as_millis(), nodes.len());
    }
    log::debug!("[PERF][Raw] step_through total: {}ms", total_start.elapsed().as_millis());

    nodes = apply_visibility_filter(nodes, ctx.visibility_filter);
    Ok(nodes)
}

// ═══════════════════════════════════════════════════════════════════
// 谓词求值
// ═══════════════════════════════════════════════════════════════════

fn node_test_match(node: &UiElement, test: &NodeTest, _axis: Axis) -> bool {
    match test {
        NodeTest::Wildcard => true,
        NodeTest::Node => true,
        NodeTest::Text | NodeTest::Comment | NodeTest::ProcessingInstruction(_) => false,
        NodeTest::Name(n) => {
            let nn = node.node_name();
            nn.eq_ignore_ascii_case(n) || node.class_name().eq_ignore_ascii_case(n)
        }
    }
}

fn apply_predicate(nodes: &[UiElement], pred: &Expr, ctx: &Context) -> Result<Vec<UiElement>> {
    let size = nodes.len();
    let mut out = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let sub = ctx.with_node(n.clone(), i + 1, size);
        let v = eval_predicate(pred, &sub)?;
        let keep = match v {
            Value::Number(num) => num as i64 == (i as i64 + 1),
            other => other.to_boolean(),
        };
        if keep { out.push(n.clone()); }
    }
    Ok(out)
}

fn apply_all_predicates(
    candidates: Vec<UiElement>,
    predicates: &[Expr],
    ctx: &Context,
) -> Result<Vec<UiElement>> {
    if predicates.is_empty() {
        return Ok(candidates);
    }
    let mut result = candidates;
    for pred in predicates {
        result = apply_predicate(&result, pred, ctx)?;
    }
    Ok(result)
}

fn eval_predicate(expr: &Expr, ctx: &Context) -> Result<Value> {
    eval_with_attrs(expr, ctx)
}

pub fn eval_with_attrs(expr: &Expr, ctx: &Context) -> Result<Value> {
    match expr {
        Expr::Path(p) if p.steps.len() == 1 && p.steps[0].axis == Axis::Attribute && !p.absolute => {
            if let NodeTest::Name(name) = &p.steps[0].test {
                if let Some(v) = ctx.node.get_property(name) {
                    return Ok(Value::String(v));
                } else {
                    return Ok(Value::String(String::new()));
                }
            }
            if let NodeTest::Wildcard = &p.steps[0].test {
                return Ok(Value::String(ctx.node.name()));
            }
        }
        Expr::BinaryOp(op, l, r) => {
            let lv = eval_with_attrs(l, ctx)?;
            let rv = eval_with_attrs(r, ctx)?;
            return apply_binop_value(*op, lv, rv);
        }
        Expr::FunctionCall { name, args } => {
            let mut vs = Vec::with_capacity(args.len());
            for a in args { vs.push(eval_with_attrs(a, ctx)?); }
            return functions::call(name, vs, ctx);
        }
        _ => {}
    }
    eval(expr, ctx)
}

fn apply_binop_value(op: BinOp, l: Value, r: Value) -> Result<Value> {
    Ok(match op {
        BinOp::Or => Value::Boolean(l.to_boolean() || r.to_boolean()),
        BinOp::And => Value::Boolean(l.to_boolean() && r.to_boolean()),
        BinOp::Eq => Value::Boolean(compare_eq(&l, &r)),
        BinOp::NotEq => Value::Boolean(!compare_eq(&l, &r)),
        BinOp::Lt => Value::Boolean(compare_rel(BinOp::Lt, &l, &r)),
        BinOp::Le => Value::Boolean(compare_rel(BinOp::Le, &l, &r)),
        BinOp::Gt => Value::Boolean(compare_rel(BinOp::Gt, &l, &r)),
        BinOp::Ge => Value::Boolean(compare_rel(BinOp::Ge, &l, &r)),
        BinOp::Add => Value::Number(l.to_number() + r.to_number()),
        BinOp::Sub => Value::Number(l.to_number() - r.to_number()),
        BinOp::Mul => Value::Number(l.to_number() * r.to_number()),
        BinOp::Div => Value::Number(l.to_number() / r.to_number()),
        BinOp::Mod => Value::Number(l.to_number() % r.to_number()),
    })
}
