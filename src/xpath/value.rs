use crate::element::UiElement;

#[derive(Debug, Clone)]
pub enum Value {
    NodeSet(Vec<UiElement>),
    Number(f64),
    String(String),
    Boolean(bool),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::NodeSet(_) => "node-set",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Boolean(_) => "boolean",
        }
    }

    pub fn to_boolean(&self) -> bool {
        match self {
            Value::Boolean(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::String(s) => !s.is_empty(),
            Value::NodeSet(ns) => !ns.is_empty(),
        }
    }

    pub fn to_number(&self) -> f64 {
        match self {
            Value::Number(n) => *n,
            Value::Boolean(b) => if *b { 1.0 } else { 0.0 },
            Value::String(s) => s.trim().parse().unwrap_or(f64::NAN),
            Value::NodeSet(_) => Value::String(self.to_string_value()).to_number(),
        }
    }

    pub fn to_string_value(&self) -> String {
        match self {
            Value::String(s) => s.clone(),
            Value::Boolean(b) => b.to_string(),
            Value::Number(n) => {
                if n.is_nan() { "NaN".into() }
                else if *n == f64::INFINITY { "Infinity".into() }
                else if *n == f64::NEG_INFINITY { "-Infinity".into() }
                else if n.fract() == 0.0 && n.abs() < 1e21 { format!("{}", *n as i64) }
                else { format!("{}", n) }
            }
            Value::NodeSet(ns) => {
                if let Some(first) = ns.first() { string_value_of_node(first) }
                else { String::new() }
            }
        }
    }
}

pub fn string_value_of_node(e: &UiElement) -> String {
    let n = e.name();
    if !n.is_empty() { n } else { e.help_text() }
}
