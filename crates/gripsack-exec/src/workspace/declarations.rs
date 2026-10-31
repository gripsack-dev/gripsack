//! Borrowed semantic views over current workspace file declarations.
use crate::ExecError;
use gripsack_ir::{Ir, Span, workspace as shared, workspace_model as catalog};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(super) struct Declarations<'a> {
    pub span: &'a Span,
    current: &'a [catalog::WorkspaceOutput],
}
impl<'a> Declarations<'a> {
    pub fn from_ir(ir: &'a Ir) -> Self {
        Self {
            span: &ir.workspace.span,
            current: &ir.workspace.outputs,
        }
    }
    pub fn outputs(&self) -> impl Iterator<Item = Output<'a>> {
        self.current.iter().map(Output)
    }
}
#[derive(Clone, Copy)]
pub(super) struct Output<'a>(&'a catalog::WorkspaceOutput);
impl<'a> Output<'a> {
    pub fn name(self) -> &'a str {
        self.0.name()
    }
    pub fn span(self) -> &'a Span {
        self.0.span()
    }
    pub fn files(self) -> Option<Files<'a>> {
        match self.0 {
            catalog::WorkspaceOutput::Profile(profile) => Some(Files {
                current: &profile.files,
            }),
            _ => None,
        }
    }
    pub fn environment(self) -> Option<&'a str> {
        match self.0 {
            catalog::WorkspaceOutput::Profile(profile) => profile.environment.as_deref(),
            _ => None,
        }
    }
    pub fn hooks(self) -> &'a [String] {
        match self.0 {
            catalog::WorkspaceOutput::Profile(profile) => &profile.hooks,
            _ => &[],
        }
    }
    pub fn admit_profile(self) -> Result<(), ExecError> {
        match self.0 {
            catalog::WorkspaceOutput::Profile(profile) => super::profile::admit(profile),
            _ => Err(super::file_failure(
                self.span(),
                "selected output is not a profile",
            )),
        }
    }
}
pub(super) struct Files<'a> {
    current: &'a [catalog::WorkspaceFile],
}
pub(super) enum Captured<'a> {
    Known(File<'a>),
    Deferred(&'a catalog::WorkspaceFile),
}
impl<'a> Files<'a> {
    pub fn destinations(
        self,
    ) -> impl Iterator<Item = (&'a Span, &'a shared::WorkspaceDestination)> {
        self.current
            .iter()
            // Tree roots are containers, not directory ownership claims.
            // Exact file destinations are checked after bounded expansion.
            .filter(|file| {
                !matches!(
                    file.source,
                    Some(
                        catalog::WorkspaceSource::Tree { .. }
                            | catalog::WorkspaceSource::RepoTree { .. }
                    )
                )
            })
            .map(|file| (&file.span, &file.destination))
    }
    pub fn captured(
        self,
        realization: Option<&super::realize::Realization<'a>>,
        readonly: bool,
    ) -> impl Iterator<Item = Result<Captured<'a>, ExecError>> {
        self.current.iter().map(move |file| {
            let source = match &file.source {
                None => None,
                Some(catalog::WorkspaceSource::RepoFile { path }) => {
                    Some(Source::RepoFile { path: path.clone() })
                }
                Some(catalog::WorkspaceSource::RepoTree {
                    path,
                    include,
                    exclude,
                }) => Some(Source::RepoTree {
                    path: path.clone(),
                    include: include.clone(),
                    exclude: exclude.clone(),
                }),
                Some(catalog::WorkspaceSource::ArtifactFile { output, selector }) => {
                    let Some((path, identity)) =
                        artifact_file(output, selector, realization, &file.span)?
                    else {
                        if readonly {
                            return Ok(Captured::Deferred(file));
                        }
                        return Err(super::file_failure(
                            &file.span,
                            format!("artifact origin {output:?} is not in the realized closure"),
                        ));
                    };
                    Some(Source::ArtifactFile { path, identity })
                }
                Some(catalog::WorkspaceSource::Tree {
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
                            format!("artifact origin {output:?} is not in the realized closure"),
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
                catalog::WorkspaceContent::Identity => Content::Identity,
                catalog::WorkspaceContent::Literal { text } => Content::Literal { text },
                catalog::WorkspaceContent::Template {
                    template,
                    variables,
                    result_digest,
                } => Content::Template {
                    template,
                    variables,
                    result_digest: result_digest.as_deref(),
                },
                catalog::WorkspaceContent::TemplateFile {
                    variables,
                    result_digest,
                } => Content::TemplateFile {
                    variables,
                    result_digest: result_digest.as_deref(),
                },
            };
            Ok(Captured::Known(File {
                span: &file.span,
                source,
                content,
                destination: &file.destination,
                checks: &file.checks,
            }))
        })
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Source {
    RepoFile {
        path: String,
    },
    RepoTree {
        path: String,
        include: Vec<String>,
        exclude: Vec<String>,
    },
    /// Resolved repository-relative target and logical alias attribution.
    RepoTreeFile {
        path: String,
        identity: String,
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
    TemplateFile {
        variables: &'a BTreeMap<String, String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        result_digest: Option<&'a str>,
    },
}
pub(super) struct File<'a> {
    pub span: &'a Span,
    pub source: Option<Source>,
    pub content: Content<'a>,
    pub destination: &'a shared::WorkspaceDestination,
    pub checks: &'a [catalog::WorkspaceFileCheck],
}

/// The realized payload path plus its stable object identity (recipe tree
/// hash or package digest), never a host-dependent store path.
fn artifact_root<'a>(
    output: &str,
    realization: Option<&super::realize::Realization<'a>>,
) -> Option<(PathBuf, String)> {
    let realization = realization?;
    if let Some(package) = realization.packages.get(output) {
        return Some((
            package.payload().to_path_buf(),
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
