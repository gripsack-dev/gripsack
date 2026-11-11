//! One prepared file through the shared ownership planner and journal.
use super::{dest_capability, dest_resolves_into, observe};
use crate::DeploymentEntry;
use crate::apply::{Machine, Verdict, agrees, refusal_error};
use crate::ctx::{Ctx, ExecError};
use crate::report::Tone;
use gripsack_host::store;
use gripsack_host::store::Ownership;
use gripsack_kernel::apply::{OnDrift, Slot, UnitFacts, Want};
use gripsack_kernel::ownership::Recorded;
use gripsack_kernel::recover::{Failure, Ident, Route};
use std::path::Path;

pub(crate) struct DeploymentInput<'a> {
    pub journal: &'a store::journal::JournalRun<'a>,
    pub owner: &'a str,
    pub store_path: &'a Path,
    pub entry: &'a DeploymentEntry,
    pub previous:
        &'a std::collections::BTreeMap<store::OwnershipKey, crate::ops::PreviousRecord<'a>>,
    pub block_id: Option<&'a store::ManagedBlockId>,
    pub migration: Option<&'a super::Migration>,
    /// The kernel's apply machine: it decides every unit (plan/0056 F2 step 7).
    pub machine: &'a mut Machine,
}

/// One content-backed workspace file through observation, authority,
/// mutation, and receipt construction.
pub(crate) fn deploy_entry(
    out: &mut Vec<store::DeployedEntry>,
    ctx: &Ctx,
    input: DeploymentInput<'_>,
) -> Result<(String, Tone), ExecError> {
    let DeploymentInput {
        journal,
        owner: output,
        store_path,
        entry,
        previous,
        block_id,
        migration,
        machine,
    } = input;
    let ownership = store::StoredOwnership::for_entry(&entry.mode, block_id)?;
    let from = &entry.from;
    // Prepared profile paths always name immutable, already published content.
    let source = store_path.join(from);
    // the canonical physical key (0030 §P0-1): one observation, one
    // transition, one journal key per physical object
    let dest =
        store::journal::directory::canonical_destination(journal, &entry.to).map_err(|e| {
            ExecError::Step {
                output: output.to_string(),
                step: "deploy".into(),
                detail: format!("destination {:?}: {e}", entry.to),
            }
        })?;
    let key = store::OwnershipKey::new(
        dest.clone(),
        (entry.mode == Ownership::Merge)
            .then(|| block_id.map_or(output, store::ManagedBlockId::as_str)),
    );
    // the previous record follows the destination's identity: a spelling the
    // filesystem resolves to the recorded object is that destination
    // (plan/0056 FND-3 — a case-only re-spelling on a case-insensitive mount)
    let prev = previous.get(&key).copied().or_else(|| {
        previous
            .iter()
            .find(|(recorded, _)| {
                crate::apply::same_destination(recorded.destination(), &dest)
                    && **recorded
                        == store::OwnershipKey::new(
                            recorded.destination().to_path_buf(),
                            (entry.mode == Ownership::Merge)
                                .then(|| block_id.map_or(output, store::ManagedBlockId::as_str)),
                        )
            })
            .map(|(_, record)| *record)
    });
    let fail = |detail: String| ExecError::Step {
        output: output.to_string(),
        step: "deploy".into(),
        detail,
    };
    // Replacing a destination symlink never writes through it, but no
    // destination entry or ancestor may reside inside captured source.
    let dest_is_symlink = dest
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink());
    let owned_replace_ok = matches!(entry.mode, Ownership::Owned) && dest_is_symlink;
    let inside_source_entry = dest.starts_with(ctx.repository.identity())
        || ctx
            .repository
            .snapshot_root()
            .is_some_and(|root| dest.starts_with(root));
    let resolves_to_source = dest_resolves_into(&dest, ctx.repository.identity())
        || ctx
            .repository
            .snapshot_root()
            .is_some_and(|root| dest_resolves_into(&dest, root));
    if inside_source_entry || (resolves_to_source && !owned_replace_ok) {
        let hint = if dest_is_symlink && !inside_source_entry {
            "\n  hint: remove the symlink into source or use symlink ownership to replace it"
        } else {
            ""
        };
        return Err(fail(format!(
            "{} resolves inside the env repo or captured source ({}) — refusing to deploy into source{hint}",
            entry.to,
            ctx.repository.identity().display()
        )));
    }
    if !source.exists() {
        // install={} keys are payload-relative — a versioned top-level
        // dir in the archive must be part of the key; say what IS here
        let hint = std::fs::read_dir(store_path)
            .map(|entries| {
                let names: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                if names.is_empty() {
                    String::new()
                } else {
                    format!(" (payload top-level: {})", names.join(", "))
                }
            })
            .unwrap_or_default();
        return Err(fail(format!(
            "no payload or repo file at {} (from {}){hint}",
            source.display(),
            entry.from
        )));
    }
    if source.is_dir() && entry.mode != Ownership::Owned {
        return Err(fail(format!(
            "{:?} on a directory ({}) — directory payloads are not supported yet; owned symlinks work today",
            entry.mode, entry.from
        )));
    }
    // template payloads render at deploy time — the vars were computed
    // by the frontend at eval; the core only substitutes (0001 §3.7)
    let rendered = match &entry.mode {
        Ownership::Template => Some(crate::template::render_template(
            &std::fs::read(&source)?,
            &entry.vars,
            &entry.from,
        )?),
        _ => None,
    };
    // the ONE planner (0034): the op IS the decision — plan renders
    // it, apply executes it, rollback constructs it
    let view = crate::ops::DestView {
        output,
        entry,
        dest: dest.clone(),
        home: &ctx.home,
        observed: {
            let (dest_dir, dest_name) =
                match store::journal::directory::destination_capability(journal, &dest)? {
                    Some(capability) => capability,
                    None => dest_capability(&dest)?,
                };
            observe(&dest_dir, &dest_name)?
        },
        prev,
        consent: crate::ops::take_over_consent(ctx.takes_over(&entry.to)),
    };
    let migration_entry = migration.and_then(|migration| migration.entry(&dest));
    if let Some(authorization) = migration_entry {
        authorization.reobserve(&view)?;
    }
    let bytes: Option<Vec<u8>> = match &entry.mode {
        Ownership::TrackedCopy | Ownership::Template => Some(match rendered {
            Some(rendered) => rendered,
            None => std::fs::read(&source)?,
        }),
        _ => None,
    };
    let merge_payload: Option<String> = match &entry.mode {
        Ownership::Merge => Some(
            std::fs::read_to_string(&source)
                .map_err(|e| fail(format!("cannot read {}: {e}", source.display())))?,
        ),
        _ => None,
    };
    // Whole-file outputs share the same source-executability policy.
    #[cfg(unix)]
    let src_exec = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(&source)?.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let src_exec = false;
    let input = match &entry.mode {
        Ownership::Owned => crate::ops::ModeInput::Link {
            source: &source,
            content_hash: store::canonical_file_hash(&source)?.into(),
        },
        Ownership::TrackedCopy | Ownership::Template => {
            let content = bytes.as_deref().unwrap_or_default();
            let permissions = crate::ops::WritePermissions::Source {
                executable: src_exec,
            };
            match migration_entry {
                Some(authorization) => crate::ops::ModeInput::MigrationWrite {
                    authorization,
                    content,
                    permissions,
                },
                None => crate::ops::ModeInput::Write {
                    content,
                    permissions,
                },
            }
        }
        Ownership::Merge => crate::ops::ModeInput::Merge {
            block_id,
            payload: merge_payload.as_deref().unwrap_or_default(),
            permissions: crate::ops::WritePermissions::Preserve,
            source: crate::ops::BlockSource::Declared,
        },
    };
    // the kernel's apply machine decides (plan/0056 F2 step 7); the planner
    // builds the op, which must be exactly what the machine commanded
    let unit_dest = machine.dests.intern(&dest)?;
    let refused = |refusal| match migration_entry {
        Some(_) => ExecError::Step {
            output: output.into(),
            step: "migrate".into(),
            detail: format!(
                "{}: predecessor changed after migration admission",
                entry.to
            ),
        },
        None => refusal_error(refusal, output, Some(&entry.to)),
    };
    let verdict = match crate::ops::unit_want(&view, &input) {
        Some(want) => {
            let facts = UnitFacts {
                dest: unit_dest,
                slot: Slot::Whole,
                route: Route::Leaf,
                recorded: want.recorded,
                want: Want::Deploy {
                    ident: want.ident,
                    adopted: want.adopted,
                },
                // a migration write replaces exactly the admitted predecessor
                consent: match migration_entry {
                    Some(_) => crate::ops::take_over_consent(false),
                    None => view.consent,
                },
                // an owned link the user replaced, or a predecessor that is no
                // longer the admitted one, stops the run; a copy keeps drift
                drift: match (&entry.mode, migration_entry) {
                    (Ownership::Owned, _) | (_, Some(_)) => OnDrift::Stop,
                    _ => OnDrift::Keep,
                },
            };
            let live = crate::ops::kernel_live(view.observed.as_ref());
            machine.unit(facts, Some(live)).map_err(refused)?
        }
        None => {
            // a managed block (K5): the host's block decision settles it
            let owner = machine.owner(block_id.map_or(output, store::ManagedBlockId::as_str));
            // the want names the payload's digest; the kernel never examines
            // a delegated unit's identity
            let payload = Ident::File(store::hash::kernel::link_digest(
                merge_payload.as_deref().unwrap_or_default().as_bytes(),
            ));
            let facts = UnitFacts {
                dest: unit_dest,
                slot: Slot::Block(owner),
                route: Route::Delegated,
                recorded: Recorded::Observed,
                want: Want::Deploy {
                    ident: payload,
                    adopted: payload,
                },
                consent: view.consent,
                drift: OnDrift::Keep,
            };
            machine.unit(facts, None).map_err(refused)?
        }
    };
    let op = crate::ops::plan_entry_op(&view, input)?;
    if !matches!(verdict, Verdict::Delegate) && !agrees(&op, &verdict) {
        return Err(fail(format!(
            "{}: the planner's op does not match the kernel's decision ({verdict:?}) — a gripsack bug; nothing was written",
            entry.to
        )));
    }
    let executed = match crate::ops::execute_op(journal, op.as_executable()?) {
        Ok(executed) => executed,
        Err(error) => {
            if matches!(verdict, Verdict::Write { .. }) {
                machine.failed(Failure::Io(0));
            }
            return Err(error);
        }
    };
    match verdict {
        Verdict::Write { .. } => {
            let landed = executed
                .landed
                .ok_or_else(|| fail(format!("{}: a commanded write landed nothing", entry.to)))?;
            machine.landed(landed).map_err(refused)?;
        }
        Verdict::Delegate => machine
            .delegated(executed.landed.is_some())
            .map_err(refused)?,
        Verdict::Settled(_) => {}
    }
    let (report, captured_prior) = (executed.report, executed.prior);
    // the manifest entry: what the op produces, or the previous entry
    // carried forward (satisfied)
    match op.produces() {
        Some(produced) => {
            out.push(store::DeployedEntry {
                // the EXPANDED key — rollback and store verify re-join
                // it against the store path verbatim
                from: std::path::PathBuf::from(from),
                to: entry.to.clone(),
                key: Some(dest.clone()),
                ownership,
                vars: entry.vars.clone(),
                hash: produced.hash.clone(),
                file_mode: produced.file_mode,
                source_executable: produced.source_executable,
                prior: captured_prior.or_else(|| produced.prior.clone()),
                preserved_drift: produced.preserved_drift,
            });
        }
        None => {
            if let Some(record) = prev {
                out.push(record.entry.clone());
            }
        }
    }
    Ok((report.summary, report.tone))
}
