//! Explicit external resolution updates. Captured repository bytes are selected
//! by the source bundle, not frozen to an earlier developer checkout in a lock.
mod retention;

use super::{
    acquire::{self, ResolutionMode},
    definitions::captured_definitions,
    pins::WorkspacePins,
    selection::Selection,
};
use crate::{
    Ctx, ExecError, LifecycleSession, UpdateMode,
    report::{SurveyReports, UpdateReport, UpdateStatus, UpdateSurvey},
};
use gripsack_ir::{
    Diagnostic, Ir, Severity, codes,
    workspace_model::{
        AcquisitionSource, WorkspaceCatalog, WorkspaceOutput, WorkspaceProducer,
        lock::{
            LockedPin, PlatformResolution, WORKSPACE_LOCK_VERSION, WorkspaceLock, platform_key,
        },
    },
};
use retention::Acquired;
pub(super) use retention::LockRoots;
use std::collections::BTreeSet;

pub(crate) fn update(ir: &Ir, ctx: &Ctx, mode: UpdateMode) -> Result<UpdateSurvey, ExecError> {
    if let Some(diagnostic) = gripsack_ir::sema::run(ir)
        .into_iter()
        .find(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Err(ExecError::Gate(diagnostic));
    }
    let workspace = &ir.workspace;
    let all;
    let selected = if ctx.only.is_empty() {
        all = workspace
            .outputs
            .iter()
            .map(|output| output.name().to_owned())
            .collect::<Vec<_>>();
        &all
    } else {
        &ctx.only
    };
    let session = LifecycleSession::acquire(&ctx.home)?;
    let mut pins = WorkspacePins::read(&ctx.repository)?;
    let old_conda: BTreeSet<_> = pins
        .lock
        .iter()
        .flat_map(|lock| lock.resolutions.values())
        .flat_map(|resolution| &resolution.pins)
        .filter(|pin| pin.conda.is_some())
        .map(|pin| pin.output.clone())
        .collect();
    if selected.is_empty() && old_conda.is_empty() {
        // An interrupted removal can leave the lock's former closure protected
        // even when no declaration remains to visit on the next publication.
        if mode == UpdateMode::Publish
            && let Some(roots) = LockRoots::plan(
                ctx,
                &session,
                old_conda,
                pins.lock.as_ref(),
                Acquired::default(),
            )?
        {
            pins.publish(ctx, &session, None, Some(roots))?;
        }
        return SurveyReports::new(0).finish();
    }
    let selection = if selected.is_empty() {
        None
    } else {
        Some(Selection::admit(workspace, selected)?)
    };
    let removed: Vec<_> = old_conda
        .iter()
        .filter(|name| {
            ctx.only.is_empty()
                && !selection
                    .as_ref()
                    .is_some_and(|s| s.outputs.contains_key(name.as_str()))
        })
        .collect();
    let definitions = captured_definitions(&ctx.repository)?;
    let changed_definitions = pins
        .lock
        .as_ref()
        .is_none_or(|lock| lock.definitions != definitions);
    let had_definitions = pins.lock.is_some();
    let previous_format = pins
        .lock
        .as_ref()
        .map(|lock| lock.lock_version)
        .filter(|version| *version != WORKSPACE_LOCK_VERSION);
    let mut lock = pins.take_for_update(definitions);
    let mut reports = SurveyReports::new(
        selection.as_ref().map_or(0, |s| s.required.len())
            + removed.len()
            + usize::from(changed_definitions)
            + usize::from(previous_format.is_some()),
    );
    if changed_definitions {
        reports.push(UpdateReport {
            output: "workspace frontend/import pins".into(),
            status: UpdateStatus::Bumped {
                old: had_definitions.then(|| "previous captured definitions".into()),
                new: "current captured definitions".into(),
            },
            layout: Default::default(),
        })?;
    }
    if let Some(version) = previous_format {
        reports.push(UpdateReport {
            output: "workspace lock format".into(),
            status: UpdateStatus::Bumped {
                old: Some(format!("v{version}")),
                new: format!("v{WORKSPACE_LOCK_VERSION}"),
            },
            layout: Default::default(),
        })?;
    }
    for name in removed {
        remove_conda(&mut lock, name);
        reports.push(UpdateReport {
            output: name.clone(),
            status: UpdateStatus::Bumped {
                old: Some("frozen Conda resolution".into()),
                new: "output no longer declared".into(),
            },
            layout: Default::default(),
        })?;
    }
    let mut acquired = Acquired::default();
    if let Some(selection) = selection {
        for name in selection.required {
            let declaration = selection.outputs[name];
            // When a selected name is no longer a provider package, no
            // partition of its former Conda pin can still apply.
            let retired = old_conda.contains(name)
                && !matches!(declaration, WorkspaceOutput::Package(package)
                    if matches!(package.producer, WorkspaceProducer::Provider { .. }))
                && remove_conda(&mut lock, name);
            let result = resolve_output(
                ctx,
                &session,
                workspace,
                declaration,
                mode,
                &mut lock,
                &mut acquired,
            );
            let status = match result {
                Ok(UpdateStatus::Skipped { .. } | UpdateStatus::Unchanged) if retired => {
                    UpdateStatus::Bumped {
                        old: Some("frozen Conda resolution".into()),
                        new: "no longer a Conda provider".into(),
                    }
                }
                Ok(status) => status,
                Err(error) if mode == UpdateMode::Check => UpdateStatus::Failed {
                    error: Box::new(error),
                },
                Err(error) => return Err(error),
            };
            reports.push(UpdateReport {
                output: name.into(),
                status,
                layout: Default::default(),
            })?;
        }
    }
    let survey = reports.finish()?;
    if mode == UpdateMode::Publish {
        let publish_lock = survey.summary().publishes_lock(mode);
        let roots = LockRoots::plan(ctx, &session, old_conda, Some(&lock), acquired)?;
        if publish_lock || roots.as_ref().is_some_and(LockRoots::needs_reconciliation) {
            pins.publish(ctx, &session, publish_lock.then_some(lock), roots)?;
        }
    }
    Ok(survey)
}

fn resolve_output(
    ctx: &Ctx,
    session: &LifecycleSession,
    workspace: &WorkspaceCatalog,
    declaration: &WorkspaceOutput,
    mode: UpdateMode,
    lock: &mut WorkspaceLock,
    acquired: &mut Acquired,
) -> Result<UpdateStatus, ExecError> {
    let name = declaration.name();
    let (key, next) = match declaration {
        WorkspaceOutput::Service(service) => (
            platform_key(&super::service_pins::platform()),
            super::service_pins::pin(service)?,
        ),
        WorkspaceOutput::Package(package) => {
            let WorkspaceProducer::Provider { provider } = &package.producer else {
                return Ok(UpdateStatus::Skipped {
                    reason: "no external source resolution",
                });
            };
            let key = platform_key(&package.target);
            let resolved = match provider {
                AcquisitionSource::CondaEnvironment(_) | AcquisitionSource::PixiLock(_) => {
                    let (next, inputs) = super::conda::resolve_update(
                        ctx,
                        session,
                        name,
                        provider,
                        &package.target,
                        workspace,
                        mode,
                    )?;
                    if mode == UpdateMode::Publish {
                        acquired.record(ctx, &next, inputs);
                    }
                    return Ok(upsert(lock, key, next));
                }
                AcquisitionSource::Fetch(fetch) => {
                    if acquire::repository_source(ctx, &fetch.fetch)?.is_some() {
                        let removed = lock.resolutions.get_mut(&key).is_some_and(|resolution| {
                            let before = resolution.pins.len();
                            resolution.pins.retain(|pin| pin.output != name);
                            before != resolution.pins.len()
                        });
                        return Ok(if removed {
                            UpdateStatus::Bumped {
                                old: Some("frozen repository snapshot".into()),
                                new: "current approved source bundle".into(),
                            }
                        } else {
                            UpdateStatus::Skipped {
                                reason: "repository source is captured by this evaluation",
                            }
                        });
                    }
                    let source = acquire::acquire(ctx, name, fetch, ResolutionMode::Update)?;
                    if mode == UpdateMode::Publish {
                        source.publish(ctx, session, name)?.resolved
                    } else {
                        source.resolved
                    }
                }
                AcquisitionSource::Package { manifest, .. } => {
                    if mode == UpdateMode::Publish {
                        super::sharing::acquire(ctx, session, manifest, ctx.fetch.limits())?
                    } else {
                        super::sharing::inspect_source(&ctx.home, manifest, ctx.fetch.limits())?
                            .map_err(|reason| {
                                let diagnostic = Diagnostic::error(codes::EXEC_STEP, reason);
                                ExecError::Gate(diagnostic.with_label(
                                    Some(declaration.span().clone()),
                                    "imported package declared here",
                                ))
                            })?
                    }
                    .resolved
                }
            };
            (
                key,
                LockedPin {
                    output: name.into(),
                    source: provider.locked(),
                    resolved,
                    conda: None,
                },
            )
        }
        _ => {
            return Ok(UpdateStatus::Skipped {
                reason: "no external source resolution",
            });
        }
    };
    Ok(upsert(lock, key, next))
}

fn remove_conda(lock: &mut WorkspaceLock, name: &str) -> bool {
    let mut removed = false;
    for resolution in lock.resolutions.values_mut() {
        let before = resolution.pins.len();
        resolution
            .pins
            .retain(|pin| pin.output != name || pin.conda.is_none());
        removed |= resolution.pins.len() != before;
    }
    removed
}

fn upsert(lock: &mut WorkspaceLock, key: String, next: LockedPin) -> UpdateStatus {
    let pins = &mut lock
        .resolutions
        .entry(key)
        .or_insert_with(|| PlatformResolution {
            pins: Vec::new(),
            transitive: Vec::new(),
        })
        .pins;
    let previous = pins.iter_mut().find(|pin| pin.output == next.output);
    if previous.as_deref() == Some(&next) {
        return UpdateStatus::Unchanged;
    }
    let new = pin_label(&next);
    let old = if let Some(previous) = previous {
        let old = pin_label(previous);
        *previous = next;
        Some(old)
    } else {
        pins.push(next);
        None
    };
    UpdateStatus::Bumped { old, new }
}

fn pin_label(pin: &LockedPin) -> String {
    pin.resolved
        .version
        .clone()
        .or_else(|| pin.resolved.sha256.clone())
        .or_else(|| pin.resolved.tree256.clone())
        .expect("acquisition measures a source identity")
}

#[cfg(test)]
mod tests;
