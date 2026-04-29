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
    log::debug!("[XPath step_through] Starting with {} nodes", nodes.len());
    for (step_idx, step) in steps.iter().enumerate() {
        log::debug!("[XPath step_through] Step {}: axis={:?}, test={:?}, predicates={}", step_idx, step.axis, step.test, step.predicates.len());
        let mut next: Vec<UiElement> = Vec::new();
        for n in &nodes {
            let mut candidates = if step.axis == Axis::Attribute {
                Vec::new() // UIA 中属性不是节点
            } else {
                axes::select_axis(n, step.axis)?
            };
            log::debug!("[XPath step_through] Step {}: {} candidates from axis", step_idx, candidates.len());
            // 节点测试
            candidates.retain(|c| node_test_match(c, &step.test, step.axis));
            log::debug!("[XPath step_through] Step {}: {} after node test", step_idx, candidates.len());

            // predicate（按 step 的轴方向考虑 position）
            for pred in &step.predicates {
                candidates = apply_predicate(&candidates, pred, ctx)?;
            }
            log::debug!("[XPath step_through] Step {}: {} after predicates", step_idx, candidates.len());
            for c in candidates {
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

fn node_test_match(node: &UiElement, test: &NodeTest, axis: Axis) -> bool {
    match test {
        NodeTest::Wildcard => axis != Axis::Attribute,
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
        // 处理属性谓词 @attr 和 @attr=...
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

fn eval_predicate(expr: &Expr, ctx: &Context) -> Result<Value> {
    // 特殊处理：@attr 解析 -> 当前节点属性的字符串值（NodeSet 包含 0/1 个伪节点用 String 表达）
    eval_with_attrs(expr, ctx)
}

fn eval_with_attrs(expr: &Expr, ctx: &Context) -> Result<Value> {
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