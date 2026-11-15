//! Payload templates bind only to their producing package's frozen release.
use super::{definitions::captured_definitions, pins::WorkspacePins, selection::Selection};
use crate::{ExecError, Repository};
use gripsack_ir::{
    Diagnostic, Ir, Span, codes,
    workspace::WorkspaceDestination,
    workspace_model::{WorkspaceOutput, WorkspaceProducer, WorkspaceSource},
};
use std::{borrow::Cow, collections::BTreeMap};

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
}

fn templated(pattern: &str) -> bool {
    pattern.contains(['{', '}'])
}

fn has_templates(output: &WorkspaceOutput) -> bool {
    match output {
        WorkspaceOutput::Package(package) => package.commands.values().any(|path| templated(path)),
        WorkspaceOutput::Profile(profile) => profile.files.iter().any(|file| match &file.source {
            Some(WorkspaceSource::ArtifactFile { selector, .. }) => templated(selector),
            Some(WorkspaceSource::Tree {
                include, exclude, ..
            }) => include.iter().chain(exclude).any(|path| templated(path)),
            _ => false,
        }),
        _ => false,
    }
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
    if !outputs().any(has_templates) {
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
            _ => {}
        }
    }
    Ok(paths)
}
