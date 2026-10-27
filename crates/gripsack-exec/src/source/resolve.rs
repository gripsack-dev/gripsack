//! Resolution: fetch spec → concrete pin (0002 §8, 0008 §5).

use crate::ctx::ExecError;

/// pin wins; github_release resolves through the API (0002 §8).
pub(crate) fn resolve_spec(
    name: &str,
    spec: &gripsack_ir::FetchSpec,
    resolved: Option<&gripsack_ir::workspace_model::lock::ResolvedPinFields>,
    context: &gripsack_fetch::FetchContext,
) -> Result<
    (
        gripsack_ir::FetchSpec,
        Option<gripsack_fetch::ResolvedRelease>,
    ),
    ExecError,
> {
    use gripsack_ir::FetchSpec as F;
    match spec {
        F::GithubRelease {
            repo,
            asset,
            version,
            base_url,
            sha256,
        } => {
            if let Some(url) = resolved.and_then(|r| r.url.clone()) {
                return Ok((
                    F::Tarball {
                        url,
                        sha256: resolved.and_then(|r| r.sha256.clone()),
                        api_url: resolved.and_then(|r| r.api_url.clone()),
                    },
                    None,
                ));
            }
            let release = context
                .resolve_latest(repo, asset, base_url.as_deref(), version.as_deref())
                .map_err(|error| ExecError::Resolve {
                    output: name.to_string(),
                    error: Box::new(error),
                })?;
            Ok((
                F::Tarball {
                    url: release.url.clone(),
                    sha256: sha256.clone(),
                    api_url: release.api_url.clone(),
                },
                Some(release),
            ))
        }
        // The lock pins the resolved commit, including when the declaration
        // names a mutable branch/tag. Update passes no old resolution.
        F::Git { url, rev } => {
            let pinned = match (resolved.and_then(|r| r.version.clone()), rev.clone()) {
                (Some(locked_rev), _) => locked_rev,
                (None, Some(revision)) => revision,
                (None, None) => {
                    gripsack_fetch::resolve_git_head(context, url).map_err(|e| ExecError::Step {
                        output: name.to_string(),
                        step: "resolve".into(),
                        detail: e.to_string(),
                    })?
                }
            };
            Ok((
                F::Git {
                    url: url.clone(),
                    rev: Some(pinned),
                },
                None,
            ))
        }
        other => Ok((inject_locked_sha(other, resolved), None)),
    }
}

/// A locked hash overrides the spec's for verification.
fn inject_locked_sha(
    spec: &gripsack_ir::FetchSpec,
    resolved: Option<&gripsack_ir::workspace_model::lock::ResolvedPinFields>,
) -> gripsack_ir::FetchSpec {
    let Some(sha) = resolved.and_then(|pin| pin.sha256.as_ref()) else {
        return spec.clone();
    };
    match spec.clone() {
        gripsack_ir::FetchSpec::Tarball { url, api_url, .. } => gripsack_ir::FetchSpec::Tarball {
            url,
            sha256: Some(sha.clone()),
            api_url: api_url.or_else(|| resolved.and_then(|pin| pin.api_url.clone())),
        },
        other => other,
    }
}
