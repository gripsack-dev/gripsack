//! Explicit external resolution updates. Captured repository bytes are selected
//! by the source bundle, not frozen to an earlier developer checkout in a lock.
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
    Ir, Severity,
    workspace_model::{
        AcquisitionSource, WorkspaceOutput, WorkspaceProducer,
        lock::{LockedPin, PlatformResolution, WORKSPACE_LOCK_VERSION, platform_key},
    },
};

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
    if selected.is_empty() {
        return SurveyReports::new(0).finish();
    }
    let selection = Selection::admit(workspace, selected)?;
    let session = LifecycleSession::acquire(&ctx.home)?;
    let mut pins = WorkspacePins::read(&ctx.repository)?;
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
        selection.required.len()
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
    for name in &selection.required {
        let declaration = selection.outputs[name];
        if let WorkspaceOutput::Service(service) = declaration {
            let next = super::service_pins::pin(service)?;
            let resolution = lock
                .resolutions
                .entry(platform_key(&super::service_pins::platform()))
                .or_insert_with(|| PlatformResolution {
                    pins: Vec::new(),
                    transitive: Vec::new(),
                });
            let previous = resolution.pins.iter().position(|pin| pin.output == *name);
            let status = if previous.is_some_and(|index| resolution.pins[index] == next) {
                UpdateStatus::Unchanged
            } else {
                let old = previous.map(|index| pin_label(&resolution.pins[index]));
                let new = pin_label(&next);
                if let Some(index) = previous {
                    resolution.pins[index] = next;
                } else {
                    resolution.pins.push(next);
                }
                UpdateStatus::Bumped { old, new }
            };
            reports.push(UpdateReport {
                output: (*name).to_owned(),
                status,
                layout: Default::default(),
            })?;
            continue;
        }
        let source = match declaration {
            WorkspaceOutput::Package(package) => match &package.producer {
                WorkspaceProducer::Provider { provider } => Some((provider, &package.target)),
                _ => None,
            },
            _ => None,
        };
        let status = if let Some((source, platform)) = source {
            let key = platform_key(platform);
            if let AcquisitionSource::CondaEnvironment(_) | AcquisitionSource::PixiLock(_) = source
            {
                match super::conda::resolve_update(
                    ctx, &session, name, source, platform, workspace, mode,
                ) {
                    Ok(next) => {
                        let resolution =
                            lock.resolutions
                                .entry(key)
                                .or_insert_with(|| PlatformResolution {
                                    pins: Vec::new(),
                                    transitive: Vec::new(),
                                });
                        let previous = resolution.pins.iter().position(|pin| pin.output == *name);
                        if previous.is_some_and(|index| resolution.pins[index] == next) {
                            UpdateStatus::Unchanged
                        } else {
                            let old = previous.map(|index| pin_label(&resolution.pins[index]));
                            let new = pin_label(&next);
                            if let Some(index) = previous {
                                resolution.pins[index] = next;
                            } else {
                                resolution.pins.push(next);
                            }
                            UpdateStatus::Bumped { old, new }
                        }
                    }
                    Err(error) if mode == UpdateMode::Check => UpdateStatus::Failed {
                        error: Box::new(error),
                    },
                    Err(error) => return Err(error),
                }
            } else if let AcquisitionSource::Fetch(fetch) = source {
                if acquire::repository_source(ctx, &fetch.fetch)?.is_some() {
                    let removed = lock.resolutions.get_mut(&key).is_some_and(|resolution| {
                        let before = resolution.pins.len();
                        resolution.pins.retain(|pin| pin.output != *name);
                        before != resolution.pins.len()
                    });
                    if removed {
                        UpdateStatus::Bumped {
                            old: Some("frozen repository snapshot".into()),
                            new: "current approved source bundle".into(),
                        }
                    } else {
                        UpdateStatus::Skipped {
                            reason: "repository source is captured by this evaluation",
                        }
                    }
                } else {
                    match acquire::acquire(ctx, name, fetch, ResolutionMode::Update) {
                        Ok(acquired) => {
                            let resolved = if mode == UpdateMode::Publish {
                                acquired.publish(ctx, &session, name)?.resolved
                            } else {
                                acquired.resolved
                            };
                            let next = LockedPin {
                                output: (*name).to_owned(),
                                source: source.locked(),
                                resolved,
                                conda: None,
                            };
                            let resolution =
                                lock.resolutions
                                    .entry(key)
                                    .or_insert_with(|| PlatformResolution {
                                        pins: Vec::new(),
                                        transitive: Vec::new(),
                                    });
                            let previous =
                                resolution.pins.iter().position(|pin| pin.output == *name);
                            if previous.is_some_and(|index| resolution.pins[index] == next) {
                                UpdateStatus::Unchanged
                            } else {
                                let old = previous.map(|index| pin_label(&resolution.pins[index]));
                                let new = pin_label(&next);
                                if let Some(index) = previous {
                                    resolution.pins[index] = next;
                                } else {
                                    resolution.pins.push(next);
                                }
                                UpdateStatus::Bumped { old, new }
                            }
                        }
                        Err(error) if mode == UpdateMode::Check => UpdateStatus::Failed {
                            error: Box::new(error),
                        },
                        Err(error) => return Err(error),
                    }
                }
            } else if let AcquisitionSource::Package { manifest, .. } = source {
                let acquired = if mode == UpdateMode::Publish {
                    super::sharing::acquire(ctx, &session, manifest, ctx.fetch.limits())
                } else {
                    super::sharing::inspect_source(&ctx.home, manifest, ctx.fetch.limits())
                        .and_then(|source| source.ok_or_else(|| super::file_failure(
                            declaration.span(), "package projection is not retained; run grip update to acquire it",
                        )))
                };
                match acquired {
                    Ok(acquired) => {
                        let next = LockedPin {
                            output: (*name).to_owned(),
                            source: source.locked(),
                            resolved: acquired.resolved,
                            conda: None,
                        };
                        let resolution =
                            lock.resolutions
                                .entry(key)
                                .or_insert_with(|| PlatformResolution {
                                    pins: Vec::new(),
                                    transitive: Vec::new(),
                                });
                        let previous = resolution.pins.iter().position(|pin| pin.output == *name);
                        if previous.is_some_and(|index| resolution.pins[index] == next) {
                            UpdateStatus::Unchanged
                        } else {
                            let old = previous.map(|index| pin_label(&resolution.pins[index]));
                            let new = pin_label(&next);
                            if let Some(index) = previous {
                                resolution.pins[index] = next;
                            } else {
                                resolution.pins.push(next);
                            }
                            UpdateStatus::Bumped { old, new }
                        }
                    }
                    Err(error) if mode == UpdateMode::Check => UpdateStatus::Failed {
                        error: Box::new(error),
                    },
                    Err(error) => return Err(error),
                }
            } else {
                unreachable!("all acquisition lanes handled")
            }
        } else {
            UpdateStatus::Skipped {
                reason: "no external source resolution",
            }
        };
        reports.push(UpdateReport {
            output: (*name).to_owned(),
            status,
            layout: Default::default(),
        })?;
    }
    let survey = reports.finish()?;
    if survey.summary().publishes_lock(mode) {
        pins.publish(ctx, &session, lock)?;
    }
    Ok(survey)
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
