//! Normalized configuration values and the Python-shaped type/repr text
//! pinned by the diagnostic corpus. Booleans are not integer-rule values.

use crate::ValueType;
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<Value>),
    /// Preserve document order for diagnostic traversal.
    Table(Vec<(String, Value)>),
    /// JSON/YAML null is not admitted as a rule type, but can occur in input.
    Null,
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
            Value::Null => "null",
        }
    }

    /// Python repr(): the text fixtures pin in A05 messages.
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
            Value::Null => "None".to_string(),
        }
    }

    /// The reference explicitly rejects Python's bool-is-an-int edge.
    pub fn matches(&self, types: &[ValueType]) -> bool {
        let actual = match self {
            Self::Str(_) => ValueType::String,
            Self::Int(_) => ValueType::Integer,
            Self::Float(_) => ValueType::Float,
            Self::Bool(_) => ValueType::Boolean,
            Self::Array(_) => ValueType::Array,
            Self::Table(_) => ValueType::Table,
            Self::Null => return false,
        };
        types.contains(&actual)
    }

    /// Numeric choices compare across int/float, recursively. Derived
    /// PartialEq remains shape-exact; bool never equals a number here.
    pub fn py_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => *a as f64 == *b,
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.py_eq(y))
            }
            (Value::Table(a), Value::Table(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, v)| {
                        b.iter()
                            .find(|(bk, _)| bk == k)
                            .is_some_and(|(_, bv)| v.py_eq(bv))
                    })
            }
            _ => self == other,
        }
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
            serde_json::Value::Null => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_matching_keeps_null_and_boolean_distinct() {
        assert_eq!(Value::Null.type_name(), "null");
        assert_eq!(Value::Null.pyrepr(), "None");
        let all = [
            ValueType::String,
            ValueType::Integer,
            ValueType::Boolean,
            ValueType::Float,
            ValueType::Array,
            ValueType::Table,
        ];
        assert!(!Value::Null.matches(&all));
        assert!(!Value::Bool(true).matches(&[ValueType::Integer]));
        assert!(Value::Bool(true).matches(&[ValueType::Boolean]));
        assert!(Value::Int(1).matches(&[ValueType::String, ValueType::Integer]));
        assert!(!Value::Int(1).matches(&[ValueType::Float]));
    }

    #[test]
    fn numbers_compare_across_int_and_float() {
        assert!(Value::Int(1).py_eq(&Value::Float(1.0)));
        assert!(!Value::Int(1).py_eq(&Value::Float(1.5)));
        assert_ne!(Value::Int(1), Value::Float(1.0));
        assert!(Value::Array(vec![Value::Int(1)]).py_eq(&Value::Array(vec![Value::Float(1.0)])));
        assert!(Value::Null.py_eq(&Value::Null));
        assert!(!Value::Bool(true).py_eq(&Value::Int(1)));
    }
}
