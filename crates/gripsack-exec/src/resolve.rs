//! Resolution: fetch spec → concrete pin (0002 §8, 0008 §5).

use crate::ctx::ExecError;
use gripsack_store as store;
use std::path::Path;

/// pin wins; github_release resolves through the API (0002 §8).
pub(crate) fn resolve_spec(
    name: &str,
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
            version,
            base_url,
            ..
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
            let release = gripsack_fetch::resolve_latest(
                repo,
                asset,
                base_url.as_deref(),
                version.as_deref(),
            )
            .map_err(|e| ExecError::Step {
                module: name.to_string(),
                step: "resolve".into(),
                detail: e.to_string(),
            })?;
            Ok((
                F::Tarball {
                    url: release.url.clone(),
                    sha256: None,
                    api_url: release.api_url.clone(),
                },
                Some(release),
            ))
        }
        F::Brew {
            formula, version, ..
        } => {
            let meta = gripsack_fetch::resolve_brew(formula).map_err(|e| ExecError::Step {
                module: name.to_string(),
                step: "resolve".into(),
                detail: e.to_string(),
            })?;
            // brew floats to the current formula — the API only serves
            // stable. A declared version must match it or the module
            // fails clearly here, not as a sha mismatch later (brew
            // review): grip update is the way to move the pin.
            if let Some(want) = version
                && meta.version != *want
            {
                return Err(ExecError::Step {
                    module: name.to_string(),
                    step: "resolve".into(),
                    detail: format!(
                        "brew serves {formula} {stable}, but the module pins {want} — \
                         brew() floats to the current formula; `grip update` to move",
                        stable = meta.version
                    ),
                });
            }
            let locked_sha = resolved.and_then(|r| r.sha256.clone());
            Ok((
                F::Brew {
                    formula: formula.clone(),
                    version: version.clone(),
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
        // rev-less git floats (0016 §D2): the locked pin wins; else
        // resolve the remote's default-branch HEAD and pin it. The
        // concrete spec always carries the rev into fetch.
        F::Git { url, rev } => {
            let pinned = match (rev.clone(), resolved.and_then(|r| r.version.clone())) {
                (Some(r), _) => r,
                (None, Some(locked_rev)) => locked_rev,
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
/// only in provenance hash identically.
fn identity_projection(module: &gripsack_ir::Module) -> gripsack_ir::Module {
    let mut projected = module.clone();
    projected.span = None;
    for entry in projected
        .install
        .iter_mut()
        .chain(projected.config.iter_mut())
    {
        entry.span = None;
    }
    for dep in projected.depends.iter_mut() {
        dep.span = None;
    }
    projected
}

/// The repo-overlay half of a module's content identity: the canonical
/// hash of the install/config `from`s that exist in the repo (a
/// payload `from` never does). None when nothing is repo-sourced.
/// Compared against the lock's repo256 by presence checks and
/// `grip update` — a config tree that gains a file moves this without
/// moving the transport pin.
pub(crate) fn repo_overlay(
    module: &gripsack_ir::Module,
    repo: &Path,
    steps: &[gripsack_ir::Step],
) -> Result<Option<String>, ExecError> {
    let step_froms: Vec<String> = steps
        .iter()
        .flat_map(|s| match &s.action {
            gripsack_ir::StepAction::Install { entries }
            | gripsack_ir::StepAction::ConfigDeploy { entries } => {
                entries.iter().map(|e| e.from.clone()).collect::<Vec<_>>()
            }
            _ => vec![],
        })
        .collect();
    let froms: Vec<String> = module
        .install
        .iter()
        .chain(module.config.iter())
        .map(|e| e.from.clone())
        .chain(step_froms)
        .filter(|f| repo.join(f).exists())
        .collect();
    if froms.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        store::canonical_overlay_hash(repo, &froms)?.to_string(),
    ))
}

pub(crate) fn module_input(
    module: &gripsack_ir::Module,
    repo: &Path,
    ir: &gripsack_ir::Ir,
    lock: &crate::lockfile::Lockfile,
    steps: &[gripsack_ir::Step],
) -> Result<String, ExecError> {
    let mut input = serde_json::to_string(&identity_projection(module))?;
    // one normalized graph (0035 F8): declarative fields AND explicit
    // step entries feed the identity alike
    let step_froms: Vec<&String> = steps
        .iter()
        .flat_map(|s| match &s.action {
            gripsack_ir::StepAction::Install { entries }
            | gripsack_ir::StepAction::ConfigDeploy { entries } => {
                entries.iter().map(|e| &e.from).collect::<Vec<_>>()
            }
            _ => vec![],
        })
        .collect();
    for from in module
        .install
        .iter()
        .chain(module.config.iter())
        .map(|e| &e.from)
        .chain(step_froms)
    {
        let repo_file = repo.join(from);
        if repo_file.exists() {
            input.push('|');
            input.push_str(from);
            input.push('=');
            input.push_str(store::canonical_file_hash(&repo_file)?.as_str());
        }
    }
    // the closure model (0001 §3.4): a dependency's identity joins the
    // dependent's input — its DECLARATION (recursive) AND its RESOLVED
    // identity (0035 F4): a pin update must move the consumer's build
    // key, or a rebuilt dep leaves dependents stale
    for dep in &module.depends {
        if let Some(dep_module) = ir.modules.get(&dep.module) {
            input.push_str(&format!(
                "|dep:{}={}",
                dep.module,
                module_input(
                    dep_module,
                    repo,
                    ir,
                    lock,
                    &crate::expand::expand(dep_module)
                )?
            ));
        }
        if let Some(resolved) = lock
            .modules
            .get(&dep.module)
            .and_then(|e| e.resolved.as_ref())
        {
            let pin = format!(
                "{}:{}:{}",
                resolved.sha256.as_deref().unwrap_or("-"),
                resolved.tree256.as_deref().unwrap_or("-"),
                resolved.version.as_deref().unwrap_or("-")
            );
            input.push_str(&format!("|dep-pin:{}={pin}", dep.module));
        }
    }
    Ok(input)
}

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
        let ir = gripsack_ir::Ir {
            ir_version: 1,
            host: gripsack_ir::HostFacts {
                os: "linux".into(),
                arch: "x86_64".into(),
                libc: Some("glibc".into()),
                tags: vec![],
            },
            modules: Default::default(),
            resources: Default::default(),
        };
        let ia = super::module_input(&a, repo, &ir, &Default::default(), &[]).unwrap();
        let ib = super::module_input(&b, repo, &ir, &Default::default(), &[]).unwrap();
        assert_eq!(ia, ib, "span/provenance must not change identity");
    }
}
