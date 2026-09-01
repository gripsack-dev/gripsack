//! The rule walk — a faithful port of griplint-common's checks.py.
//! Codes A00–A05 and the W10 coverage warning keep their exact shapes;
//! message text is byte-identical except A00 parse errors, whose body
//! is the underlying parser's (tomllib/PyYAML/JSON wording differs by
//! implementation; the prefix and span are pinned).

use crate::difflib::suggest;
use crate::document::{Document, SECTION};
use crate::value::Value;
use crate::{FileTable, Pack, RuleValue, SectionRules};
use gripsack_ir::{Diagnostic, Severity};

/// What the walk needs from a section's table entry.
enum SectionKind<'a> {
    Free,
    WholeRule(crate::Rule),
    Keys(&'a std::collections::BTreeMap<String, RuleValue>),
}

fn section_kind(rules: Option<&SectionRules>) -> SectionKind<'_> {
    match rules {
        None => SectionKind::Free, // reached only as a namespace prefix
        Some(SectionRules::Markers(m)) => {
            if m.get("_free").and_then(|v| v.as_bool()) == Some(true) {
                SectionKind::Free
            } else if let Some(rule) = m.get("_rule") {
                match serde_json::from_value::<crate::Rule>(rule.clone()) {
                    Ok(r) => SectionKind::WholeRule(r),
                    Err(_) => SectionKind::Free,
                }
            } else {
                SectionKind::Free
            }
        }
        Some(SectionRules::Keys(keys)) => SectionKind::Keys(keys),
    }
}

fn known_keys(rules: &std::collections::BTreeMap<String, RuleValue>) -> Vec<&str> {
    rules.keys().map(String::as_str).collect()
}

fn check_key(
    doc: &Document,
    _table: &FileTable,
    section: &str,
    key: &str,
    val: &Value,
    rules: &std::collections::BTreeMap<String, RuleValue>,
) -> Vec<Diagnostic> {
    let where_ = if section.is_empty() {
        String::new()
    } else {
        format!("[{section}] ")
    };
    // a TABLE-valued entry is a section header, not a key — blame the
    // [section.key] header line; the (section, key) probe misses those
    // (the scanner records headers flattened), which silently fell back
    // to line 1:1 before
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
    let rule = rules.get(key);
    let Some(rule) = rule else {
        let suggestion = suggest(key, known_keys(rules));
        // a table we have no rules for is an unknown SECTION, not an
        // unknown key — name it that way
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
        RuleValue::Free(_) => return vec![],
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
        let want = rule.types.join(" or ");
        return vec![Diagnostic {
            code: "A04".into(),
            severity: Severity::Error,
            message: format!("{where_}'{key}' must be {want}, got {}", val.type_name()),
            labels: vec![label],
            help: None,
        }];
    }
    if let Some(choices) = &rule.choices {
        let in_choices = choices.iter().any(|c| Value::from(c.clone()).py_eq(val));
        if !in_choices {
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
    }
    vec![]
}

/// The engine pass (checks.py table_check): one traversal emitting
/// A01–A05. `strict=false` (shared files like pyproject.toml) suppresses
/// unknown-key/section errors; known paths still checked.
pub fn table_check(doc: &Document, table: &FileTable, strict: bool) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    fn walk(
        doc: &Document,
        table: &FileTable,
        out: &mut Vec<Diagnostic>,
        section: &str,
        mapping: &[(String, Value)],
        strict: bool,
    ) {
        let rules = section_kind(table.rules.get(section));
        for (key, val) in mapping {
            let sub = if section.is_empty() {
                key.clone()
            } else {
                format!("{section}.{key}")
            };
            if let Value::Table(entries) = val {
                if let Some(entry) = table.rules.get(&sub) {
                    // a tabled section: descend unless FREE_FORM
                    if !matches!(section_kind(Some(entry)), SectionKind::Free) {
                        walk(doc, table, out, &sub, entries, strict);
                    }
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
            match rules {
                SectionKind::Free => continue,
                SectionKind::WholeRule(_) => {
                    // the section type-checks as a whole; members pass
                    continue;
                }
                SectionKind::Keys(keys) => {
                    if strict {
                        out.extend(check_key(doc, table, section, key, val, keys));
                    } else {
                        out.extend(
                            check_key(doc, table, section, key, val, keys)
                                .into_iter()
                                .filter(|d| d.code != "A01"),
                        );
                    }
                }
            }
        }
    }

    for (key, value) in &doc.data {
        // section-shaped top levels: a [key] table, or a [[key]]
        // array-of-tables (parses as Value::Array — invisible to the
        // section dispatch, and so to A02, until now)
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
            if let Some(entry) = table.rules.get(key) {
                match section_kind(Some(entry)) {
                    SectionKind::Free => continue,
                    SectionKind::WholeRule(rule) => {
                        // the whole table must match the rule's types
                        let where_ = format!("[{key}] ");
                        let (span, col, note) = doc.label(SECTION, key, "");
                        if !value.matches(&rule.types) {
                            out.push(Diagnostic {
                                code: "A04".into(),
                                severity: Severity::Error,
                                message: format!(
                                    "{where_}must be {}, got {}",
                                    rule.types.join(" or "),
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
                            });
                        }
                        continue;
                    }
                    SectionKind::Keys(_) => {
                        for entries in &section_tables {
                            walk(doc, table, &mut out, key, entries, strict);
                        }
                        continue;
                    }
                }
            }
            if table
                .rules
                .keys()
                .any(|t| t.starts_with(&format!("{key}.")))
            {
                for entries in &section_tables {
                    walk(doc, table, &mut out, key, entries, strict);
                }
                continue;
            }
            // a tabled/array-valued bare key declared in the "" table
            // is not a section — check it before complaining
            // (profiles = {...}, [[columns]])
            if let Some(SectionRules::Keys(bare)) = table.rules.get("")
                && bare.get(key).is_some()
            {
                out.extend(check_key(doc, table, "", key, value, bare));
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
            if strict {
                out.extend(check_key(doc, table, "", key, value, bare));
            } else {
                out.extend(
                    check_key(doc, table, "", key, value, bare)
                        .into_iter()
                        .filter(|d| d.code != "A01"),
                );
            }
        }
    }
    out
}

/// Dotted-numeric prefix coverage for W10: "0.14" covers 0.14.3 but
/// not 0.140, and "0.1" does not cover 0.14 — a text starts_with got
/// both wrong. Segments compare numerically (missing = 0); a leading
/// v and prerelease suffixes ("1.2.0-beta") are ignored.
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

/// Lint one file against a pack (0012 §move-3): basename dispatch,
/// version coverage warning (W10), parse (A00), then the rule walk.
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
    // W10: the pinned version is outside the table's supported ranges
    if let Some(version) = tool_version
        && !version.is_empty()
        && !pack
            .meta
            .supported
            .iter()
            .any(|p| version_covered(version, p))
    {
        let message = pack
            .meta
            .coverage_warning
            .clone()
            .unwrap_or_else(|| {
                format!(
                    "key tables are written for {} {}; this module pins {{version}} — results may be stale",
                    pack.meta.tool,
                    pack.meta.supported.join(", ")
                )
            })
            .replace("{version}", version);
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
    let parsed = match pack.meta.format.as_str() {
        "yaml" => crate::document::parse_yaml(path, text),
        "json" => crate::document::parse_json(path, text),
        _ => crate::document::parse_toml(path, text),
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
        // the overreach starts_with allowed
        assert!(!version_covered("0.140", "0.14"));
        assert!(!version_covered("0.14.2", "0.1"));
        assert!(!version_covered("0.10", "0.1"));
        // series prefixes, v-spelled pins, prereleases, year majors
        assert!(version_covered("0.14.2", "0."));
        assert!(version_covered("v25.3", "25."));
        assert!(version_covered("25.3", "v25."));
        assert!(version_covered("1.2.0-beta.4", "1.2"));
        assert!(version_covered("2025.3", "2025"));
        assert!(!version_covered("2024.1", "2025"));
        assert!(!version_covered("17.9", "18."));
    }
}
