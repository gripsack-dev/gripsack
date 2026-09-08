//! Resolution: fetch spec → concrete pin (0002 §8, 0008 §5).

use crate::ctx::ExecError;

/// pin wins; github_release resolves through the API (0002 §8).
pub(crate) fn resolve_spec(
    name: &str,
    spec: &gripsack_ir::FetchSpec,
    locked: Option<&crate::lockfile::LockEntry>,
    context: &gripsack_fetch::FetchContext,
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
                    module: name.to_string(),
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
        F::Brew {
            formula,
            version,
            sha256,
        } => Ok((
            F::Brew {
                formula: formula.clone(),
                version: version.clone(),
                sha256: resolved
                    .and_then(|pin| pin.sha256.clone())
                    .or_else(|| sha256.clone()),
            },
            None,
        )),
        F::Pixi {
            package,
            version,
            sha256,
        } => Ok((
            F::Pixi {
                package: package.clone(),
                version: resolved
                    .and_then(|pin| pin.version.clone())
                    .or_else(|| version.clone()),
                sha256: resolved
                    .and_then(|pin| pin.sha256.clone())
                    .or_else(|| sha256.clone()),
            },
            None,
        )),
        // The lock pins the resolved commit, including when the declaration
        // names a mutable branch/tag. Update passes no old resolution.
        F::Git { url, rev } => {
            let pinned = match (resolved.and_then(|r| r.version.clone()), rev.clone()) {
                (Some(locked_rev), _) => locked_rev,
                (None, Some(revision)) => revision,
                (None, None) => {
                    gripsack_fetch::resolve_git_head(url).map_err(|e| ExecError::Step {
                        module: name.to_string(),
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
        gripsack_ir::FetchSpec::Tarball { url, api_url, .. } => gripsack_ir::FetchSpec::Tarball {
            url,
            sha256: Some(sha.clone()),
            api_url: api_url.or_else(|| entry.resolved.as_ref().and_then(|r| r.api_url.clone())),
        },
        other => other,
    }
}

/// The identity projection (0004 §2, enforced): provenance NEVER
/// changes identity. A module's store-path input serializes the module
/// with span removed — a line edit in your module source, or the same
/// repo cloned at a different absolute path, must not re-fetch the
/// world. The regression test pins this: two IR documents differing
mod input;
mod recipes;
pub(crate) use input::repo_overlay;
pub(crate) use recipes::RecipeGraph;

#[cfg(test)]
mod identity_tests {
    //! 0004 §2, load-bearing: provenance must NEVER change identity.

    #[test]
    fn provenance_differences_hash_identically() {
        let json = |span_file: &str, module_line: u32| {
            format!(
                r#"{{"fetch": {{"kind": "tarball", "url": "https://x/y.tar.gz"}},
                 "config": [{{"from": "c.toml", "to": "~/.c.toml",
                              "span": {{"file": "{span_file}", "line": {module_line}}}}}],
                 "span": {{"file": "{span_file}", "line": {module_line}}}}}"#
            )
        };
        let a: gripsack_ir::Module =
            serde_json::from_str(&json("/home/alice/env/modules/m.py", 3)).unwrap();
        let b: gripsack_ir::Module =
            serde_json::from_str(&json("/home/bob/dotfiles/modules/m.py", 47)).unwrap();
        let repo = std::path::Path::new("/nonexistent");
        let mut ir = gripsack_ir::Ir {
            ir_version: gripsack_ir::IR_VERSION,
            host: gripsack_ir::HostFacts {
                os: "linux".into(),
                arch: "x86_64".into(),
                libc: Some("glibc".into()),
                tags: vec![],
            },
            modules: Default::default(),
            resources: Default::default(),
        };
        ir.modules.insert("m".into(), a);
        let plans = crate::expand::expand_all(&ir.modules).unwrap();
        let ia = super::RecipeGraph::new(&ir, repo, &plans, ["m"])
            .unwrap()
            .input("m", &Default::default());
        ir.modules.insert("m".into(), b);
        let plans = crate::expand::expand_all(&ir.modules).unwrap();
        let ib = super::RecipeGraph::new(&ir, repo, &plans, ["m"])
            .unwrap()
            .input("m", &Default::default());
        assert_eq!(ia, ib, "span/provenance must not change identity");
    }
}
