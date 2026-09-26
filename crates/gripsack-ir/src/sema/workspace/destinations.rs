//! E111 — two workspace profile files may not declare the same
//! destination (case-insensitively). The module grammar rejects the
//! same race for module entries: two owners for one path deploy twice
//! into one journal key and race each other. Profile files lower to
//! the same owned destinations, so the same rule applies; the check
//! runs before any executor exists to observe the conflict.
//!
//! Managed blocks are the one composition: blocks are per-marker
//! ownership units inside one shared host file (Epic A §3.3), so two
//! blocks over one path with **different** markers coexist. The same
//! marker twice, or any block mixing with a whole-file
//! symlink/tracked-copy policy over that path, is still one owner too
//! many.

use crate::diagnostic::{Diagnostic, codes};
use crate::span::Span;
use crate::workspace::{Workspace, WorkspaceDestination, WorkspaceOutput};
use std::collections::BTreeMap;

pub(super) fn check(workspace: &Workspace, diagnostics: &mut Vec<Diagnostic>) {
    // Group by case-folded path first: any whole-file policy over a
    // path makes every declaration on that path one owner too many.
    // Blocks then subgroup by case-folded marker — distinct markers
    // are distinct per-block owners inside the shared host file.
    let mut by_path: BTreeMap<String, Vec<(&Span, Option<&str>)>> = BTreeMap::new();
    for output in &workspace.outputs {
        let WorkspaceOutput::Profile(profile) = output else {
            continue;
        };
        for file in &profile.files {
            let (path, marker) = match &file.destination {
                WorkspaceDestination::Symlink { path }
                | WorkspaceDestination::TrackedCopy { path } => (path, None),
                WorkspaceDestination::ManagedBlock { path, marker } => {
                    (path, Some(marker.as_str()))
                }
            };
            by_path
                .entry(path.to_lowercase())
                .or_default()
                .push((&file.span, marker));
        }
    }
    for (folded, declarations) in by_path {
        if declarations.len() < 2 {
            continue;
        }
        if declarations.iter().any(|(_, marker)| marker.is_none()) {
            let spans: Vec<&Span> = declarations.iter().map(|(span, _)| *span).collect();
            reject(&folded, None, &spans, diagnostics);
            continue;
        }
        let mut by_marker: BTreeMap<String, Vec<&Span>> = BTreeMap::new();
        for (span, marker) in declarations {
            by_marker
                .entry(marker.unwrap_or_default().to_lowercase())
                .or_default()
                .push(span);
        }
        for (marker, group) in &by_marker {
            eprintln!("KEYTRACE {marker:?} n={}", group.len());
        }
        for (marker, group) in by_marker {
            if group.len() > 1 {
                reject(&folded, Some(&marker), &group, diagnostics);
            }
        }
    }
}

fn reject(folded: &str, marker: Option<&str>, spans: &[&Span], diagnostics: &mut Vec<Diagnostic>) {
    let mut ordered = spans.to_vec();
    ordered.sort_by_key(|span| (span.file.clone(), span.line, span.col));
    let what = match marker {
        Some(marker) => format!("managed block {marker:?} at destination {folded:?}"),
        None => format!("destination {folded:?}"),
    };
    let mut diagnostic = Diagnostic::error(
        codes::DUPLICATE_DESTINATION,
        format!(
            "workspace profile files declare {what} {} times \
             (case-insensitive filesystems treat these as one file)",
            ordered.len()
        ),
    );
    for (index, span) in ordered.iter().enumerate() {
        diagnostic = diagnostic.with_label(
            Some((*span).clone()),
            if index == 0 {
                "first declared here".to_string()
            } else {
                "also declared here".to_string()
            },
        );
    }
    diagnostics.push(diagnostic.with_help("split the destination, or drop one declaration"));
}

#[cfg(test)]
mod tests {
    use crate::codes;
    use crate::sema::workspace::testutil::doc;

    fn profile(name: &str, line: u32, files: &str) -> String {
        format!(
            r#"{{
            "kind": "profile", "name": "{name}", "span": {{"file": "grip.ts", "line": {line}}},
            "files": [{files}]}}"#
        )
    }

    fn literal_file(line: u32, path: &str) -> String {
        format!(
            r#"{{
                "span": {{"file": "grip.ts", "line": {line}}},
                "source": {{"kind": "repo_file", "path": "cfg/a.conf"}},
                "content": {{"kind": "identity"}},
                "destination": {{"kind": "tracked_copy", "path": "{path}"}}}}"#
        )
    }

    fn block(line: u32, path: &str, marker: &str) -> String {
        format!(
            r#"{{
                "span": {{"file": "grip.ts", "line": {line}}},
                "source": {{"kind": "repo_file", "path": "cfg/snippet"}},
                "content": {{"kind": "identity"}},
                "destination": {{"kind": "managed_block", "path": "{path}", "marker": "{marker}"}}}}"#
        )
    }

    #[test]
    fn duplicate_profile_destinations_label_every_declaration() {
        let files = format!(
            "{},{}",
            literal_file(3, "~/.config/a.conf"),
            literal_file(4, "~/.config/A.CONF")
        );
        let diagnostics = crate::check(&doc(&profile("p", 2, &files))).unwrap_err();
        let duplicate = diagnostics
            .iter()
            .find(|d| d.code == codes::DUPLICATE_DESTINATION)
            .expect("case-variant destinations must fold and reject");
        let lines: Vec<u32> = duplicate
            .labels
            .iter()
            .filter_map(|l| l.span.as_ref().map(|s| s.line))
            .collect();
        assert_eq!(lines, vec![3, 4], "both file declarations labeled");
    }

    #[test]
    fn same_destination_across_profiles_rejects_and_equal_names_coexist() {
        let first = profile("work", 2, &literal_file(3, "/etc/tool/settings.conf"));
        let second = profile("home", 6, &literal_file(7, "/etc/tool/settings.conf"));
        let diagnostics = crate::check(&doc(&format!("{first},{second}"))).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == codes::DUPLICATE_DESTINATION),
            "ownership crosses profile boundaries"
        );

        // Equal relative filenames with different origins (and distinct
        // destinations) are distinct owned files and coexist.
        let coexist = format!(
            "{},{}",
            literal_file(3, "~/.config/a.conf"),
            literal_file(4, "~/.config/sub/a.conf")
        );
        crate::check(&doc(&profile("p", 2, &coexist))).unwrap();
    }

    #[test]
    fn mixed_policies_over_one_path_still_collide() {
        let symlink = literal_file(3, "~/.vimrc");
        let block = block(4, "~/.vimrc", "gripsack");
        let diagnostics =
            crate::check(&doc(&profile("p", 2, &format!("{symlink},{block}")))).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == codes::DUPLICATE_DESTINATION),
            "a whole-file policy and a managed block over one path are still one owner too many"
        );
    }

    #[test]
    fn distinct_markers_over_one_host_file_coexist_but_one_marker_is_one_owner() {
        // Failing-before: two blocks over one path were always E111.
        let coexist = format!(
            "{},{}",
            block(3, "~/.shellrc", "gripsack:tools"),
            block(4, "~/.shellrc", "gripsack:editor")
        );
        crate::check(&doc(&profile("p", 2, &coexist))).unwrap();

        // The same marker twice — case-variant too — is one owner declared twice.
        let duplicate = format!(
            "{},{}",
            block(3, "~/.shellrc", "gripsack:tools"),
            block(7, "~/.SHELLRC", "GRIPSACK:TOOLS")
        );
        let diagnostics = crate::check(&doc(&profile("p", 2, &duplicate))).unwrap_err();
        let rejection = diagnostics
            .iter()
            .find(|d| d.code == codes::DUPLICATE_DESTINATION)
            .expect("duplicate marker over one host path must reject");
        assert!(rejection.message.contains("managed block"));
        let lines: Vec<u32> = rejection
            .labels
            .iter()
            .filter_map(|l| l.span.as_ref().map(|s| s.line))
            .collect();
        assert_eq!(lines, vec![3, 7], "both block declarations labeled");
    }
}
