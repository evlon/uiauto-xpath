use super::ast::*;
use super::axes;
use super::context::Context;
use super::functions;
use super::value::Value;
use crate::element::UiElement;
use crate::error::{Result, XPathError};

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
            for p in parts {
                match eval(p, ctx)? {
                    Value::NodeSet(ns) => {
                        for n in ns {
                            if !out.iter().any(|x| x.equals(&n)) { out.push(n); }
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
    log::debug!("[XPath step_through] Starting with {} nodes, {} steps", nodes.len(), steps.len());
    
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
        let analysis = uia_condition::analyze_predicates(&step.predicates);
        
        let should_optimize = analysis.can_optimize 
            && matches!(effective_axis, Axis::Child | Axis::Descendant | Axis::DescendantOrSelf)
            && analysis.expected_benefit >= 0.5; // 至少 50% 的谓词可以用 Condition
        
        log::debug!("[XPath step_through] Step {} optimization: can_optimize={}, axis_ok={}, benefit={:.2}, should_optimize={}",
            step_idx, analysis.can_optimize, 
            matches!(effective_axis, Axis::Child | Axis::Descendant | Axis::DescendantOrSelf),
            analysis.expected_benefit, should_optimize);
        
        for n in &nodes {
            let (candidates, predicates_fully_applied) = if should_optimize {
                // ★ 两阶段过滤：先用 UIA Condition 快速筛选，空结果自动回退 raw tree
                match uia_condition::build_condition_from_analysis(
                    &n.automation,
                    &step.predicates,
                    &analysis
                ) {
                    Ok(condition) => {
                        // 阶段 1：UIA 引擎过滤（Control View）
                        let control_view_result = match effective_axis {
                            Axis::Child => {
                                n.find_children_with_condition(&condition)
                                    .unwrap_or_else(|e| {
                                        log::warn!("[XPath step_through] FindAll(Children) failed: {:?}", e);
                                        Vec::new()
                                    })
                            },
                            Axis::Descendant | Axis::DescendantOrSelf => {
                                n.find_descendants_with_condition(&condition)
                                    .unwrap_or_else(|e| {
                                        log::warn!("[XPath step_through] FindAll(Descendants) failed: {:?}", e);
                                        Vec::new()
                                    })
                            },
                            _ => axes::select_axis(n, step.axis)?
                        };

                        // ★ 自动回退：当 Control View 的 FindAll 返回空结果时，
                        // 说明目标元素可能只存在于 Raw View（如 Qt 中间层 Group），
                        // 回退到 RawViewWalker 遍历 + Rust 层全谓词求值
                        let mut predicates_fully_applied = false;
                        let filtered = if control_view_result.is_empty() && !step.predicates.is_empty() {
                            log::debug!("[XPath step_through] FindAll({:?}) returned 0, falling back to raw tree traversal", effective_axis);
                            let raw_candidates = match effective_axis {
                                Axis::Child => n.raw_children().unwrap_or_default(),
                                Axis::Descendant | Axis::DescendantOrSelf => n.raw_descendants().unwrap_or_default(),
                                _ => Vec::new(),
                            };
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
                        (candidates, predicates_fully_applied)
                    },
                    Err(e) => {
                        // Condition 构建失败，回退到 axes::select_axis（RawViewWalker）
                        log::warn!("[XPath step_through] Condition build failed: {:?}, falling back to raw tree", e);
                        (axes::select_axis(n, step.axis)?, false)
                    }
                }
            } else {
                // 不优化的情况：使用 RawViewWalker 遍历（axes::select_axis 内部已用 RawViewWalker）
                if step.axis == Axis::Attribute {
                    (Vec::new(), false)
                } else {
                    (axes::select_axis(n, step.axis)?, false)
                }
            };
            
            log::debug!("[XPath step_through] Step {}: {} candidates from axis", 
                step_idx, candidates.len());
            
            // 节点测试（仅对优化路径需要，非优化路径 axes::select_axis 已返回正确节点）
            let mut after_test: Vec<UiElement> = Vec::new();
            for c in candidates {
                if node_test_match(&c, &step.test, step.axis) {
                    after_test.push(c);
                }
            }
            log::debug!("[XPath step_through] Step {}: {} after node test", step_idx, after_test.len());

            if !predicates_fully_applied && !step.predicates.is_empty() {
                after_test = apply_all_predicates(after_test, &step.predicates, ctx)?;
                log::debug!("[XPath step_through] Step {}: {} after predicate filter", step_idx, after_test.len());
            }

            // 去重
            for c in after_test {
                if !next.iter().any(|x| x.equals(&c)) {
                    next.push(c);
                }
            }
        }
        nodes = next;
        log::debug!("[XPath step_through] Step {}: {} nodes after step", step_idx, nodes.len());
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
