//! Per-module preparation is shared; only the publishing driver may commit sources.
mod prepare;
use crate::ctx::{Ctx, ExecError};
use crate::lockfile::{LockRead, Resolved};
use crate::report::{UpdateReport, UpdateStatus};
use gripsack_ir::{Ir, prepared::PreparedModule};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateMode {
    Publish,
    Check,
}

pub fn update(ir: &Ir, ctx: &Ctx, mode: UpdateMode) -> Result<Vec<UpdateReport>, ExecError> {
    let _session = crate::util::LifecycleSession::acquire(&ctx.home)?;
    let (order, missing) = crate::apply::scoped_order(ir, &ctx.only)?;
    let mut reports = Vec::new();
    for name in missing
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
    {
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
        });
    }
    let mut lock = match crate::lockfile::read(&ctx.repo, &ctx.host) {
        LockRead::Parsed(lock) => lock,
        LockRead::Missing => Default::default(),
        LockRead::Corrupt(reason) => {
            return Err(ExecError::Step {
                module: "*".into(),
                step: "lockfile".into(),
                detail: format!(
                    "{} is corrupt ({reason}) — restore it or delete it to re-pin deliberately",
                    crate::lockfile::path(&ctx.repo, &ctx.host).display()
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
            });
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
                });
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
        let unchanged = if mode == UpdateMode::Check {
            old_entry == Some(&prepared.entry)
        } else {
            old.is_some_and(|old| same_source(old, pin))
        };
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
        }
        lock.modules.insert(name.clone(), prepared.entry);
        reports.push(UpdateReport {
            module: name,
            status,
            layout: prepared.layout,
        });
    }
    if mode == UpdateMode::Publish {
        crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;
    }
    Ok(reports)
}

fn same_source(old: &Resolved, new: &Resolved) -> bool {
    old.sha256 == new.sha256
        && old.repo256 == new.repo256
        && match (&old.version, &new.version) {
            (Some(old), Some(new)) => old == new,
            _ => true,
        }
}
