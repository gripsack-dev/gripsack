//! The per-module lifecycle: satisfy, produce, publish, deploy,
//! verify (0008 §3).

use crate::ctx::{Ctx, ExecError};
use crate::deploy::deploy_entry;
use crate::report::{ReportKind, StepReport, describe_fetch, describe_verify};
use crate::resolve::{module_input, resolve_spec};
use crate::util::{fresh_staging, progress};
use crate::verify::{run_shell, run_verify};
use gripsack_fetch::fetch;
use gripsack_ir::{Build, Step, StepAction, Verify};
use gripsack_store as store;
use std::path::PathBuf;
use tracing::info;

/// Run one module's steps; returns its state for the manifest.
pub(crate) fn run_module(
    name: &str,
    module: &gripsack_ir::Module,
    steps: &[Step],
    ctx: &Ctx,
    prev: Option<&store::ModuleState>,
    locked: Option<&crate::lockfile::LockEntry>,
) -> Result<
    (
        store::ModuleState,
        Vec<StepReport>,
        Option<crate::lockfile::LockEntry>,
    ),
    ExecError,
> {
    // The payload hash participates in the store-path identity from
    // the very first apply (0008 §5) — resolve it before the existence
    // check or the first and second applies would compute different paths.
    let resolved = locked
        .and_then(|e| e.resolved.as_ref())
        .and_then(|r| r.sha256.clone())
        .or_else(|| {
            // the fetch spec lives in module.fetch (declarative) or in a
            // fetch step (explicit steps) — check both
            let spec = module.fetch.as_ref().or_else(|| {
                steps.iter().find_map(|s| match &s.action {
                    StepAction::Fetch { fetch } => Some(fetch),
                    _ => None,
                })
            });
            spec.and_then(|s| gripsack_fetch::payload_hash(s).ok().flatten())
        });
    let input = match &resolved {
        Some(sha) => format!("{}|payload={sha}", module_input(module, &ctx.repo)?),
        None => module_input(module, &ctx.repo)?,
    };
    let store_path = store::store_path(&ctx.home, name, &input);
    // {version} in entry paths substitutes from the locked pin, or
    // from a resolution that happens during this apply (0008 §5)
    let mut version = locked
        .and_then(|e| e.resolved.as_ref())
        .and_then(|r| r.version.clone());
    let mut lock_entry: Option<crate::lockfile::LockEntry> = None;
    // Satisfaction (0008 §3): presence is proof — skip fetch and build.
    let present = store_path.exists();
    let mut reports = Vec::new();
    if present {
        reports.push(StepReport {
            module: name.to_string(),
            summary: "payload already in store".into(),
            kind: ReportKind::Satisfied,
        });
    }
    let mut staging: Option<PathBuf> = None;
    let mut pending_verifies: Vec<&Verify> = Vec::new();

    // Phase A: produce the payload (fetch/build/custom steps).
    if !present {
        for step in steps {
            match &step.action {
                StepAction::Fetch { fetch: spec } => {
                    progress(ctx, name, "fetching");
                    let stage = staging.get_or_insert_with(|| fresh_staging(name));
                    // resolve to a concrete spec — the locked pin wins;
                    // else resolve now (trust on first use, 0002 §3)
                    let (concrete, meta) = resolve_spec(spec, locked)?;
                    if let Some(m) = &meta {
                        version = Some(m.version.clone());
                    }
                    let sha = fetch(&concrete, stage).map_err(ExecError::Fetch)?;
                    // pin enforcement for kinds without download-level
                    // verification (pixi trees, git revs)
                    if let Some(expected) = locked
                        .and_then(|e| e.resolved.as_ref())
                        .and_then(|r| r.sha256.as_ref())
                        && sha != *expected
                        && !matches!(concrete, gripsack_ir::FetchSpec::Tarball { .. })
                    {
                        return Err(ExecError::Fetch(gripsack_fetch::FetchError::HashMismatch {
                            url: format!("{name} payload"),
                            expected: expected.clone(),
                            actual: sha,
                        }));
                    }
                    lock_entry = Some(crate::lockfile::LockEntry {
                        fetch: spec.clone(),
                        resolved: Some(crate::lockfile::Resolved {
                            url: meta.as_ref().map(|m| m.url.clone()),
                            version: meta.as_ref().map(|m| m.version.clone()),
                            sha256: Some(sha),
                        }),
                    });
                    info!(step = %step.id, "fetched");
                    reports.push(StepReport {
                        module: name.to_string(),
                        summary: format!("fetched {}", describe_fetch(spec)),
                        kind: ReportKind::Fetched,
                    });
                }
                StepAction::Build {
                    spec: Build::CustomShell { script },
                }
                | StepAction::CustomShell { script, .. } => {
                    progress(ctx, name, "building");
                    let dir = staging.clone().unwrap_or_else(|| fresh_staging(name));
                    run_shell(script, &dir).map_err(|detail| ExecError::Step {
                        module: name.to_string(),
                        step: step.id.clone(),
                        detail,
                    })?;
                }
                _ => {}
            }
            if let Some(verify) = &step.verify {
                pending_verifies.push(verify);
            }
        }
        let stage = staging.take().unwrap_or_else(|| fresh_staging(name));
        // Config/install content can live in the repo (dotfiles travel
        // with the env repo — 0006): copy referenced files into staging.
        for entry in module.install.iter().chain(module.config.iter()) {
            let repo_file = ctx.repo.join(&entry.from);
            if repo_file.is_file() {
                let dest = stage.join(&entry.from);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&repo_file, &dest)?;
            }
        }
        store::publish_dir(&stage, &store_path)?;
    } else {
        for step in steps {
            if let Some(verify) = &step.verify {
                pending_verifies.push(verify);
            }
        }
    }

    // Phase B: deploy + verify against the published store path.
    let mut deployed = Vec::new();
    for step in steps {
        match &step.action {
            StepAction::Install { entries } | StepAction::ConfigDeploy { entries } => {
                progress(ctx, name, "deploying");
                for entry in entries {
                    let (summary, kind) = deploy_entry(
                        &mut deployed,
                        &store_path,
                        entry,
                        ctx,
                        prev,
                        version.as_deref(),
                    )?;
                    reports.push(StepReport {
                        module: name.to_string(),
                        summary,
                        kind,
                    });
                }
            }
            StepAction::Verify { verify } if !present => {
                progress(ctx, name, "verifying");
                run_verify(name, verify, &store_path, version.as_deref())?;
                reports.push(StepReport {
                    module: name.to_string(),
                    summary: describe_verify(verify),
                    kind: ReportKind::Verified,
                });
            }
            StepAction::Intent { action } => {
                // Activation adapters are 0.2 (0001 §3.8); declared
                // intents are recorded, not yet executed.
                info!(?action, "intent declared (not yet executed)");
            }
            _ => {}
        }
    }
    if !present {
        for verify in pending_verifies {
            run_verify(name, verify, &store_path, version.as_deref())?;
            reports.push(StepReport {
                module: name.to_string(),
                summary: describe_verify(verify),
                kind: ReportKind::Verified,
            });
        }
    }
    Ok((
        store::ModuleState {
            store_path,
            entries: deployed,
        },
        reports,
        lock_entry,
    ))
}
