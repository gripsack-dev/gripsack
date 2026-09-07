//! The rule walk. A00–A05 and W10 retain the reference diagnostic
//! shapes and text, except for underlying parser error wording.

use crate::difflib::suggest;
use crate::document::{Document, SECTION};
use crate::value::Value;
use crate::{FileTable, Format, Pack, Rule, RuleValue, SectionRules};
use gripsack_ir::{Diagnostic, Severity};
use std::collections::BTreeMap;

fn known_keys(rules: &BTreeMap<String, RuleValue>) -> Vec<&str> {
    rules.keys().map(String::as_str).collect()
}

fn check_key(
    doc: &Document,
    section: &str,
    key: &str,
    val: &Value,
    rules: &BTreeMap<String, RuleValue>,
) -> Vec<Diagnostic> {
    let where_ = if section.is_empty() {
        String::new()
    } else {
        format!("[{section}] ")
    };
    // A table-valued key is labelled at its section header, when present.
    let (span, col, note) = if matches!(val, Value::Table(_)) {
        let dotted = if section.is_empty() {
            key.to_string()
        } else {
            format!("{section}.{key}")
        };
        let header = doc.label(SECTION, &dotted, "");
        if header.0.line == 1 && header.0.col == 1 {
            doc.label(section, key, "")
        } else {
            header
        }
    } else {
        doc.label(section, key, "")
    };
    let label = gripsack_ir::Label {
        span: Some(gripsack_ir::Span {
            file: doc.path.clone(),
            line: span.line as u32,
            col,
        }),
        note,
    };
    let Some(rule) = rules.get(key) else {
        let suggestion = suggest(key, known_keys(rules));
        let message = if matches!(val, Value::Table(_)) {
            let dotted = if section.is_empty() {
                key.to_string()
            } else {
                format!("{section}.{key}")
            };
            format!("unknown section [{dotted}]")
        } else {
            format!("unknown key {where_}'{key}'")
        };
        return vec![Diagnostic {
            code: "A01".into(),
            severity: Severity::Error,
            message,
            labels: vec![label],
            help: suggestion.map(|s| format!("did you mean '{s}'?")),
        }];
    };
    let rule = match rule {
        RuleValue::Free => return vec![],
        RuleValue::Rule(r) => r,
    };
    if let Some(replacement) = &rule.deprecated {
        return vec![Diagnostic {
            code: "A03".into(),
            severity: Severity::Warning,
            message: format!("{where_}'{key}' is deprecated"),
            labels: vec![label],
            help: Some(format!("renamed to '{replacement}'")),
        }];
    }
    if !val.matches(&rule.types) {
        let want = rule.type_names();
        return vec![Diagnostic {
            code: "A04".into(),
            severity: Severity::Error,
            message: format!("{where_}'{key}' must be {want}, got {}", val.type_name()),
            labels: vec![label],
            help: None,
        }];
    }
    if let Some(choices) = &rule.choices
        && !choices.iter().any(|c| Value::from(c.clone()).py_eq(val))
    {
        let allowed = choices
            .iter()
            .map(|c| Value::from(c.clone()).pyrepr())
            .collect::<Vec<_>>()
            .join(", ");
        return vec![Diagnostic {
            code: "A05".into(),
            severity: Severity::Error,
            message: format!(
                "{where_}'{key}' must be one of {allowed}, got {}",
                val.pyrepr()
            ),
            labels: vec![label],
            help: None,
        }];
    }
    vec![]
}

/// Whole-section rules check the container, not its members. The same
/// dispatch applies to scalar and nested sections, not just root tables.
fn check_whole(doc: &Document, section: &str, value: &Value, rule: &Rule) -> Vec<Diagnostic> {
    if value.matches(&rule.types) {
        return vec![];
    }
    let (span, col, note) = doc.label(SECTION, section, "");
    vec![Diagnostic {
        code: "A04".into(),
        severity: Severity::Error,
        message: format!(
            "[{section}] must be {}, got {}",
            rule.type_names(),
            value.type_name()
        ),
        labels: vec![gripsack_ir::Label {
            span: Some(gripsack_ir::Span {
                file: doc.path.clone(),
                line: span.line as u32,
                col,
            }),
            note,
        }],
        help: None,
    }]
}

fn checked_key(
    doc: &Document,
    section: &str,
    key: &str,
    value: &Value,
    keys: &BTreeMap<String, RuleValue>,
    strict: bool,
) -> Vec<Diagnostic> {
    check_key(doc, section, key, value, keys)
        .into_iter()
        .filter(|d| strict || d.code != "A01")
        .collect()
}

/// One traversal emitting A01–A05. Shared-file leniency suppresses
/// unknown keys/sections but still checks known paths.
pub fn table_check(doc: &Document, table: &FileTable, strict: bool) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    match table.rules.get("") {
        Some(SectionRules::Free) => return out,
        Some(SectionRules::WholeRule(rule)) => {
            return check_whole(doc, "", &Value::Table(doc.data.clone()), rule);
        }
        _ => {}
    }

    fn walk(
        doc: &Document,
        table: &FileTable,
        out: &mut Vec<Diagnostic>,
        section: &str,
        mapping: &[(String, Value)],
        strict: bool,
    ) {
        let rules = table.rules.get(section);
        for (key, val) in mapping {
            let sub = if section.is_empty() {
                key.clone()
            } else {
                format!("{section}.{key}")
            };
            match table.rules.get(&sub) {
                Some(SectionRules::Free) => continue,
                Some(SectionRules::WholeRule(rule)) => {
                    out.extend(check_whole(doc, &sub, val, rule));
                    continue;
                }
                _ => {}
            }
            if let Value::Table(entries) = val {
                if table.rules.contains_key(&sub) {
                    walk(doc, table, out, &sub, entries, strict);
                    continue;
                }
                if table
                    .rules
                    .keys()
                    .any(|t| t.starts_with(&format!("{sub}.")))
                {
                    walk(doc, table, out, &sub, entries, strict);
                    continue;
                }
            }
            if let Some(SectionRules::Keys(keys)) = rules {
                out.extend(checked_key(doc, section, key, val, keys, strict));
            }
            // Absent sections are namespace prefixes. Free/whole sections
            // are handled before descent and never decoded during a walk.
        }
    }

    for (key, value) in &doc.data {
        match table.rules.get(key) {
            Some(SectionRules::Free) => continue,
            Some(SectionRules::WholeRule(rule)) => {
                out.extend(check_whole(doc, key, value, rule));
                continue;
            }
            _ => {}
        }
        // Arrays of tables are section-shaped too; retain A02 dispatch.
        let section_tables: Vec<&Vec<(String, Value)>> = match value {
            Value::Table(entries) => vec![entries],
            Value::Array(items)
                if !items.is_empty() && items.iter().all(|v| matches!(v, Value::Table(_))) =>
            {
                items
                    .iter()
                    .filter_map(|v| match v {
                        Value::Table(entries) => Some(entries),
                        _ => None,
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        if !section_tables.is_empty() {
            if matches!(table.rules.get(key), Some(SectionRules::Keys(_)))
                || table
                    .rules
                    .keys()
                    .any(|t| t.starts_with(&format!("{key}.")))
            {
                for entries in &section_tables {
                    walk(doc, table, &mut out, key, entries, strict);
                }
                continue;
            }
            // A declared bare table/array key is not an unknown section.
            if let Some(SectionRules::Keys(bare)) = table.rules.get("")
                && bare.contains_key(key)
            {
                out.extend(check_key(doc, "", key, value, bare));
                continue;
            }
            if !strict {
                continue;
            }
            let suggestion = suggest(key, table.rules.keys().map(String::as_str));
            let (span, col, note) = doc.label(SECTION, key, "");
            out.push(Diagnostic {
                code: "A02".into(),
                severity: Severity::Error,
                message: format!("unknown section [{key}]"),
                labels: vec![gripsack_ir::Label {
                    span: Some(gripsack_ir::Span {
                        file: doc.path.clone(),
                        line: span.line as u32,
                        col,
                    }),
                    note,
                }],
                help: suggestion.map(|s| format!("did you mean [{s}]?")),
            });
            continue;
        }
        if let Some(SectionRules::Keys(bare)) = table.rules.get("") {
            out.extend(checked_key(doc, "", key, value, bare, strict));
        }
    }
    out
}

/// Dotted-numeric prefix coverage for W10. Missing segments are zero;
/// leading v and prerelease suffixes are ignored.
fn version_covered(version: &str, prefix: &str) -> bool {
    let segments = |s: &str| -> Vec<i64> {
        s.trim_start_matches(['v', 'V'])
            .split('.')
            .filter(|seg| !seg.is_empty())
            .map(|seg| {
                seg.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    let version = segments(version);
    segments(prefix)
        .iter()
        .enumerate()
        .all(|(i, n)| version.get(i).copied().unwrap_or(0) == *n)
}

/// Basename dispatch, version coverage, format parsing, and rule walk.
pub fn lint_file(
    pack: &Pack,
    path: &str,
    text: &str,
    tool_version: Option<&str>,
) -> Vec<Diagnostic> {
    let basename = path.rsplit('/').next().unwrap_or(path);
    if !pack.meta.handles.iter().any(|h| h == basename) {
        return vec![];
    }
    let Some(file) = pack.files.get(basename) else {
        return vec![];
    };
    let mut out = Vec::new();
    if let Some(version) = tool_version
        && !version.is_empty()
        && !pack
            .meta
            .supported
            .iter()
            .any(|p| version_covered(version, p))
    {
        let message = pack.meta.coverage_warning.clone().unwrap_or_else(|| {
            format!(
                "key tables are written for {} {}; this module pins {{version}} — results may be stale",
                pack.meta.tool, pack.meta.supported.join(", ")
            )
        }).replace("{version}", version);
        out.push(Diagnostic {
            code: "W10".into(),
            severity: Severity::Warning,
            message,
            labels: vec![],
            help: None,
        });
    }
    if file.no_table {
        return out;
    }
    let parsed = match pack.meta.format {
        Format::Yaml => crate::document::parse_yaml(path, text),
        Format::Json => crate::document::parse_json(path, text),
        Format::Toml => crate::document::parse_toml(path, text),
    };
    let doc = match parsed {
        Ok(doc) => doc,
        Err((message, line, col)) => {
            out.push(Diagnostic {
                code: "A00".into(),
                severity: Severity::Error,
                message,
                labels: vec![gripsack_ir::Label {
                    span: Some(gripsack_ir::Span {
                        file: path.to_string(),
                        line: line as u32,
                        col: Some(col as u32),
                    }),
                    note: "parse stops here".into(),
                }],
                help: None,
            });
            return out;
        }
    };
    let strict = !pack.meta.lenient.iter().any(|b| b == basename);
    out.extend(table_check(&doc, file, strict));
    out
}

#[cfg(test)]
mod tests {
    use super::version_covered;

    #[test]
    fn version_coverage_is_numeric_not_textual() {
        assert!(version_covered("0.14", "0.14"));
        assert!(version_covered("0.14.3", "0.14"));
        assert!(version_covered("0.1.7", "0.1"));
        assert!(!version_covered("0.140", "0.14"));
        assert!(!version_covered("0.14.2", "0.1"));
        assert!(!version_covered("0.10", "0.1"));
        assert!(version_covered("0.14.2", "0."));
        assert!(version_covered("v25.3", "25."));
        assert!(version_covered("25.3", "v25."));
        assert!(version_covered("1.2.0-beta.4", "1.2"));
        assert!(version_covered("2025.3", "2025"));
        assert!(!version_covered("2024.1", "2025"));
        assert!(!version_covered("17.9", "18."));
    }
}
