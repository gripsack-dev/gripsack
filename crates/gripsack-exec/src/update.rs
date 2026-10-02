//! Per-module preparation is shared; only the publishing driver may commit sources.
mod prepare;
use crate::ctx::{Ctx, ExecError};
use crate::lockfile::LockRead;
use crate::report::{SurveyReports, UpdateReport, UpdateStatus, UpdateSurvey};
use gripsack_ir::{Ir, prepared::PreparedModule};

pub use gripsack_policy::update_survey::UpdateMode;

pub fn update(ir: &Ir, ctx: &Ctx, mode: UpdateMode) -> Result<UpdateSurvey, ExecError> {
    if let Some(diagnostic) =
        ir.workspace_execution_error(gripsack_ir::workspace::WorkspaceOperation::Update)
    {
        return Err(ExecError::Gate(diagnostic));
    }
    if let Some(workspace) = &ir.workspace {
        let native = crate::workspace::NativeProfiles::prepare(
            workspace,
            ctx.repository.contents(),
            &ctx.home,
            &ctx.only,
            ctx.fetch.limits(),
        )?;
        return native.update_reports();
    }
    let _session = crate::util::LifecycleSession::acquire(&ctx.home)?;
    let (order, missing) = crate::apply::scoped_order(ir, &ctx.only)?;
    let missing: std::collections::BTreeSet<_> = missing.into_iter().collect();
    let selected = order
        .len()
        .checked_add(missing.len())
        .ok_or_else(|| ExecError::Step {
            module: "*".into(),
            step: "selection".into(),
            detail: "selected update entry count exceeds the addressable range".into(),
        })?;
    let mut reports = SurveyReports::new(selected);
    for name in missing {
        let status = if mode == UpdateMode::Check {
            UpdateStatus::Failed {
                error: Box::new(ExecError::Step {
                    module: name.clone(),
                    step: "selection".into(),
                    detail: "not in this host's graph".into(),
                }),
            }
        } else {
            UpdateStatus::Skipped {
                reason: "not in this host's graph",
            }
        };
        reports.push(UpdateReport {
            module: name,
            status,
            layout: Default::default(),
        })?;
    }
    let mut lock = match crate::lockfile::read(ctx.repository.identity(), &ctx.host) {
        LockRead::Parsed(lock) => lock,
        LockRead::Missing => Default::default(),
        LockRead::Corrupt(reason) => {
            return Err(ExecError::Step {
                module: "*".into(),
                step: "lockfile".into(),
                detail: format!(
                    "{} is corrupt ({reason}) — restore it or delete it to re-pin deliberately",
                    crate::lockfile::path(ctx.repository.identity(), &ctx.host).display()
                ),
            });
        }
    };
    for name in order {
        let _module = tracing::info_span!("module", module = %name).entered();
        let plan = PreparedModule::new(&ir.modules[&name]).map_err(ExecError::Gate)?;
        if plan.fetch().is_none() {
            reports.push(UpdateReport {
                module: name,
                status: UpdateStatus::Skipped {
                    reason: "no fetch source",
                },
                layout: Default::default(),
            })?;
            continue;
        }
        let prepared = match prepare::PreparedUpdate::acquire(ctx, &name, &plan) {
            Ok(prepared) => prepared,
            Err(error) if mode == UpdateMode::Check => {
                reports.push(UpdateReport {
                    module: name,
                    status: UpdateStatus::Failed {
                        error: Box::new(error),
                    },
                    layout: Default::default(),
                })?;
                continue;
            }
            Err(error) => return Err(error),
        };
        let pin = prepared
            .entry
            .resolved
            .as_ref()
            .expect("acquisition creates a pin");
        let old_entry = lock.modules.get(&name);
        let old = old_entry.and_then(|entry| entry.resolved.as_ref());
        // Check predicts the exact publication decision, including source
        // metadata and spec changes — not a weaker hash/version projection.
        let unchanged = old_entry == Some(&prepared.entry);
        let status = if unchanged {
            UpdateStatus::Unchanged
        } else {
            UpdateStatus::Bumped {
                old: old.and_then(|pin| pin.version.clone().or_else(|| pin.sha256.clone())),
                new: pin
                    .version
                    .clone()
                    .or_else(|| pin.sha256.clone())
                    .expect("source identity"),
            }
        };
        if mode == UpdateMode::Publish {
            prepared.publish(ctx, &name)?;
            if !unchanged {
                lock.modules.insert(name.clone(), prepared.entry);
            }
        }
        reports.push(UpdateReport {
            module: name,
            status,
            layout: prepared.layout,
        })?;
    }
    let survey = reports.finish()?;
    if survey.summary().publishes_lock(mode) {
        crate::lockfile::write(ctx.repository.identity(), &ctx.host, &lock)?;
    }
    Ok(survey)
}

#[cfg(test)]
mod tests;
