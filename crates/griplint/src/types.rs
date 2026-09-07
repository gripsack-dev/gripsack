//! Public pack vocabulary. Admission is implemented separately so the
//! checker only sees typed rules, never unvalidated marker maps.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Toml,
    Yaml,
    Json,
}

impl Format {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Yaml => "yaml",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    String,
    Integer,
    Boolean,
    Float,
    Array,
    Table,
}

impl ValueType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Float => "float",
            Self::Array => "array",
            Self::Table => "table",
        }
    }
}

/// Tool identity, basename dispatch, format, and version coverage.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "crate::admission::RawMeta")]
pub struct Meta {
    pub tool: String,
    pub handles: Vec<String>,
    pub format: Format,
    pub supported: Vec<String>,
    pub lenient: Vec<String>,
    pub series: String,
    /// Exact W10 text; `{version}` is replaced with the pinned version.
    pub coverage_warning: Option<String>,
}

/// Expected types, optional closed choices, and an upstream rename.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "crate::admission::RawRule")]
pub struct Rule {
    pub types: Vec<ValueType>,
    pub choices: Option<Vec<serde_json::Value>>,
    pub deprecated: Option<String>,
}

impl Rule {
    pub fn type_names(&self) -> String {
        self.types
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join(" or ")
    }
}

/// Only `_free` and `_rule` are reserved section keys. Other underscore
/// keys, including the literal `_`, are ordinary tool configuration keys.
#[derive(Debug, Clone, PartialEq)]
pub enum SectionRules {
    Keys(BTreeMap<String, RuleValue>),
    Free,
    WholeRule(Rule),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuleValue {
    Free,
    Rule(Rule),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "crate::admission::RawPack")]
pub struct Pack {
    pub meta: Meta,
    pub files: BTreeMap<String, FileTable>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "crate::admission::RawFileTable")]
pub struct FileTable {
    pub no_table: bool,
    pub rules: BTreeMap<String, SectionRules>,
}
