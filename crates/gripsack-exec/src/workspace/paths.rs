//! Payload templates bind only to their producing package's frozen release.
use super::{
    artifact::Package, definitions::captured_definitions, pins::WorkspacePins,
    realize::Realization, selection::Selection,
};
use crate::{ExecError, Repository};
use gripsack_ir::{
    Diagnostic, Ir, Span, codes,
    workspace::WorkspaceDestination,
    workspace_model::{
        WorkspaceArg, WorkspaceCatalog, WorkspaceOutput, WorkspacePath, WorkspaceProducer,
        WorkspaceSource,
        graph::{Reference, references},
    },
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) fn expand<'a>(
    pattern: &'a str,
    version: Option<&str>,
    span: &Span,
) -> Result<Cow<'a, str>, ExecError> {
    if !pattern.contains(['{', '}']) {
        return Ok(Cow::Borrowed(pattern));
    }
    gripsack_fetch::placeholders::payload_path(pattern, version)
        .map(Cow::Owned)
        .map_err(|error| {
            ExecError::Gate(
                Diagnostic::error(codes::EXEC_STEP, error.to_string())
                    .with_label(Some(span.clone()), "payload path declared here"),
            )
        })
}

pub(super) fn expand_patterns(
    patterns: &[String],
    version: Option<&str>,
    span: &Span,
) -> Result<Vec<String>, ExecError> {
    patterns
        .iter()
        .map(|pattern| expand(pattern, version, span).map(Cow::into_owned))
        .collect()
}

/// Authored selectors bind to the producing package, never the calling command.
pub(super) fn resolve(
    package: &Package,
    selector: &str,
    span: &Span,
) -> Result<PathBuf, ExecError> {
    let selector = expand(selector, package.source_version.as_deref(), span)?;
    resolve_exact(package.payload(), selector.as_ref(), span)
}

pub(super) fn resolve_artifact(
    output: &str,
    selector: &str,
    realization: &Realization<'_>,
    span: &Span,
) -> Result<PathBuf, ExecError> {
    let package = realization
        .packages
        .get(output)
        .ok_or_else(|| binding_error(span, format!("artifact {output:?} is not realized")))?;
    resolve(package, selector, span)
}

/// Concrete retained selectors have no template semantics. Canonical containment
/// applies equally to files, directories and the whole-artifact selector `.`.
pub(super) fn resolve_exact(
    payload: &Path,
    selector: &str,
    span: &Span,
) -> Result<PathBuf, ExecError> {
    let root = payload
        .canonicalize()
        .map_err(|error| binding_error(span, format!("artifact root does not resolve: {error}")))?;
    let selected = root.join(selector).canonicalize().map_err(|error| {
        binding_error(
            span,
            format!("artifact selector {selector:?} does not resolve: {error}"),
        )
    })?;
    if !selected.starts_with(&root) {
        return Err(binding_error(span, "artifact selector escapes its payload"));
    }
    Ok(selected)
}

fn binding_error(span: &Span, detail: impl Into<String>) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, detail)
            .with_label(Some(span.clone()), "artifact reference declared here"),
    )
}

/// Concrete payload-relative paths, independent of local artifact availability.
#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedPayloadPath<'a> {
    Command {
        output: &'a str,
        command: &'a str,
        path: Cow<'a, str>,
    },
    ArtifactFile {
        profile: &'a str,
        output: &'a str,
        selector: Cow<'a, str>,
        destination: &'a str,
    },
    ArtifactTree {
        profile: &'a str,
        output: &'a str,
        include: Vec<String>,
        exclude: Vec<String>,
        destination: &'a str,
    },
    Artifact {
        consumer: &'a str,
        output: &'a str,
        selector: Cow<'a, str>,
        span: &'a Span,
    },
}

fn templated(pattern: &str) -> bool {
    pattern.contains(['{', '}'])
}

fn has_payload_paths(output: &WorkspaceOutput) -> bool {
    match output {
        WorkspaceOutput::Package(package) => package.commands.values().any(|path| templated(path)),
        WorkspaceOutput::Profile(profile) => profile.files.iter().any(|file| match &file.source {
            Some(WorkspaceSource::ArtifactFile { selector, .. }) => templated(selector),
            Some(WorkspaceSource::Tree {
                include, exclude, ..
            }) => include.iter().chain(exclude).any(|path| templated(path)),
            _ => false,
        }),
        WorkspaceOutput::Check(check) => {
            check
                .run
                .arguments()
                .any(|arg| matches!(arg, WorkspaceArg::Source { .. }))
                || matches!(
                    check.run.working_directory(),
                    Some(WorkspacePath::Source { .. })
                )
        }
        _ => false,
    }
}

struct SelectorReference<'a> {
    consumer: &'a str,
    output: &'a str,
    selector: &'a str,
    span: &'a Span,
}

/// The admitted graph already visits every explicit artifact position, including
/// environment values and cwd. Do not maintain a second command-field walker.
fn selector_references<'a>(
    workspace: &'a WorkspaceCatalog,
    selection: Option<&Selection<'a>>,
) -> Vec<SelectorReference<'a>> {
    let mut selectors = Vec::new();
    let mut collect = |reference: &Reference<'a>| {
        // Profile origins have their existing file/tree-specific report rows.
        if !matches!(reference.from, WorkspaceOutput::Profile(_))
            && let Some(selector) = reference.selector
        {
            selectors.push(SelectorReference {
                consumer: reference.from.name(),
                output: reference.to,
                selector,
                span: reference.at,
            });
        }
    };
    if let Some(selection) = selection {
        for name in &selection.required {
            for reference in selection.references(name) {
                collect(reference);
            }
        }
    } else {
        for reference in references(workspace) {
            collect(&reference);
        }
    }
    selectors
}

/// Preview templates from approved lock evidence only. Never resolve, fetch,
/// inspect a payload, or claim that a selected file exists.
pub fn preview_payload_paths<'a>(
    ir: &'a Ir,
    repository: &Repository,
    selected: &[String],
) -> Result<Vec<ResolvedPayloadPath<'a>>, ExecError> {
    let selection = if selected.is_empty() {
        None
    } else {
        Some(Selection::admit(&ir.workspace, selected)?)
    };
    let outputs = || {
        ir.workspace.outputs.iter().filter(|output| {
            selection
                .as_ref()
                .is_none_or(|selection| selection.required.contains(output.name()))
        })
    };
    let selectors = selector_references(&ir.workspace, selection.as_ref());
    if selectors.is_empty() && !outputs().any(has_payload_paths) {
        return Ok(Vec::new());
    }
    let pins = WorkspacePins::read(repository)?;
    pins.admit_definitions(&captured_definitions(repository)?)?;
    let mut versions = BTreeMap::new();
    for output in outputs() {
        if let WorkspaceOutput::Package(package) = output {
            let version = match &package.producer {
                WorkspaceProducer::Provider { provider } => pins
                    .lookup(&package.target, &package.name, &provider.locked())?
                    .and_then(|pin| pin.resolved.version.as_deref()),
                WorkspaceProducer::Recipe { .. } => None,
            };
            versions.insert(package.name.as_str(), version);
        }
    }
    let mut paths = Vec::new();
    for output in outputs() {
        match output {
            WorkspaceOutput::Package(package) => {
                for (command, pattern) in &package.commands {
                    if templated(pattern) {
                        paths.push(ResolvedPayloadPath::Command {
                            output: &package.name,
                            command,
                            path: expand(pattern, versions[package.name.as_str()], &package.span)?,
                        });
                    }
                }
            }
            WorkspaceOutput::Profile(profile) => {
                for file in &profile.files {
                    let destination = match &file.destination {
                        WorkspaceDestination::Symlink { path }
                        | WorkspaceDestination::TrackedCopy { path }
                        | WorkspaceDestination::ManagedBlock { path, .. } => path,
                    };
                    match &file.source {
                        Some(WorkspaceSource::ArtifactFile { output, selector })
                            if templated(selector) =>
                        {
                            let version = versions.get(output.as_str()).copied().flatten();
                            paths.push(ResolvedPayloadPath::ArtifactFile {
                                profile: &profile.name,
                                output,
                                selector: expand(selector, version, &file.span)?,
                                destination,
                            });
                        }
                        Some(WorkspaceSource::Tree {
                            output,
                            include,
                            exclude,
                        }) if include.iter().chain(exclude).any(|path| templated(path)) => {
                            let version = versions.get(output.as_str()).copied().flatten();
                            paths.push(ResolvedPayloadPath::ArtifactTree {
                                profile: &profile.name,
                                output,
                                include: expand_patterns(include, version, &file.span)?,
                                exclude: expand_patterns(exclude, version, &file.span)?,
                                destination,
                            });
                        }
                        _ => {}
                    }
                }
            }
            WorkspaceOutput::Check(check) => {
                // An attached file subject is not a package namespace. Only a
                // package-subject check can bind release selectors here.
                if let Some(&version) = versions.get(check.subject.as_str()) {
                    let mut collect = |selector: &'a str| -> Result<(), ExecError> {
                        paths.push(ResolvedPayloadPath::Artifact {
                            consumer: &check.name,
                            output: &check.subject,
                            selector: expand(selector, version, check.run.span())?,
                            span: check.run.span(),
                        });
                        Ok(())
                    };
                    for argument in check.run.arguments() {
                        if let WorkspaceArg::Source { selector } = argument {
                            collect(selector)?;
                        }
                    }
                    if let Some(WorkspacePath::Source { selector }) = check.run.working_directory()
                    {
                        collect(selector)?;
                    }
                }
            }
            _ => {}
        }
    }
    for reference in selectors {
        let version = versions.get(reference.output).copied().flatten();
        paths.push(ResolvedPayloadPath::Artifact {
            consumer: reference.consumer,
            output: reference.output,
            selector: expand(reference.selector, version, reference.span)?,
            span: reference.span,
        });
    }
    Ok(paths)
}

#[cfg(all(test, unix))]
mod tests;
