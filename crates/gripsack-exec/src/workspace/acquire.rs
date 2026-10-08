//! Native acquisition before any production solve. Repository file sources
//! resolve through the approved captured bundle; external acquisition consumes
//! a concrete lock unless the caller owns an explicit update operation.
use crate::{
    Ctx, ExecError, LifecycleSession,
    lockfile::{LockEntry, Resolved},
};
use gripsack_fetch::FetchIdentity;
use gripsack_ir::{
    Diagnostic, FetchSpec, codes,
    workspace::{RecipeOutputKind, WorkspaceFetch},
    workspace_v6::lock::{LockedPin, ResolvedPinFields},
};
use gripsack_store as store;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

pub(super) enum ResolutionMode<'a> {
    Update,
    Frozen(Option<&'a LockedPin>),
}
pub(super) struct AcquiredSource {
    pub path: PathBuf,
    pub tree: store::hash::PayloadHash,
    pub resolved: ResolvedPinFields,
}

/// Survey owns private bytes but has no authority to publish them.
pub(super) struct PreparedSource {
    storage: SourceStorage,
    tree: store::hash::PayloadHash,
    pub resolved: ResolvedPinFields,
}
enum SourceStorage {
    Retained(PathBuf),
    Staged {
        _capture: tempfile::TempDir,
        payload: PathBuf,
    },
}
impl PreparedSource {
    pub fn publish(
        self,
        ctx: &Ctx,
        session: &LifecycleSession,
        name: &str,
    ) -> Result<AcquiredSource, ExecError> {
        if session.home() != ctx.home {
            return Err(ExecError::Step {
                module: name.into(),
                step: "source".into(),
                detail: "source publication belongs to another home".into(),
            });
        }
        let path = match self.storage {
            SourceStorage::Retained(path) => path,
            SourceStorage::Staged { _capture, payload } => {
                let path = store::content_path(&ctx.home, "workspace-source", self.tree.as_str());
                match std::fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_dir() => {
                        let retained = super::stage::validate_output_tree(
                            &path,
                            RecipeOutputKind::Tree,
                            name,
                            ctx.fetch.limits(),
                        )?;
                        if retained.tree_hash() != &self.tree {
                            return Err(ExecError::Step {
                                module: name.into(),
                                step: "source".into(),
                                detail:
                                    "retained source differs; immutable bytes will not be replaced"
                                        .into(),
                            });
                        }
                    }
                    Ok(_) => {
                        return Err(std::io::Error::other(
                            "retained source is not a real directory",
                        )
                        .into());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        crate::source::publish(ctx, name, &payload, &path)?
                    }
                    Err(error) => return Err(error.into()),
                }
                path
            }
        };
        Ok(AcquiredSource {
            path,
            tree: self.tree,
            resolved: self.resolved,
        })
    }
}

pub(super) fn acquire(
    ctx: &Ctx,
    name: &str,
    source: &WorkspaceFetch,
    mode: ResolutionMode<'_>,
) -> Result<PreparedSource, ExecError> {
    let failure = |detail: String| {
        ExecError::Gate(
            Diagnostic::error(codes::EXEC_STEP, detail)
                .with_label(Some(source.span.clone()), "source declared here"),
        )
    };
    if matches!(
        source.fetch,
        FetchSpec::Brew { .. } | FetchSpec::Pixi { .. }
    ) {
        return Err(ExecError::Gate(Diagnostic::error(codes::WORKSPACE_EXEC_UNAVAILABLE,
            "coherent workspace bottle/Conda production belongs to A4/A3; a legacy single-package transport cannot substitute for it")
            .with_label(Some(source.span.clone()), "provider declared here")));
    }
    let captured = repository_source(ctx, &source.fetch)?;
    let (spec, local) = match captured {
        Some(path) => {
            let path = path
                .into_os_string()
                .into_string()
                .map_err(|_| failure("captured source path is not UTF-8".into()))?;
            (Cow::Owned(FetchSpec::File { path }), true)
        }
        None => (Cow::Borrowed(&source.fetch), false),
    };
    let locked = match mode {
        ResolutionMode::Update => None,
        ResolutionMode::Frozen(None) if local => None,
        ResolutionMode::Frozen(None) => {
            return Err(failure(
                "external source has no lock; run grip update before frozen acquisition".into(),
            ));
        }
        ResolutionMode::Frozen(Some(pin)) => {
            let gripsack_ir::workspace_v6::LockedSource::Fetch { fetch: locked } = &pin.source
            else {
                return Err(failure(
                    "fetch source carries a non-fetch frozen pin".into(),
                ));
            };
            if *locked != source.fetch {
                return Err(failure(
                    "source declaration differs from its frozen pin".into(),
                ));
            }
            frozen_source(locked, &pin.resolved).map_err(failure)?;
            Some(pin)
        }
    };
    if let Some(pin) = locked.filter(|_| !local)
        && let Some(tree) = &pin.resolved.tree256
    {
        let path = store::content_path(&ctx.home, "workspace-source", tree);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                let checked = super::stage::validate_output_tree(
                    &path,
                    RecipeOutputKind::Tree,
                    name,
                    ctx.fetch.limits(),
                )?;
                if checked.tree_hash().as_str() != tree {
                    return Err(failure(
                        "retained source differs from its frozen tree identity".into(),
                    ));
                }
                return Ok(PreparedSource {
                    storage: SourceStorage::Retained(path),
                    tree: checked.tree_hash().clone(),
                    resolved: pin.resolved.clone(),
                });
            }
            Ok(_) => return Err(failure("retained source is not a real directory".into())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let resolver_pin = locked.map(|pin| LockEntry {
        fetch: spec.as_ref().clone(),
        resolved: Some(Resolved {
            url: pin.resolved.url.clone(),
            version: pin.resolved.version.clone(),
            sha256: pin.resolved.sha256.clone(),
            tree256: pin.resolved.tree256.clone(),
            repo256: pin.resolved.repo256.clone(),
            api_url: pin.resolved.api_url.clone(),
        }),
    });
    let (concrete, metadata) =
        crate::resolve::resolve_spec(name, spec.as_ref(), resolver_pin.as_ref(), &ctx.fetch)?;
    let locked_json = locked
        .map(|pin| serde_json::to_value(&pin.resolved))
        .transpose()?;
    let temporary = tempfile::Builder::new()
        .prefix("grip-workspace-source-")
        .tempdir()?;
    let payload = temporary.path().join("payload");
    let outcome = ctx
        .fetch
        .fetch(&concrete, &payload, locked_json.as_ref())
        .map_err(|error| {
            ExecError::Fetch(match &source.fetch {
                FetchSpec::GithubRelease { base_url, .. } => {
                    error.with_github_context(base_url.as_deref())
                }
                _ => error,
            })
        })?;
    // FetchContext already applied the shared payload validator. Preserve its
    // distinct raw-download/tree domains rather than the legacy lock's union.
    let (sha256, tree) = match outcome.identity {
        FetchIdentity::Download(hash) => (
            Some(String::from(hash)),
            store::canonical_tree_hash(&payload)?,
        ),
        FetchIdentity::Tree(tree) => (None, tree),
    };
    if let Some(pin) = locked
        && (pin
            .resolved
            .sha256
            .as_ref()
            .is_some_and(|expected| Some(expected) != sha256.as_ref())
            || pin
                .resolved
                .tree256
                .as_ref()
                .is_some_and(|expected| expected != tree.as_str()))
    {
        return Err(failure(
            "acquired source differs from its frozen byte/tree identity".into(),
        ));
    }
    // Frozen metadata is retained rather than replaced by a transport's newer
    // discovery/version claims. The independently measured tree binds extraction.
    let mut resolved = if let Some(pin) = locked {
        pin.resolved.clone()
    } else {
        let (url, version, api_url) = match metadata {
            Some(metadata) => (Some(metadata.url), Some(metadata.version), metadata.api_url),
            None => (
                outcome.url,
                outcome.version.or_else(|| match &concrete {
                    FetchSpec::Git { rev, .. } => rev.clone(),
                    _ => None,
                }),
                None,
            ),
        };
        ResolvedPinFields {
            url,
            version,
            sha256,
            tree256: None,
            repo256: None,
            api_url,
        }
    };
    resolved.tree256 = Some(tree.to_string());
    Ok(PreparedSource {
        storage: SourceStorage::Staged {
            _capture: temporary,
            payload,
        },
        tree,
        resolved,
    })
}

pub(super) fn repository_source(
    ctx: &Ctx,
    fetch: &FetchSpec,
) -> Result<Option<PathBuf>, ExecError> {
    let FetchSpec::File { path } = fetch else {
        return Ok(None);
    };
    let path = Path::new(path);
    let relative = ctx.repository.source_relative(path);
    relative
        .map(|relative| {
            ctx.repository
                .materialization_path(relative)
                .map_err(ExecError::from)
        })
        .transpose()
}

fn frozen_source(fetch: &FetchSpec, pin: &ResolvedPinFields) -> Result<(), String> {
    if pin.sha256.is_none() && pin.tree256.is_none() {
        return Err("frozen acquisition requires its acquired byte/tree identity".into());
    }
    match fetch {
        FetchSpec::GithubRelease { .. } | FetchSpec::Tarball { .. } if pin.sha256.is_none() => {
            Err("frozen archive acquisition requires its raw download SHA-256".into())
        }
        FetchSpec::GithubRelease { .. } | FetchSpec::Brew { .. } if pin.url.is_none() => {
            Err("frozen discovery requires its selected URL".into())
        }
        FetchSpec::Git { .. }
            if !pin.version.as_ref().is_some_and(|version| {
                matches!(version.len(), 40 | 64)
                    && version
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }) =>
        {
            Err("frozen Git acquisition requires a full immutable commit identity".into())
        }
        _ => Ok(()),
    }
}
