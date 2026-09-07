//! Resolve and materialize sources without deploying or running recipes.
//! The lockfile is replaced once, only after every selected source succeeds.

use crate::ctx::{Ctx, ExecError};
use crate::lockfile::{LockRead, Resolved};
use crate::report::{UpdateReport, UpdateStatus};
use gripsack_ir::{Ir, prepared::PreparedModule};

pub fn update(ir: &Ir, ctx: &Ctx) -> Result<Vec<UpdateReport>, ExecError> {
    let _lifecycle_lock = crate::util::acquire_lifecycle_lock(&ctx.home)?;
    let (order, missing) = crate::apply::scoped_order(ir, &ctx.only)?;
    let mut reports: Vec<_> = missing
        .into_iter()
        .map(|module| UpdateReport {
            module,
            status: UpdateStatus::Skipped {
                reason: "not in this host's graph",
            },
        })
        .collect();
    let mut lock = match crate::lockfile::read(&ctx.repo, &ctx.host) {
        LockRead::Parsed(lock) => lock,
        LockRead::Missing => Default::default(),
        LockRead::Corrupt(reason) => {
            return Err(ExecError::Step {
                module: "*".into(),
                step: "lockfile".into(),
                detail: format!(
                    "{} is corrupt ({reason}) — delete it to re-pin from scratch",
                    crate::lockfile::path(&ctx.repo, &ctx.host).display()
                ),
            });
        }
    };
    for name in order {
        let _module = tracing::info_span!("module", module = %name).entered();
        let plan = PreparedModule::new(&ir.modules[&name]).map_err(ExecError::Gate)?;
        let Some(spec) = plan.fetch() else {
            continue;
        };
        let staging = tempfile::Builder::new().prefix("grip-update-").tempdir()?;
        // No old resolution here: update deliberately refreshes floating sources.
        // Inline versions/revisions/digests remain enforced by the declaration.
        let mut entry = crate::source::fetch(
            ctx,
            crate::source::FetchInputs {
                name: &name,
                spec,
                locked: None,
                dest: staging.path(),
            },
        )?;
        let pin = entry.resolved.as_mut().expect("acquisition creates a pin");
        let overlay = crate::source::Overlay::capture(&plan, &ctx.repo, staging.path())?;
        pin.repo256 = if plan.has_recipe() {
            overlay.into_hash()
        } else {
            overlay.merge(staging.path())?
        };
        if !plan.has_recipe() {
            let tree = gripsack_store::canonical_tree_hash(staging.path())?.to_string();
            let destination = gripsack_store::content_path(&ctx.home, &name, &tree);
            if destination.exists() {
                let actual = gripsack_store::canonical_tree_hash(&destination)?;
                if actual.as_str() != tree {
                    return Err(ExecError::Fetch(gripsack_fetch::FetchError::HashMismatch {
                        url: destination.display().to_string(),
                        expected: tree,
                        actual: actual.into(),
                    }));
                }
            } else {
                crate::source::publish(ctx, &name, staging.path(), &destination)?;
            }
            pin.tree256 = Some(tree);
        }
        let old = lock
            .modules
            .get(&name)
            .and_then(|entry| entry.resolved.as_ref());
        let status = if old.is_some_and(|old| same_source(old, pin)) {
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
        lock.modules.insert(name.clone(), entry);
        reports.push(UpdateReport {
            module: name,
            status,
        });
    }
    crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;
    Ok(reports)
}

fn same_source(old: &Resolved, new: &Resolved) -> bool {
    old.sha256 == new.sha256
        && old.repo256 == new.repo256
        && match (&old.version, &new.version) {
            (Some(old), Some(new)) => old == new,
            _ => true, // filling missing metadata is not a payload bump
        }
}
