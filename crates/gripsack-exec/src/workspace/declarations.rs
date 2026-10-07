//! Borrowed semantic views over retained v5 and current v6 file declarations.
//! No wire rewrite, cloned catalog or second deployment implementation.
use crate::ExecError;
use gripsack_ir::{Ir, Span, workspace as v5, workspace_v6 as v6};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(super) struct Declarations<'a> {
    pub span: &'a Span,
    old: &'a [v5::WorkspaceOutput],
    current: &'a [v6::WorkspaceOutput],
}
impl<'a> Declarations<'a> {
    pub fn from_ir(ir: &'a Ir) -> Option<Self> {
        if let Some(workspace) = &ir.workspace_v6 {
            Some(Self {
                span: &workspace.span,
                old: &[],
                current: &workspace.outputs,
            })
        } else {
            ir.workspace.as_ref().map(|workspace| Self {
                span: &workspace.span,
                old: &workspace.outputs,
                current: &[],
            })
        }
    }
    pub fn outputs(&self) -> impl Iterator<Item = Output<'a>> {
        self.old
            .iter()
            .map(Output::Old)
            .chain(self.current.iter().map(Output::Current))
    }
}
#[derive(Clone, Copy)]
pub(super) enum Output<'a> {
    Old(&'a v5::WorkspaceOutput),
    Current(&'a v6::WorkspaceOutput),
}
impl<'a> Output<'a> {
    pub fn name(self) -> &'a str {
        match self {
            Self::Old(output) => output.name(),
            Self::Current(output) => output.name(),
        }
    }
    pub fn span(self) -> &'a Span {
        match self {
            Self::Old(output) => output.span(),
            Self::Current(output) => output.span(),
        }
    }
    pub fn files(self) -> Option<Files<'a>> {
        match self {
            Self::Old(v5::WorkspaceOutput::Profile(profile)) => Some(Files {
                old: &profile.files,
                current: &[],
            }),
            Self::Current(v6::WorkspaceOutput::Profile(profile)) => Some(Files {
                old: &[],
                current: &profile.files,
            }),
            _ => None,
        }
    }
    pub fn environment(self) -> Option<&'a str> {
        match self {
            Self::Old(v5::WorkspaceOutput::Profile(profile)) => profile.environment.as_deref(),
            Self::Current(v6::WorkspaceOutput::Profile(profile)) => profile.environment.as_deref(),
            _ => None,
        }
    }
    pub fn hooks(self) -> &'a [String] {
        match self {
            Self::Current(v6::WorkspaceOutput::Profile(profile)) => &profile.hooks,
            _ => &[],
        }
    }
    pub fn admit_profile(self) -> Result<(), ExecError> {
        match self {
            Self::Current(v6::WorkspaceOutput::Profile(profile)) => super::profile::admit(profile),
            Self::Old(v5::WorkspaceOutput::Profile(_)) => Ok(()),
            _ => Err(super::file_failure(
                self.span(),
                "selected output is not a profile",
            )),
        }
    }
}
pub(super) struct Files<'a> {
    old: &'a [v5::WorkspaceFile],
    current: &'a [v6::WorkspaceFile],
}
pub(super) enum Captured<'a> {
    Known(File<'a>),
    Deferred(&'a v6::WorkspaceFile),
}
impl<'a> Files<'a> {
    pub fn destinations(self) -> impl Iterator<Item = (&'a Span, &'a v5::WorkspaceDestination)> {
        self.old
            .iter()
            .map(|file| (&file.span, &file.destination))
            .chain(
                self.current
                    .iter()
                    // Tree roots are containers, not directory ownership claims.
                    // Exact file destinations are checked after bounded expansion.
                    .filter(|file| !matches!(file.source, Some(v6::WorkspaceSource::Tree { .. })))
                    .map(|file| (&file.span, &file.destination)),
            )
    }
    pub fn captured(
        self,
        realization: Option<&super::realize::Realization<'a>>,
        readonly: bool,
    ) -> impl Iterator<Item = Result<Captured<'a>, ExecError>> {
        self.old
            .iter()
            .map(|file| {
                let source = match &file.source {
                    None => None,
                    Some(v5::WorkspaceSource::RepoFile { path }) => {
                        Some(Source::RepoFile { path: path.clone() })
                    }
                    Some(v5::WorkspaceSource::ArtifactFile { .. }) => {
                        return Err(super::file_failure(
                            &file.span,
                            "artifact file requires package realization",
                        ));
                    }
                };
                let content = match &file.content {
                    v5::WorkspaceContent::Identity => Content::Identity,
                    v5::WorkspaceContent::Literal { text } => Content::Literal { text },
                    v5::WorkspaceContent::Template {
                        template,
                        variables,
                    } => Content::Template {
                        template,
                        variables,
                        result_digest: None,
                    },
                };
                Ok(Captured::Known(File {
                    span: &file.span,
                    source,
                    content,
                    destination: &file.destination,
                }))
            })
            .chain(self.current.iter().map(move |file| {
                if !file.checks.is_empty() {
                    return Err(super::file_failure(
                        &file.span,
                        "file-check execution requires its lifecycle adapter",
                    ));
                }
                let source = match &file.source {
                    None => None,
                    Some(v6::WorkspaceSource::RepoFile { path }) => {
                        Some(Source::RepoFile { path: path.clone() })
                    }
                    Some(v6::WorkspaceSource::ArtifactFile { output, selector }) => {
                        let Some((path, identity)) =
                            artifact_file(output, selector, realization, &file.span)?
                        else {
                            if readonly {
                                return Ok(Captured::Deferred(file));
                            }
                            return Err(super::file_failure(
                                &file.span,
                                format!(
                                    "artifact origin {output:?} is not in the realized closure"
                                ),
                            ));
                        };
                        Some(Source::ArtifactFile { path, identity })
                    }
                    Some(v6::WorkspaceSource::Tree {
                        output,
                        include,
                        exclude,
                    }) => {
                        let Some((root, object)) = artifact_root(output, realization) else {
                            if readonly {
                                return Ok(Captured::Deferred(file));
                            }
                            return Err(super::file_failure(
                                &file.span,
                                format!(
                                    "artifact origin {output:?} is not in the realized closure"
                                ),
                            ));
                        };
                        let identity = serde_json::to_string(&("tree", &object, include, exclude))
                            .map_err(|error| super::file_failure(&file.span, error))?;
                        Some(Source::Tree {
                            root,
                            include: include.clone(),
                            exclude: exclude.clone(),
                            identity,
                        })
                    }
                };
                let content = match &file.content {
                    v6::WorkspaceContent::Identity => Content::Identity,
                    v6::WorkspaceContent::Literal { text } => Content::Literal { text },
                    v6::WorkspaceContent::Template {
                        template,
                        variables,
                        result_digest,
                    } => Content::Template {
                        template,
                        variables,
                        result_digest: result_digest.as_deref(),
                    },
                };
                Ok(Captured::Known(File {
                    span: &file.span,
                    source,
                    content,
                    destination: &file.destination,
                }))
            }))
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Source {
    RepoFile {
        path: String,
    },
    /// A file inside a realized recipe/package payload. `identity` binds the
    /// producing object digest and selector, never the host store path.
    ArtifactFile {
        #[serde(skip)]
        path: PathBuf,
        identity: String,
    },
    /// A subtree of a realized payload selected by include/exclude prefixes.
    Tree {
        #[serde(skip)]
        root: PathBuf,
        include: Vec<String>,
        exclude: Vec<String>,
        identity: String,
    },
}
#[derive(Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Content<'a> {
    Identity,
    Literal {
        text: &'a str,
    },
    Template {
        template: &'a str,
        variables: &'a BTreeMap<String, String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        result_digest: Option<&'a str>,
    },
}
pub(super) struct File<'a> {
    pub span: &'a Span,
    pub source: Option<Source>,
    pub content: Content<'a>,
    pub destination: &'a v5::WorkspaceDestination,
}

/// The realized payload path plus its stable object identity (recipe tree
/// hash or package digest), never a host-dependent store path.
fn artifact_root<'a>(
    output: &str,
    realization: Option<&super::realize::Realization<'a>>,
) -> Option<(PathBuf, String)> {
    let realization = realization?;
    if let Some(recipe) = realization.recipes.get(output) {
        return Some((recipe.payload.clone(), recipe.tree.as_str().to_owned()));
    }
    if let Some(package) = realization.packages.get(output) {
        return Some((
            package.producer.payload.clone(),
            package.identity.to_string(),
        ));
    }
    None
}

fn artifact_file<'a>(
    output: &str,
    selector: &str,
    realization: Option<&super::realize::Realization<'a>>,
    span: &Span,
) -> Result<Option<(PathBuf, String)>, ExecError> {
    let Some((root, object)) = artifact_root(output, realization) else {
        return Ok(None);
    };
    let path = root.join(selector);
    let canonical = path.canonicalize().map_err(|error| {
        super::file_failure(
            span,
            format!("artifact selector {selector:?} does not resolve: {error}"),
        )
    })?;
    let root = root
        .canonicalize()
        .map_err(|error| super::file_failure(span, error))?;
    if !canonical.starts_with(&root) {
        return Err(super::file_failure(
            span,
            "artifact selector escapes its payload",
        ));
    }
    Ok(Some((canonical, format!("{object}:{selector}"))))
}
