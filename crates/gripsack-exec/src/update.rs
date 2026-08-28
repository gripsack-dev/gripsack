//! `update` — the only lockfile mutator (0008 §5).

use crate::apply::scoped_order;
use crate::ctx::{Ctx, ExecError};
use crate::report::{UpdateReport, UpdateStatus};
use gripsack_ir::Ir;

/// (0008 §5). `grip update` never deploys; apply does.
pub fn update(ir: &Ir, ctx: &Ctx) -> Result<Vec<UpdateReport>, ExecError> {
    // update rewrites the lockfile while apply may be reading it —
    // same lifecycle lock (finding F, closed alongside D)
    let _lifecycle_lock = crate::util::acquire_lifecycle_lock(&ctx.home)?;
    use gripsack_ir::FetchSpec as F;
    let order = scoped_order(ir, &ctx.only)?;
    let mut lock = crate::lockfile::read(&ctx.repo, &ctx.host).unwrap_or_default();
    let mut reports = Vec::new();
    for name in &order {
        let module = &ir.modules[name.as_str()];
        let Some(spec) = &module.fetch else {
            continue;
        };
        let old = lock
            .modules
            .get(name.as_str())
            .and_then(|e| e.resolved.as_ref())
            .and_then(|r| r.sha256.clone());
        match spec {
            gripsack_ir::FetchSpec::File { .. } | gripsack_ir::FetchSpec::Tarball { .. } => {
                // resolve the payload hash without deploying
                let sha = gripsack_fetch::payload_hash(spec)
                    .map_err(ExecError::Fetch)?
                    .expect("file/tarball always hash");
                if old.as_deref() == Some(sha.as_str()) {
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Unchanged,
                    });
                } else {
                    lock.modules.insert(
                        name.clone(),
                        crate::lockfile::LockEntry {
                            fetch: spec.clone(),
                            resolved: Some(crate::lockfile::Resolved {
                                url: None,
                                version: None,
                                sha256: Some(sha.clone()),
                                // the new content's tree is known
                                // only after the next apply fetches
                                // it — deferred identity (0014 §3)
                                tree256: None,
                                api_url: None,
                            }),
                        },
                    );
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Bumped { old, new: sha },
                    });
                }
            }
            F::GithubRelease {
                repo,
                asset,
                version,
                base_url,
                ..
            } => {
                let release = gripsack_fetch::resolve_latest(
                    repo,
                    asset,
                    base_url.as_deref(),
                    version.as_deref(),
                )
                .map_err(|e| ExecError::Step {
                    module: repo.clone(),
                    step: "resolve".into(),
                    detail: e.to_string(),
                })?;
                let sha = gripsack_fetch::payload_hash(&F::Tarball {
                    url: release.url.clone(),
                    sha256: None,
                    api_url: release.api_url.clone(),
                })
                .map_err(ExecError::Fetch)?
                .expect("tarball hashes");
                let old_v = lock
                    .modules
                    .get(name.as_str())
                    .and_then(|e| e.resolved.as_ref())
                    .and_then(|r| r.version.clone());
                let old_sha = lock
                    .modules
                    .get(name.as_str())
                    .and_then(|e| e.resolved.as_ref())
                    .and_then(|r| r.sha256.clone());
                if old_sha.as_deref() == Some(sha.as_str()) {
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Unchanged,
                    });
                } else {
                    lock.modules.insert(
                        name.clone(),
                        crate::lockfile::LockEntry {
                            fetch: spec.clone(),
                            resolved: Some(crate::lockfile::Resolved {
                                url: Some(release.url),
                                version: Some(release.version.clone()),
                                sha256: Some(sha),
                                tree256: None, // deferred identity (0014 §3)
                                api_url: release.api_url,
                            }),
                        },
                    );
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Bumped {
                            old: old_v.or(old_sha),
                            new: release.version,
                        },
                    });
                }
            }
            F::Pixi { .. } | F::Plugin { .. } => {
                // re-resolve into staging and compare the tree hash —
                // pixi and plugin fetches are resolvable too (the
                // "skipped" path left graphs unpinned; enterprise review)
                let staging = ctx.home.join("staging").join(format!(".update-{name}"));
                let _ = std::fs::remove_dir_all(&staging);
                std::fs::create_dir_all(&staging)?;
                let locked_json = None; // resolve fresh, never reproduce
                let outcome = gripsack_fetch::fetch_with_locked(spec, &staging, locked_json)
                    .map_err(ExecError::Fetch)?;
                let sha = outcome.hash;
                let _ = std::fs::remove_dir_all(&staging);
                if old.as_deref() == Some(sha.as_str()) {
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Unchanged,
                    });
                } else {
                    lock.modules.insert(
                        name.clone(),
                        crate::lockfile::LockEntry {
                            fetch: spec.clone(),
                            resolved: Some(crate::lockfile::Resolved {
                                url: outcome.plugin_url,
                                version: outcome.plugin_version,
                                sha256: Some(sha.clone()),
                                tree256: None, // deferred identity (0014 §3)
                                api_url: None,
                            }),
                        },
                    );
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Bumped { old, new: sha },
                    });
                }
            }
            _ => {
                reports.push(UpdateReport {
                    module: name.clone(),
                    status: UpdateStatus::Skipped,
                });
            }
        }
    }
    crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;
    Ok(reports)
}
