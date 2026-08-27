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
    let (span, col, note) = doc.label(section, key, "");
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
        return vec![Diagnostic {
            code: "A01".into(),
            severity: Severity::Error,
            message: format!("unknown key {where_}'{key}'"),
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
        let in_choices = choices.iter().any(|c| {
            let cv = Value::from(c.clone());
            cv == *val
        });
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
        if let Value::Table(entries) = value {
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
                        walk(doc, table, &mut out, key, entries, strict);
                        continue;
                    }
                }
            }
            if table
                .rules
                .keys()
                .any(|t| t.starts_with(&format!("{key}.")))
            {
                walk(doc, table, &mut out, key, entries, strict);
                continue;
            }
            // a dict-valued bare key declared in the "" table is not a
            // section — check it before complaining (profiles = {...})
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
    // W10: the pinned version is outside the table's supported prefixes
    if let Some(version) = tool_version
        && !version.is_empty()
        && !pack
            .meta
            .supported
            .iter()
            .any(|p| version.starts_with(p.as_str()))
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
