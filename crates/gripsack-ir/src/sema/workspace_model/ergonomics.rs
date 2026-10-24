//! v8 declaration constraints inherit the owning file/package/profile span.
use super::fail;
use crate::sema::workspace::span_value::{admissible_destination, admissible_repo_file};
use crate::{Diagnostic, codes, workspace_model::*};
use std::collections::{BTreeMap, BTreeSet};

fn segment_prefix(value: &str) -> bool {
    admissible_repo_file(value)
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || b"*?[]{}".contains(&byte))
}

pub(super) fn file(file: &WorkspaceFile, out: &mut Vec<Diagnostic>) {
    if let Some(WorkspaceSource::RepoTree {
        path,
        include,
        exclude,
    }) = &file.source
        && (!(path == "." || segment_prefix(path))
            || !include
                .iter()
                .chain(exclude)
                .all(|value| segment_prefix(value)))
    {
        fail(
            out,
            codes::INVALID_WORKSPACE_VALUE,
            &file.span,
            "repository trees require normalized relative paths and segment-prefix selections, not globs",
        );
    }
    if matches!(
        file.source,
        Some(WorkspaceSource::Tree { .. } | WorkspaceSource::RepoTree { .. })
    ) && !matches!(file.content, WorkspaceContent::Identity)
    {
        fail(
            out,
            codes::INVALID_WORKSPACE_VALUE,
            &file.span,
            "tree sources require identity content",
        );
    }
}

pub(super) fn package(
    package: &PackageOutput,
    catalog: &BTreeMap<&str, &WorkspaceOutput>,
    out: &mut Vec<Diagnostic>,
) {
    if package.archive_root.is_none() {
        return;
    }
    let source = match &package.producer {
        WorkspaceProducer::Provider { provider } => Some(provider),
        WorkspaceProducer::Recipe { recipe } => match catalog.get(recipe.as_str()) {
            Some(WorkspaceOutput::Recipe(recipe)) => Some(&recipe.source),
            _ => None, // Reference admission reports a missing/wrong-kind producer.
        },
    };
    if !matches!(package.layout, CatalogPackageLayout::Relocatable)
        || source.is_some_and(|source| !matches!(source, AcquisitionSource::Fetch(_)))
    {
        fail(
            out,
            codes::INVALID_WORKSPACE_VALUE,
            &package.span,
            "archive_root requires a native relocatable package",
        );
    }
}

pub(super) fn profile(
    profile: &ProfileOutput,
    catalog: &BTreeMap<&str, &WorkspaceOutput>,
    out: &mut Vec<Diagnostic>,
) {
    let Some(launchers) = &profile.launchers else {
        return;
    };
    if !admissible_destination(&launchers.directory)
        || launchers.directory.contains('\\')
        || launchers
            .directory
            .bytes()
            .any(|byte| byte.is_ascii_control())
    {
        fail(
            out,
            codes::BAD_DESTINATION,
            &profile.span,
            "launcher directory must be a normalized absolute or ~/ path",
        );
    }
    let mut names = BTreeSet::new();
    if launchers.commands.is_empty()
        || launchers.commands.iter().any(|name| {
            !admissible_repo_file(name)
                || name.contains('/')
                || name.bytes().any(|byte| byte <= b' ' || byte == 127)
                || !names.insert(name.as_str())
        })
    {
        fail(
            out,
            codes::INVALID_WORKSPACE_VALUE,
            &profile.span,
            "launchers require nonempty unique plain command names",
        );
    }
    let Some(environment) = &profile.environment else {
        fail(
            out,
            codes::INVALID_WORKSPACE_VALUE,
            &profile.span,
            "launchers require an explicit profile environment",
        );
        return;
    };
    let Some(WorkspaceOutput::Environment(environment)) = catalog.get(environment.as_str()) else {
        return; // Ordinary environment reference admission supplies its diagnostic.
    };
    for command in &launchers.commands {
        let mut exports = environment.packages.iter().filter(|name| {
            matches!(catalog.get(name.as_str()), Some(WorkspaceOutput::Package(package))
                if package.commands.contains_key(command))
        });
        if exports.next().is_none() || exports.next().is_some() {
            fail(
                out,
                codes::UNKNOWN_WORKSPACE_REF,
                &profile.span,
                format!(
                    "launcher command {command:?} must have exactly one export in the profile environment"
                ),
            );
        }
    }
}
