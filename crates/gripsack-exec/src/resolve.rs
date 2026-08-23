//! Resolution: fetch spec → concrete pin (0002 §8, 0008 §5).

use crate::ctx::ExecError;
use gripsack_store as store;
use std::path::Path;

/// pin wins; github_release resolves through the API (0002 §8).
pub(crate) fn resolve_spec(
    spec: &gripsack_ir::FetchSpec,
    locked: Option<&crate::lockfile::LockEntry>,
) -> Result<
    (
        gripsack_ir::FetchSpec,
        Option<gripsack_fetch::ResolvedRelease>,
    ),
    ExecError,
> {
    use gripsack_ir::FetchSpec as F;
    let resolved = locked.and_then(|e| e.resolved.as_ref());
    match spec {
        F::GithubRelease {
            repo,
            asset,
            base_url,
            ..
        } => {
            if let Some(url) = resolved.and_then(|r| r.url.clone()) {
                return Ok((
                    F::Tarball {
                        url,
                        sha256: resolved.and_then(|r| r.sha256.clone()),
                    },
                    None,
                ));
            }
            let release = gripsack_fetch::resolve_latest(repo, asset, base_url.as_deref())
                .map_err(|e| ExecError::Step {
                    module: repo.clone(),
                    step: "resolve".into(),
                    detail: e.to_string(),
                })?;
            Ok((
                F::Tarball {
                    url: release.url.clone(),
                    sha256: None,
                },
                Some(release),
            ))
        }
        F::Brew { formula, .. } => {
            let meta = gripsack_fetch::resolve_brew(formula).map_err(|e| ExecError::Step {
                module: formula.clone(),
                step: "resolve".into(),
                detail: e.to_string(),
            })?;
            let locked_sha = resolved.and_then(|r| r.sha256.clone());
            Ok((
                F::Brew {
                    formula: formula.clone(),
                    sha256: locked_sha,
                },
                Some(meta),
            ))
        }
        F::Pixi {
            package, version, ..
        } => {
            let locked_sha = resolved.and_then(|r| r.sha256.clone());
            Ok((
                F::Pixi {
                    package: package.clone(),
                    version: version.clone(),
                    sha256: locked_sha,
                },
                None,
            ))
        }
        other => Ok((inject_locked_sha(other, locked), None)),
    }
}

/// A locked hash overrides the spec's for verification.
fn inject_locked_sha(
    spec: &gripsack_ir::FetchSpec,
    locked: Option<&crate::lockfile::LockEntry>,
) -> gripsack_ir::FetchSpec {
    let Some(entry) = locked else {
        return spec.clone();
    };
    let Some(sha) = entry.resolved.as_ref().and_then(|r| r.sha256.as_ref()) else {
        return spec.clone();
    };
    match spec.clone() {
        gripsack_ir::FetchSpec::Tarball { url, .. } => gripsack_ir::FetchSpec::Tarball {
            url,
            sha256: Some(sha.clone()),
        },
        other => other,
    }
}

/// files, so editing a dotfile changes the identity (0008 §2).
pub(crate) fn module_input(module: &gripsack_ir::Module, repo: &Path) -> Result<String, ExecError> {
    let mut input = serde_json::to_string(module)?;
    for entry in module.install.iter().chain(module.config.iter()) {
        let repo_file = repo.join(&entry.from);
        if repo_file.exists() {
            input.push('|');
            input.push_str(&entry.from);
            input.push('=');
            input.push_str(&store::canonical_file_hash(&repo_file)?);
        }
    }
    Ok(input)
}
