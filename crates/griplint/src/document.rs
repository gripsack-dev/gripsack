//! A parsed config document: data + spans, the input the rule walk
//! sees. Spans are code-point based (python str semantics), 1-based.
//!
//! The scanners are ports of griplint-common's document.py: TOML by
//! line walk, JSON by a token scanner, YAML by indentation. Arrays and
//! sequence members are not addressable (v1, same as python).

use crate::value::Value;
use std::collections::HashMap;

pub const SECTION: &str = "__section__";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeySpan {
    pub line: usize,
    pub col: usize,
}

pub struct Document {
    pub path: String,
    pub data: Vec<(String, Value)>,
    pub spans: HashMap<(String, String), KeySpan>,
}

impl Document {
    /// The label for (section, key), falling back to line 1 WITHOUT a
    /// column (the reference fallback shape: no col key at all).
    pub fn label(&self, section: &str, key: &str, note: &str) -> (KeySpan, Option<u32>, String) {
        match self.spans.get(&(section.to_string(), key.to_string())) {
            Some(span) => (*span, Some(span.col as u32), note.to_string()),
            None => (KeySpan { line: 1, col: 1 }, None, note.to_string()),
        }
    }
}

// ── TOML ────────────────────────────────────────────────────────────

fn is_key_start(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Port of _scan_spans: `[section]` headers and `key = value` lines;
/// dotted section headers flatten. Comment-stripped for matching, raw
/// for columns.
fn scan_toml_spans(text: &str) -> HashMap<(String, String), KeySpan> {
    let mut spans = HashMap::new();
    let mut section = String::new();
    for (lineno, raw) in text.lines().enumerate() {
        let lineno = lineno + 1;
        let line: &str = if raw.trim_start().starts_with('#') {
            ""
        } else {
            raw.split('#').next().unwrap_or(raw)
        };
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            // [[...]] array-of-tables and [...] sections both land here
            let inner = trimmed
                .trim_start_matches('[')
                .split(']')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !inner.is_empty() {
                section = inner;
                let col = raw.chars().position(|c| c == '[').unwrap_or(0) + 1;
                spans
                    .entry((SECTION.to_string(), section.clone()))
                    .or_insert(KeySpan { line: lineno, col });
            }
            continue;
        }
        // key = value
        let key_len: usize = trimmed.chars().take_while(|c| is_key_start(*c)).count();
        if key_len > 0 {
            let key: String = trimmed.chars().take(key_len).collect();
            let after = trimmed[key.len()..].trim_start();
            if after.starts_with('=') {
                let col = raw
                    .chars()
                    .position(|c| c.to_string() == key)
                    .map(|p| p + 1)
                    .unwrap_or(1);
                spans
                    .entry((section.clone(), key))
                    .or_insert(KeySpan { line: lineno, col });
            }
        }
    }
    spans
}

/// toml_edit parse error → (line, col), code-point columns.
fn toml_error_span(text: &str, span: Option<std::ops::Range<usize>>) -> (usize, usize) {
    let Some(span) = span else { return (1, 1) };
    let mut line = 1;
    let mut col = 1;
    for (i, c) in text.chars().enumerate() {
        if i >= span.start {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn toml_value(v: toml_edit::Item) -> Value {
    match v {
        toml_edit::Item::None => Value::Str(String::new()),
        toml_edit::Item::Value(v) => toml_scalar(v),
        toml_edit::Item::Table(t) => Value::Table(
            t.iter()
                .map(|(k, v)| (k.to_string(), toml_value(v.clone())))
                .collect(),
        ),
        toml_edit::Item::ArrayOfTables(arr) => Value::Array(
            arr.iter()
                .map(|t| {
                    Value::Table(
                        t.iter()
                            .map(|(k, v)| (k.to_string(), toml_value(v.clone())))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

fn toml_scalar(v: toml_edit::Value) -> Value {
    match v {
        toml_edit::Value::String(s) => Value::Str(s.value().to_string()),
        toml_edit::Value::Integer(i) => Value::Int(*i.value()),
        toml_edit::Value::Float(f) => Value::Float(*f.value()),
        toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
        toml_edit::Value::Datetime(d) => Value::Str(d.value().to_string()),
        toml_edit::Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| toml_scalar(v.clone())).collect())
        }
        toml_edit::Value::InlineTable(t) => Value::Table(
            t.iter()
                .map(|(k, v)| (k.to_string(), toml_scalar(v.clone())))
                .collect(),
        ),
    }
}

pub fn parse_toml(path: &str, text: &str) -> Result<Document, (String, usize, usize)> {
    let doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        let (line, col) = toml_error_span(text, e.span());
        (format!("invalid TOML: {e}"), line, col)
    })?;
    let data = doc
        .as_table()
        .iter()
        .map(|(k, v)| (k.to_string(), toml_value(v.clone())))
        .collect();
    Ok(Document {
        path: path.to_string(),
        data,
        spans: scan_toml_spans(text),
    })
}

// ── JSON ────────────────────────────────────────────────────────────

/// Port of _scan_json_spans: a token scanner over strings (with
/// escapes), brackets, and punctuation; object paths dotted like TOML
/// sections; array members not addressable.
fn scan_json_spans(text: &str) -> HashMap<(String, String), KeySpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut line_starts = vec![0usize];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            line_starts.push(i + 1);
        }
    }
    let line_col = |offset: usize| {
        let line = line_starts.partition_point(|s| *s <= offset);
        (line, offset - line_starts[line - 1] + 1)
    };

    let mut spans = HashMap::new();
    let mut path: Vec<String> = Vec::new();
    let mut expect_key: Vec<bool> = Vec::new();
    let mut array_depth = 0usize;
    let mut pending_key: Option<String> = None;

    let mut i = 0;
    while i < chars.len() {
        let tok_start = i;
        let c = chars[i];
        let tok: String = match c {
            '"' => {
                let mut j = i + 1;
                while j < chars.len() {
                    if chars[j] == '\\' {
                        j += 1;
                    } else if chars[j] == '"' {
                        break;
                    }
                    j += 1;
                }
                let s: String = chars[i..=j.min(chars.len() - 1)].iter().collect();
                i = j + 1;
                s
            }
            '{' | '}' | '[' | ']' | ':' | ',' => {
                i += 1;
                c.to_string()
            }
            c if !c.is_whitespace() => {
                i += 1;
                c.to_string()
            }
            _ => {
                i += 1;
                continue;
            }
        };
        match tok.as_str() {
            "[" => {
                array_depth += 1;
                pending_key = None;
            }
            "]" => {
                array_depth = array_depth.saturating_sub(1);
                if array_depth == 0 && !expect_key.is_empty() {
                    let l = expect_key.len();
                    expect_key[l - 1] = true;
                }
            }
            _ if array_depth > 0 => {}
            "{" => {
                if let Some(k) = pending_key.take() {
                    path.push(k);
                }
                expect_key.push(true);
            }
            "}" => {
                expect_key.pop();
                path.pop();
                if !expect_key.is_empty() {
                    let l = expect_key.len();
                    expect_key[l - 1] = true;
                }
            }
            "," => {
                if !expect_key.is_empty() {
                    let l = expect_key.len();
                    expect_key[l - 1] = true;
                }
            }
            _ if tok.starts_with('"')
                && !expect_key.is_empty()
                && expect_key.last() == Some(&true) =>
            {
                let key = serde_json::from_str::<String>(&tok).unwrap_or_default();
                let section = path.join(".");
                let (line, col) = line_col(tok_start);
                spans
                    .entry((section, key.clone()))
                    .or_insert(KeySpan { line, col });
                pending_key = Some(key);
                let l = expect_key.len();
                expect_key[l - 1] = false;
            }
            _ => {}
        }
    }
    spans
}

pub fn parse_json(path: &str, text: &str) -> Result<Document, (String, usize, usize)> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| (format!("invalid JSON: {e}"), e.line(), e.column()))?;
    let serde_json::Value::Object(map) = value else {
        return Err(("top level of a config file must be an object".into(), 1, 1));
    };
    let data = map.into_iter().map(|(k, v)| (k, Value::from(v))).collect();
    Ok(Document {
        path: path.to_string(),
        data,
        spans: scan_json_spans(text),
    })
}

// ── YAML ────────────────────────────────────────────────────────────

/// Port of _scan_yaml_spans as an indentation line-walk (PyYAML's
/// event stream is exact; this handles the map shapes configs are).
/// Sequence members are not keys; a key followed by a deeper-indented
/// key is a section, spanned at the key itself.
fn scan_yaml_spans(text: &str) -> HashMap<(String, String), KeySpan> {
    struct KeyLine {
        line: usize,
        indent: usize,
        col: usize,
        key: String,
        has_value: bool,
    }
    let mut keys = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let indent = raw.len() - raw.trim_start().len();
        let trimmed = raw.trim_start();
        if trimmed.starts_with('#') || trimmed.starts_with('-') || trimmed.is_empty() {
            continue;
        }
        let key_len: usize = trimmed.chars().take_while(|c| is_key_start(*c)).count();
        if key_len == 0 {
            continue;
        }
        let key: String = trimmed.chars().take(key_len).collect();
        let after = trimmed[key.len()..].trim_start();
        if !after.starts_with(':') {
            continue;
        }
        let has_value = !after[1..].trim().is_empty() && !after[1..].trim_start().starts_with('#');
        keys.push(KeyLine {
            line: lineno + 1,
            indent,
            col: indent + 1,
            key,
            has_value,
        });
    }

    let mut spans = HashMap::new();
    let mut stack: Vec<(usize, String)> = Vec::new(); // (indent, key)
    for (idx, kl) in keys.iter().enumerate() {
        while stack.last().is_some_and(|(ind, _)| *ind >= kl.indent) {
            stack.pop();
        }
        let section = stack
            .iter()
            .map(|(_, k)| k.as_str())
            .collect::<Vec<_>>()
            .join(".");
        spans
            .entry((section.clone(), kl.key.clone()))
            .or_insert(KeySpan {
                line: kl.line,
                col: kl.col,
            });
        // a section: no inline value, and the next key sits deeper
        let next_deeper = keys.get(idx + 1).is_some_and(|n| n.indent > kl.indent);
        if !kl.has_value && next_deeper {
            let path = if section.is_empty() {
                kl.key.clone()
            } else {
                format!("{section}.{}", kl.key)
            };
            spans.entry((SECTION.to_string(), path)).or_insert(KeySpan {
                line: kl.line,
                col: kl.col,
            });
            stack.push((kl.indent, kl.key.clone()));
        }
    }
    spans
}

pub fn parse_yaml(path: &str, text: &str) -> Result<Document, (String, usize, usize)> {
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| {
        let (line, col) = e
            .location()
            .map(|l| (l.line(), l.column()))
            .unwrap_or((1, 1));
        (format!("invalid YAML: {e}"), line, col)
    })?;
    let serde_yaml::Value::Mapping(map) = value else {
        return Err(("top level of a config file must be a mapping".into(), 1, 1));
    };
    let data = map
        .into_iter()
        .filter_map(|(k, v)| {
            let key = k
                .as_str()
                .map(str::to_string)
                .or_else(|| k.as_i64().map(|i| i.to_string()))?;
            Some((key, yaml_value(v)))
        })
        .collect();
    Ok(Document {
        path: path.to_string(),
        data,
        spans: scan_yaml_spans(text),
    })
}

fn yaml_value(v: serde_yaml::Value) -> Value {
    match v {
        serde_yaml::Value::Null => Value::Str(String::new()),
        serde_yaml::Value::Bool(b) => Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_yaml::Value::String(s) => Value::Str(s),
        serde_yaml::Value::Sequence(items) => {
            Value::Array(items.into_iter().map(yaml_value).collect())
        }
        serde_yaml::Value::Mapping(map) => Value::Table(
            map.into_iter()
                .filter_map(|(k, v)| {
                    let key = k.as_str().map(str::to_string)?;
                    Some((key, yaml_value(v)))
                })
                .collect(),
        ),
        serde_yaml::Value::Tagged(t) => yaml_value(t.value),
    }
}
