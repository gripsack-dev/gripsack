//! Catalog construction and duplicate output-name admission (E125).
//! Output names are the single declaration namespace (0052 §2.1); a
//! collision names every declaration span.

use crate::diagnostic::{Diagnostic, codes};
use crate::span::Span;
use crate::workspace::{Workspace, WorkspaceOutput};
use std::collections::BTreeMap;

/// The admitted catalog: name → output. The first declaration wins for
/// reference resolution so one collision never hides another pass's
/// findings.
pub(super) struct Catalog<'a> {
    pub(super) outputs: BTreeMap<&'a str, &'a WorkspaceOutput>,
}

/// E125 — duplicate names get one diagnostic with every declaration
/// span labeled (0052 §2.1: both collision sources shown).
pub(super) fn check<'a>(
    workspace: &'a Workspace,
    diagnostics: &mut Vec<Diagnostic>,
) -> Catalog<'a> {
    let mut outputs: BTreeMap<&str, &WorkspaceOutput> = BTreeMap::new();
    let mut declarations: BTreeMap<&str, Vec<&Span>> = BTreeMap::new();
    for output in &workspace.outputs {
        declarations
            .entry(output.name())
            .or_default()
            .push(output.span());
        outputs.entry(output.name()).or_insert(output);
    }
    for (name, spans) in declarations {
        if spans.len() > 1 {
            let mut diagnostic = Diagnostic::error(
                codes::DUPLICATE_WORKSPACE_OUTPUT,
                format!(
                    "workspace output `{name}` is declared {} times; output names are the single catalog namespace",
                    spans.len()
                ),
            );
            for (index, span) in spans.iter().enumerate() {
                diagnostic = diagnostic.with_label(
                    Some((*span).clone()),
                    if index == 0 {
                        "first declared here".to_string()
                    } else {
                        "also declared here".to_string()
                    },
                );
            }
            diagnostics.push(diagnostic);
        }
    }
    Catalog { outputs }
}

#[cfg(test)]
mod tests {
    use crate::codes;
    use crate::sema::workspace::testutil::{PACKAGE, doc};

    #[test]
    fn duplicate_output_names_label_both_spans() {
        let second = PACKAGE.replace(r#""line": 3"#, r#""line": 9"#);
        let diagnostics = crate::check(&doc(&format!("{PACKAGE},{second}"))).unwrap_err();
        let dup = diagnostics
            .iter()
            .find(|d| d.code == codes::DUPLICATE_WORKSPACE_OUTPUT)
            .expect("E125 fired");
        let lines: Vec<u32> = dup
            .labels
            .iter()
            .filter_map(|l| l.span.as_ref().map(|s| s.line))
            .collect();
        assert_eq!(lines, vec![3, 9], "both declaration spans labeled");
    }
}
