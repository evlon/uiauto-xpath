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
            // 应用 predicates
            for p in predicates {
                nodes = apply_predicate(&nodes, p, ctx)?;
            }
            // 然后继续后续 steps
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

fn step_through(mut nodes: Vec<UiElement>, steps: &[Step], ctx: &Context) -> Result<Vec<UiElement>> {
    let step_through_start = std::time::Instant::now();
    log::debug!("[XPath step_through] Starting with {} nodes, {} steps", nodes.len(), steps.len());
    
    // Guard: if nodes is empty, nothing to process
    if nodes.is_empty() {
        log::warn!("[XPath step_through] Empty nodes list, returning empty result");
        return Ok(Vec::new());
    }
    
    // ★ 特殊优化：如果 Step 0 是 DescendantOrSelf + Node (无谓词)，且 Step 1 有谓词
    // 则跳过 Step 0，直接在 Step 1 使用 FindAll(Descendants)
    let skip_step_0 = steps.len() >= 2
        && matches!(steps[0].axis, Axis::DescendantOrSelf)
        && matches!(steps[0].test, NodeTest::Node)
        && steps[0].predicates.is_empty()
        && !steps[1].predicates.is_empty();
    
    if skip_step_0 {
        log::info!("[XPath step_through] ★ Optimization: Skipping Step 0 (DescendantOrSelf/Node), merging with Step 1");
    }
    
    for (step_idx, step) in steps.iter().enumerate() {
        let step_start = std::time::Instant::now();
        // 如果跳过了 Step 0，直接处理 Step 1
        if skip_step_0 && step_idx == 0 {
            log::debug!("[XPath step_through] Step 0: SKIPPED (merged with Step 1)");
            continue;
        }
        
        // 如果是 Step 1 且 Step 0 被跳过，使用 Descendants 轴而非 Child
        let effective_axis = if skip_step_0 && step_idx == 1 {
            log::debug!("[XPath step_through] Step {}: axis={:?} (effective: Descendants), predicates={}", 
                step_idx, step.axis, step.predicates.len());
            Axis::Descendant
        } else {
            log::debug!("[XPath step_through] Step {}: axis={:?}, test={:?}, predicates={}", 
                step_idx, step.axis, step.test, step.predicates.len());
            step.axis
        };
        
        // 输出谓词详情（仅 debug 模式）
        for (i, pred) in step.predicates.iter().enumerate() {
            log::debug!("[XPath step_through]   Predicate {}: {:?}", i, pred);
        }
        
        let mut next: Vec<UiElement> = Vec::new();
        
        // 分析谓词，决定是否使用 FindAll 优化
        use crate::xpath::uia_condition;
        let analysis = uia_condition::analyze_predicates(&step.predicates, Some(&step.test));
        
        let should_optimize = analysis.can_optimize 
            && matches!(effective_axis, Axis::Child | Axis::Descendant | Axis::DescendantOrSelf)
            && analysis.expected_benefit >= 0.5
        // ★ 即使没有谓词（can_optimize=false），如果 NodeTest 是具体类型名，
        // 也走优化路径（用 ControlType Condition 替代 Walker 遍历）
        || (matches!(effective_axis, Axis::Child | Axis::Descendant | Axis::DescendantOrSelf)
            && analysis.expected_benefit >= 0.5
            && matches!(&step.test, NodeTest::Name(_)));
        
        log::debug!("[XPath step_through] Step {} optimization: can_optimize={}, axis_ok={}, benefit={:.2}, should_optimize={}",
            step_idx, analysis.can_optimize, 
            matches!(effective_axis, Axis::Child | Axis::Descendant | Axis::DescendantOrSelf),
            analysis.expected_benefit, should_optimize);
        
        // ★ Create a CacheRequest for BuildCache optimization.
        // When should_optimize is true, we'll use FindAllBuildCache instead of FindAll,
        // which prefetches commonly accessed properties (Name, ControlType, ClassName, etc.)
        // into the UIA cache. This eliminates per-element cross-process COM calls.
        // SAFETY: nodes is guaranteed non-empty by the guard at the top of this function.
        let cache_request = if should_optimize && !nodes.is_empty() {
            crate::element::create_default_cache_request(&nodes[0].automation).ok()
        } else {
            None
        };

        for (node_idx, n) in nodes.iter().enumerate() {
            let node_start = std::time::Instant::now();
            let (candidates, predicates_fully_applied) = if should_optimize {
                // ★ 两阶段过滤：先用 UIA Condition 快速筛选 + BuildCache 预取属性
                let cond_build_start = std::time::Instant::now();
                match uia_condition::build_condition_from_analysis(
                    &n.automation,
                    &step.predicates,
                    &analysis,
                    Some(&step.test),  // ★ 传入 node_test 用于无谓词 step 的 ControlType 优化
                ) {
                    Ok(mut condition) => {
                        log::info!("[PERF][XPATH] step={} node={} build_condition: {}ms", step_idx, node_idx, cond_build_start.elapsed().as_millis());
                        
                        // ★ Fast/Strict 模式：追加 IsOffscreen=false 条件，让 UIA 服务端过滤掉不可见元素，
                        // 大幅减少返回的候选节点数（如 Chrome WebView 的 Group 有 2257 个子节点，
                        // 大部分是 offscreen 的，加上此条件后 FindAllBuildCache 只返回可见的）。
                        if ctx.strict_control_view {
                            // 使用与 uia_condition.rs 相同的布尔 VARIANT 构造方式 (VT_BOOL)
                            let is_offscreen_false = unsafe {
                                use windows::Win32::System::Variant::*;
                                let mut variant = VARIANT::default();
                                let var_ptr = &mut variant as *mut VARIANT;
                                std::ptr::write(var_ptr as *mut VARENUM, VT_BOOL);
                                // bool_val=0 表示 VARIANT_FALSE
                                let bool_ptr = (var_ptr as *mut u8).add(8) as *mut i16;
                                std::ptr::write(bool_ptr, 0i16);
                                n.automation.CreatePropertyCondition(
                                    windows::Win32::UI::Accessibility::UIA_IsOffscreenPropertyId, &variant
                                )
                            };
                            if let Ok(offscreen_cond) = is_offscreen_false {
                                condition = unsafe {
                                    n.automation.CreateAndCondition(&condition, &offscreen_cond)
                                        .unwrap_or(condition)
                                };
                                log::info!("[PERF][XPATH] step={} node={} appended IsOffscreen=false condition (strict mode)", step_idx, node_idx);
                            }
                        }
                        
                        // 阶段 1：UIA 引擎过滤（Control View）+ BuildCache 预取属性
                        let uia_start = std::time::Instant::now();
                        let control_view_result = match (&effective_axis, &cache_request) {
                            (Axis::Child, Some(cr)) => {
                                n.find_children_with_condition_cached(&condition, cr)
                                    .unwrap_or_else(|e| {
                                        log::warn!("[XPath step_through] FindAllBuildCache(Children) failed: {:?}, falling back", e);
                                        n.find_children_with_condition(&condition)
                                            .unwrap_or_default()
                                    })
                            },
                            (Axis::Descendant | Axis::DescendantOrSelf, Some(cr)) => {
                                n.find_descendants_with_condition_cached(&condition, cr)
                                    .unwrap_or_else(|e| {
                                        log::warn!("[XPath step_through] FindAllBuildCache(Descendants) failed: {:?}, falling back", e);
                                        n.find_descendants_with_condition(&condition)
                                            .unwrap_or_default()
                                    })
                            },
                            // Fallback when CacheRequest creation failed
                            (Axis::Child, None) => {
                                n.find_children_with_condition(&condition)
                                    .unwrap_or_else(|e| {
                                        log::warn!("[XPath step_through] FindAll(Children) failed: {:?}", e);
                                        Vec::new()
                                    })
                            },
                            (Axis::Descendant | Axis::DescendantOrSelf, None) => {
                                n.find_descendants_with_condition(&condition)
                                    .unwrap_or_else(|e| {
                                        log::warn!("[XPath step_through] FindAll(Descendants) failed: {:?}", e);
                                        Vec::new()
                                    })
                            },
                            _ => axes::select_axis(n, step.axis)?
                        };
                        let uia_ms = uia_start.elapsed().as_millis();
                        log::info!("[PERF][XPATH] step={} node={} FindAllBuildCache({:?}): {}ms, {} results", step_idx, node_idx, effective_axis, uia_ms, control_view_result.len());
                        
                        // ★ 对比测试：同时测量 children() (ControlViewWalker) 的耗时
                        if effective_axis == Axis::Child && ctx.strict_control_view {
                            let cmp_start = std::time::Instant::now();
                            let cmp_children = n.children().unwrap_or_default();
                            let cmp_ms = cmp_start.elapsed().as_millis();
                            log::info!("[PERF][XPATH] step={} node={} children() comparison: {}ms, {} nodes (vs FindAllBuildCache: {}ms)", 
                                step_idx, node_idx, cmp_ms, cmp_children.len(), uia_ms);
                            
                            // ★ 同时测量 raw_children() 的子节点数
                            let raw_start = std::time::Instant::now();
                            let raw_children = n.raw_children().unwrap_or_default();
                            let raw_ms = raw_start.elapsed().as_millis();
                            log::info!("[PERF][XPATH] step={} node={} raw_children() comparison: {}ms, {} nodes", 
                                step_idx, node_idx, raw_ms, raw_children.len());
                        }

                        // ★ 自动回退：当 Control View 的 FindAll 返回空结果时，
                        // 说明目标元素可能只存在于 Raw View（如 Qt 中间层 Group），
                        // 回退到 RawViewWalker 遍历 + Rust 层全谓词求值
                        //
                        // 严格模式下跳过回退：strict_control_view 时 FindAll 空即空
                        let mut predicates_fully_applied = false;
                        let filtered = if !ctx.strict_control_view
                            && control_view_result.is_empty()
                            && !step.predicates.is_empty()
                        {
                            let raw_start = std::time::Instant::now();
                            log::info!("[PERF][XPATH] step={} node={} FindAll returned 0, falling back to raw tree", step_idx, node_idx);
                            let raw_candidates = match effective_axis {
                                Axis::Child => n.raw_children().unwrap_or_default(),
                                Axis::Descendant | Axis::DescendantOrSelf => n.raw_descendants().unwrap_or_default(),
                                _ => Vec::new(),
                            };
                            log::info!("[PERF][XPATH] step={} node={} raw_children/descendants: {}ms, {} candidates", step_idx, node_idx, raw_start.elapsed().as_millis(), raw_candidates.len());
                            if raw_candidates.is_empty() {
                                control_view_result
                            } else {
                                let after_test: Vec<UiElement> = raw_candidates
                                    .into_iter()
                                    .filter(|c| node_test_match(c, &step.test, effective_axis))
                                    .collect();
                                log::debug!("[XPath step_through] raw tree fallback: {} after node_test", after_test.len());
                                // Apply ALL predicates (simple + complex) via Rust layer
                                predicates_fully_applied = true;
                                apply_all_predicates(after_test, &step.predicates, ctx)?
                            }
                        } else {
                            control_view_result
                        };
                        
                        log::debug!("[XPath step_through] Step {}: {} after UIA filter (complex predicates: {})", 
                            step_idx, filtered.len(), analysis.complex_indices.len());
                        
                        // 诊断：如果结果为空，输出详细信息
                        if filtered.is_empty() && !step.predicates.is_empty() {
                            uia_condition::diagnose_empty_result(
                                n, step.axis, &step.predicates, &analysis
                            );
                        }
                        
                        // 阶段 2：Rust 层应用复杂谓词（仅当 FindAll 有结果时才需要此阶段，
                        // 因为 raw tree 回退已经应用了全部谓词）
                        let complex_start = std::time::Instant::now();
                        let candidates = if predicates_fully_applied {
                            filtered
                        } else if !analysis.complex_indices.is_empty() {
                            uia_condition::apply_complex_predicates(
                                filtered,
                                &step.predicates,
                                &analysis.complex_indices,
                                ctx
                            )?
                        } else {
                            filtered
                        };
                        if !analysis.complex_indices.is_empty() {
                            log::info!("[PERF][XPATH] step={} node={} complex_predicates: {}ms", step_idx, node_idx, complex_start.elapsed().as_millis());
                        }
                        (candidates, predicates_fully_applied)
                    },
                    Err(e) => {
                        // Condition 构建失败，回退到 axes 遍历
                        log::info!("[PERF][XPATH] step={} node={} build_condition failed: {}ms", step_idx, node_idx, cond_build_start.elapsed().as_millis());
                        let fallback_start = std::time::Instant::now();
                        // 严格模式下使用 ControlViewWalker，普通模式使用 RawViewWalker
                        let result = if ctx.strict_control_view {
                            log::warn!("[XPath step_through] Condition build failed: {:?}, falling back to strict control tree", e);
                            (axes::select_axis_strict(n, step.axis)?, false)
                        } else {
                            log::warn!("[XPath step_through] Condition build failed: {:?}, falling back to raw tree", e);
                            (axes::select_axis(n, step.axis)?, false)
                        };
                        log::info!("[PERF][XPATH] step={} node={} axis_fallback({:?}): {}ms, {} results", step_idx, node_idx, effective_axis, fallback_start.elapsed().as_millis(), result.0.len());
                        result
                    }
                }
            } else {
                // 不优化的情况
                // 严格模式下使用 ControlViewWalker，普通模式使用 RawViewWalker
                let fallback_start = std::time::Instant::now();
                let result = if step.axis == Axis::Attribute {
                    (Vec::new(), false)
                } else if ctx.strict_control_view {
                    (axes::select_axis_strict(n, step.axis)?, false)
                } else {
                    (axes::select_axis(n, step.axis)?, false)
                };
                log::info!("[PERF][XPATH] step={} node={} no_opt axis({:?}): {}ms, {} results", step_idx, node_idx, effective_axis, fallback_start.elapsed().as_millis(), result.0.len());
                result
            };
            
            log::debug!("[XPath step_through] Step {}: {} candidates from axis", 
                step_idx, candidates.len());
            
            // 节点测试（仅对非优化路径需要，优化路径已通过 UIA Condition 过滤了 ControlType）
            let mut after_test: Vec<UiElement> = if should_optimize {
                // ★ 优化路径：UIA Condition 已经通过 ControlType 过滤，
                // 跳过 node_test_match 避免冗余的 COM 调用
                candidates
            } else {
                let mut filtered = Vec::new();
                for c in candidates {
                    if node_test_match(&c, &step.test, step.axis) {
                        filtered.push(c);
                    }
                }
                filtered
            };
            log::debug!("[XPath step_through] Step {}: {} after node test", step_idx, after_test.len());

            if !predicates_fully_applied && !step.predicates.is_empty() {
                after_test = apply_all_predicates(after_test, &step.predicates, ctx)?;
                log::debug!("[XPath step_through] Step {}: {} after predicate filter", step_idx, after_test.len());
            }

            // ★ 去重：使用 RuntimeId 的 HashSet 做 O(1) 查重，
            // 替代原来的 O(N^2) equals() 线性扫描。
            // equals() 每次都是跨进程 COM 调用，在 Descendant 搜索结果集大时是主要瓶颈。
            let mut seen_ids: HashSet<Vec<i32>> = HashSet::new();
            // 先收集已存在节点的 RuntimeId
            for existing in &next {
                if let Some(rid) = existing.runtime_id() {
                    seen_ids.insert(rid);
                }
            }
            for c in after_test {
                if let Some(rid) = c.runtime_id() {
                    if seen_ids.insert(rid) {
                        next.push(c);
                    }
                } else {
                    // Fallback: 没有 RuntimeId 的节点用 equals() 比较
                    if !next.iter().any(|x| x.equals(&c)) {
                        next.push(c);
                    }
                }
            }
            log::info!("[PERF][XPATH] step={} node={} total: {}ms", step_idx, node_idx, node_start.elapsed().as_millis());
        }
        nodes = next;
        log::info!("[PERF][XPATH] step={} done: {}ms, {} nodes after step", step_idx, step_start.elapsed().as_millis(), nodes.len());
    }
    log::info!("[PERF][XPATH] step_through total: {}ms", step_through_start.elapsed().as_millis());
    
    // 应用可见性过滤（在所有步骤完成后）
    if ctx.visibility_filter != super::context::VisibilityFilter::All {
        let before_filter = nodes.len();
        nodes.retain(|elem| {
            let is_offscreen = elem.is_offscreen();
            match ctx.visibility_filter {
                super::context::VisibilityFilter::VisibleOnly => !is_offscreen,
                super::context::VisibilityFilter::OffscreenOnly => is_offscreen,
                super::context::VisibilityFilter::All => true,
            }
        });
        log::debug!("[XPath step_through] Visibility filter: {} -> {} nodes", before_filter, nodes.len());
    }
    
    Ok(nodes)
}

fn node_test_match(node: &UiElement, test: &NodeTest, _axis: Axis) -> bool {
    match test {
        NodeTest::Wildcard => true, // All elements in the raw tree are valid
        NodeTest::Node => true,
        NodeTest::Text | NodeTest::Comment | NodeTest::ProcessingInstruction(_) => false,
        NodeTest::Name(n) => {
            // 大小写不敏感匹配 ControlType / 类名
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
        log::debug!("[apply_predicate] node {} {}: class='{}' predicate result={:?}", i, n.node_name(), n.class_name(), v);
        let keep = match v {
            Value::Number(num) => num as i64 == (i as i64 + 1),
            other => other.to_boolean(),
        };
        if keep { out.push(n.clone()); }
    }
    Ok(out)
}

/// Apply ALL predicates (both simple and complex) via Rust-layer evaluation.
/// Used when falling back from UIA Condition (Control View) to raw tree traversal,
/// because raw tree candidates haven't been filtered by UIA Condition.
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

// 导出供 uia_condition 模块使用
pub fn eval_with_attrs(expr: &Expr, ctx: &Context) -> Result<Value> {
    // 对二元运算左右进行属性短路求值
    match expr {
        Expr::Path(p) if p.steps.len() == 1 && p.steps[0].axis == Axis::Attribute && !p.absolute => {
            // @attr -> 字符串
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
