//! File subjects acquire authority only through an explicit owning-profile attachment.
use super::fail;
use crate::{Diagnostic, codes, workspace_model::*};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn attachments<'a>(
    catalog: &BTreeMap<&'a str, &'a WorkspaceOutput>,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeSet<&'a str> {
    let mut attached = BTreeSet::new();
    for output in catalog.values() {
        let WorkspaceOutput::Profile(profile) = output else {
            continue;
        };
        for file in &profile.files {
            for binding in &file.checks {
                let Some(WorkspaceOutput::Check(check)) = catalog.get(binding.check.as_str())
                else {
                    // The shared graph admission reports missing and wrong-kind references.
                    continue;
                };
                if check.subject != profile.name {
                    fail(
                        diagnostics,
                        codes::BAD_WORKSPACE_CONTEXT,
                        &binding.span,
                        "a profile file check must name its owning profile as the check subject",
                    );
                    continue;
                }
                attached.insert(check.name.as_str());
                if binding.subject == FileCheckSubject::Source && file.source.is_none() {
                    fail(
                        diagnostics,
                        codes::BAD_WORKSPACE_CONTEXT,
                        &binding.span,
                        "a source check requires a captured file source",
                    );
                }
                if binding.subject == FileCheckSubject::Rendered
                    && binding.stage != FileCheckStage::PreFlip
                {
                    fail(
                        diagnostics,
                        codes::BAD_WORKSPACE_CONTEXT,
                        &binding.span,
                        "rendered file validation must run before deployment at pre_flip",
                    );
                }
            }
        }
    }
    for output in catalog.values() {
        let checks = match output {
            WorkspaceOutput::Recipe(recipe) => &recipe.checks,
            WorkspaceOutput::Task(task) => &task.checks,
            _ => continue,
        };
        for check in checks {
            if attached.contains(check.as_str()) {
                fail(
                    diagnostics,
                    codes::BAD_WORKSPACE_CONTEXT,
                    output.span(),
                    "a profile-bound file check cannot serve as a recipe validator or task postcondition",
                );
            }
        }
    }
    attached
}
