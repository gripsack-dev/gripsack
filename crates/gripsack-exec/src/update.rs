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
        // the repo-overlay half of the pin — a config tree that gains
        // a file moves this WITHOUT moving any transport hash, and the
        // tree256 it invalidates must be re-pinned at the next apply
        let repo256 = crate::resolve::repo_overlay(module, &ctx.repo)?;
        let old_repo = lock
            .modules
            .get(name.as_str())
            .and_then(|e| e.resolved.as_ref())
            .and_then(|r| r.repo256.clone());
        let repo_moved = old_repo != repo256;
        match spec {
            gripsack_ir::FetchSpec::File { .. } | gripsack_ir::FetchSpec::Tarball { .. } => {
                // resolve the payload hash without deploying
                let sha = gripsack_fetch::payload_hash(spec)
                    .map_err(ExecError::Fetch)?
                    .expect("file/tarball always hash");
                if old.as_deref() == Some(sha.as_str()) && !repo_moved {
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
                                repo256: repo256.clone(),
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
                    module: name.clone(),
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
                // Heal a pin whose metadata an older apply dropped
                // (url/version/api_url are re-recorded from this
                // resolution — the sha didn't move, so this is not a
                // bump)
                let metadata_missing = lock
                    .modules
                    .get(name.as_str())
                    .and_then(|e| e.resolved.as_ref())
                    .is_some_and(|r| r.url.is_none() || r.version.is_none());
                if old_sha.as_deref() == Some(sha.as_str()) && !repo_moved && !metadata_missing {
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Unchanged,
                    });
                } else {
                    let moved = old_sha.as_deref() != Some(sha.as_str()) || repo_moved;
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
                                repo256: repo256.clone(),
                            }),
                        },
                    );
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: if moved {
                            UpdateStatus::Bumped {
                                old: old_v.or(old_sha),
                                new: release.version,
                            }
                        } else {
                            UpdateStatus::Unchanged
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
                if old.as_deref() == Some(sha.as_str()) && !repo_moved {
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
                                repo256: repo256.clone(),
                            }),
                        },
                    );
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Bumped { old, new: sha },
                    });
                }
            }
            // git with an inline rev is pinned deliberately — skipped;
            // floating git re-resolves the remote's HEAD (0016 §D2)
            F::Git { url, rev } => match rev {
                // the rev IS the pin — nothing to resolve
                Some(_) => reports.push(UpdateReport {
                    module: name.clone(),
                    status: UpdateStatus::Skipped {
                        reason: "pinned by rev",
                    },
                }),
                None => {
                    let head =
                        gripsack_fetch::resolve_git_head(url).map_err(|e| ExecError::Step {
                            module: name.clone(),
                            step: "update".into(),
                            detail: e.to_string(),
                        })?;
                    // the float's pin is the lock's `version` (0016 §D2)
                    let old = lock
                        .modules
                        .get(name.as_str())
                        .and_then(|e| e.resolved.as_ref())
                        .and_then(|r| r.version.clone());
                    if old.as_deref() == Some(head.as_str()) && !repo_moved {
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
                                    version: Some(head.clone()),
                                    sha256: None,
                                    tree256: None,
                                    api_url: None,
                                    repo256: repo256.clone(),
                                }),
                            },
                        );
                        reports.push(UpdateReport {
                            module: name.clone(),
                            status: UpdateStatus::Bumped { old, new: head },
                        });
                    }
                }
            },
            _ => {
                reports.push(UpdateReport {
                    module: name.clone(),
                    status: UpdateStatus::Skipped {
                        reason: "resolution not supported yet",
                    },
                });
            }
        }
    }
    crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;
    Ok(reports)
}
