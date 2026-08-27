//! The config value model: one normalized Value for every format
//! (TOML/YAML/JSON), with the type names and repr() text the python
//! reference implementation pinned in the fixture corpus.
//!
//! Two python behaviors are load-bearing and ported exactly:
//! - type names come from _TYPE_NAMES (str→"string", int→"integer",
//!   bool→"boolean", float→"float", list→"array", dict→"table")
//! - messages use python repr(): strings single-quoted, booleans
//!   capitalized (True/False), numbers plain

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<Value>),
    /// Insertion order preserved — fixtures compare diagnostics in
    /// document order (python dict order), so no BTreeMap here.
    Table(Vec<(String, Value)>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Str(_) => "string",
            Value::Int(_) => "integer",
            Value::Bool(_) => "boolean",
            Value::Float(_) => "float",
            Value::Array(_) => "array",
            Value::Table(_) => "table",
        }
    }

    /// python repr(): the text fixtures pin in A05 messages.
    pub fn pyrepr(&self) -> String {
        match self {
            Value::Str(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
            Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            Value::Array(items) => {
                let mut out = String::from("[");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(out, "{}", item.pyrepr());
                }
                out.push(']');
                out
            }
            Value::Table(entries) => {
                let mut out = String::from("{");
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(out, "'{k}': {}", v.pyrepr());
                }
                out.push('}');
                out
            }
        }
    }

    /// The python isinstance check, with its one sharp edge preserved:
    /// bool IS an int in python, but the rule engine rejects bools for
    /// int rules explicitly (checks.py).
    pub fn matches(&self, types: &[String]) -> bool {
        if let Value::Bool(_) = self {
            return types.iter().any(|t| t == "boolean");
        }
        types.iter().any(|t| t == self.type_name())
    }
}

impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::String(s) => Value::Str(s),
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else {
                    Value::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::Array(items) => {
                Value::Array(items.into_iter().map(Value::from).collect())
            }
            serde_json::Value::Object(map) => {
                Value::Table(map.into_iter().map(|(k, v)| (k, Value::from(v))).collect())
            }
            serde_json::Value::Null => Value::Str(String::new()),
        }
    }
}
