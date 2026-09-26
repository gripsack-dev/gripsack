//! E111 — two workspace profile files may not declare the same
//! destination (case-insensitively). The module grammar rejects the
//! same race for module entries: two owners for one path deploy twice
//! into one journal key and race each other. Profile files lower to
//! the same owned destinations, so the same rule applies; the check
//! runs before any executor exists to observe the conflict.
//!
//! Exclusion: two managed blocks over one host file with different
//! markers are a legitimate future composition — blocked here until
//! the A1-11 marker/tree grammar makes per-block ownership explicit,
//! matching the module grammar which also folds by destination alone.

use crate::diagnostic::{Diagnostic, codes};
use crate::span::Span;
use crate::workspace::{Workspace, WorkspaceDestination, WorkspaceOutput};
use std::collections::BTreeMap;

pub(super) fn check(workspace: &Workspace, diagnostics: &mut Vec<Diagnostic>) {
    // BTreeMap keyed on the case-folded destination so one collision
    // group renders together; entries keep the file span for labels
    // and are sorted for deterministic "first/also" attribution.
    let mut declarations: BTreeMap<String, Vec<&Span>> = BTreeMap::new();
    for output in &workspace.outputs {
        let WorkspaceOutput::Profile(profile) = output else {
            continue;
        };
        for file in &profile.files {
            let path = match &file.destination {
                WorkspaceDestination::Symlink { path }
                | WorkspaceDestination::TrackedCopy { path }
                | WorkspaceDestination::ManagedBlock { path, .. } => path,
            };
            declarations
                .entry(path.to_lowercase())
                .or_default()
                .push(&file.span);
        }
    }
    for (folded, group) in declarations {
        if group.len() < 2 {
            continue;
        }
        let mut ordered = group;
        ordered.sort_by_key(|span| (span.file.clone(), span.line, span.col));
        let mut diagnostic = Diagnostic::error(
            codes::DUPLICATE_DESTINATION,
            format!(
                "workspace profile files declare destination {:?} {} times \
                 (case-insensitive filesystems treat these as one file)",
                folded,
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
        let block = r#"{
                "span": {"file": "grip.ts", "line": 4},
                "source": {"kind": "repo_file", "path": "cfg/vim.snippet"},
                "content": {"kind": "identity"},
                "destination": {"kind": "managed_block", "path": "~/.vimrc", "marker": "gripsack"}}"#;
        let diagnostics =
            crate::check(&doc(&profile("p", 2, &format!("{symlink},{block}")))).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == codes::DUPLICATE_DESTINATION),
            "one path has one owner regardless of policy until per-block grammar lands"
        );
    }
}
