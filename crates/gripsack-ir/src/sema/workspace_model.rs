//! Current-wire admission. Historical readers retain their own wire meaning;
//! common target/name-index/edge-role policy remains one production kernel.
mod graph;
mod image;
mod values;
use crate::{Diagnostic, Span, codes, workspace_model::*};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn check(workspace: &WorkspaceCatalog, diagnostics: &mut Vec<Diagnostic>) {
    values::check(workspace, diagnostics);
    let mut catalog = BTreeMap::new();
    for output in &workspace.outputs {
        if let WorkspaceOutput::Package(package) = output
            && let Some(policy) = &package.host_runtime
        {
            if package.target.os != crate::workspace::PlatformOs::Linux
                || package.target.abi != Some(crate::workspace::PlatformAbi::Gnu)
            {
                fail(
                    diagnostics,
                    codes::INVALID_WORKSPACE_VALUE,
                    &package.span,
                    "host runtime requirements require an explicit Linux GNU target",
                );
            }
            if let Err(message) = policy.validate() {
                fail(
                    diagnostics,
                    codes::INVALID_WORKSPACE_VALUE,
                    &package.span,
                    message,
                );
            }
        }
        if let Some(previous) = catalog.insert(output.name(), output) {
            diagnostics.push(
                Diagnostic::error(
                    codes::DUPLICATE_WORKSPACE_OUTPUT,
                    format!("workspace output {:?} is declared twice", output.name()),
                )
                .with_label(Some(previous.span().clone()), "first declaration")
                .with_label(Some(output.span().clone()), "conflicting declaration"),
            );
        }
    }
    let mut inputs = BTreeMap::new();
    for input in &workspace.inputs {
        if let Some(previous) = inputs.insert(input.name.as_str(), input) {
            diagnostics.push(
                Diagnostic::error(
                    codes::DUPLICATE_WORKSPACE_OUTPUT,
                    format!("workspace input {:?} is declared twice", input.name),
                )
                .with_label(Some(previous.span.clone()), "first declaration")
                .with_label(Some(input.span.clone()), "conflicting declaration"),
            );
        }
    }
    let mut declared = BTreeMap::new();
    for lock in &workspace.mutation_locks {
        let key = (lock.scope, lock.key.as_str());
        if let Some(previous) = declared.insert(key, lock) {
            diagnostics.push(
                Diagnostic::error(
                    codes::INVALID_WORKSPACE_VALUE,
                    "duplicate mutation-lock declaration",
                )
                .with_label(Some(previous.span.clone()), "first declaration")
                .with_label(Some(lock.span.clone()), "duplicate declaration"),
            );
        }
    }
    let mut used = BTreeSet::new();
    for output in &workspace.outputs {
        if let WorkspaceOutput::Task(task) = output {
            for lock in &task.mutation_locks {
                let key = (lock.scope, lock.key.as_str());
                if !declared.contains_key(&key) {
                    fail(
                        diagnostics,
                        codes::UNKNOWN_WORKSPACE_REF,
                        &lock.span,
                        "mutation lock is not declared in this workspace",
                    );
                }
                used.insert(key);
            }
        }
    }
    for (key, lock) in declared {
        if !used.contains(&key) {
            fail(
                diagnostics,
                codes::INVALID_WORKSPACE_VALUE,
                &lock.span,
                "unreachable mutation-lock declaration",
            );
        }
    }
    if diagnostics.is_empty() {
        graph::check(workspace, &catalog, &inputs, diagnostics);
    }
}
fn fail(out: &mut Vec<Diagnostic>, code: &'static str, span: &Span, message: impl Into<String>) {
    out.push(Diagnostic::error(code, message).with_label(Some(span.clone()), "declared here"));
}
