//! One prepared file through the shared ownership planner and journal.
use super::{dest_capability, dest_resolves_into, observe};
use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;
use gripsack_store as store;
use gripsack_store::{DeploymentEntry, Ownership};
use std::path::Path;

pub(crate) struct DeploymentInput<'a> {
    pub journal: &'a store::journal::JournalRun<'a>,
    pub owner: &'a str,
    pub store_path: &'a Path,
    pub entry: &'a DeploymentEntry,
    pub previous: &'a std::collections::BTreeMap<store::OwnershipKey, &'a store::DeployedEntry>,
    pub block_id: Option<&'a store::ManagedBlockId>,
    pub migration: Option<&'a super::Migration>,
}

/// One content-backed workspace file through observation, authority,
/// mutation, and receipt construction.
pub(crate) fn deploy_entry(
    out: &mut Vec<store::DeployedEntry>,
    ctx: &Ctx,
    input: DeploymentInput<'_>,
) -> Result<(String, ReportKind), ExecError> {
    let DeploymentInput {
        journal,
        owner: output,
        store_path,
        entry,
        previous,
        block_id,
        migration,
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
    let prev = previous.get(&key).copied();
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
        take_over: ctx.takes_over(&entry.to),
    };
    let migration_entry = migration.and_then(|migration| migration.entry(&dest));
    if let Some(authorization) = migration_entry {
        authorization.reobserve(&view)?;
    }
    let op = match &entry.mode {
        Ownership::Owned => {
            let already = std::fs::read_link(&dest)
                .map(|t| t == source)
                .unwrap_or(false);
            crate::ops::plan_entry_op(
                &view,
                crate::ops::ModeInput::Link {
                    source: &source,
                    content_hash: store::canonical_file_hash(&source)?.into(),
                    already,
                },
            )?
        }
        Ownership::TrackedCopy | Ownership::Template => {
            let content: &[u8] = match &rendered {
                Some(r) => r.as_slice(),
                None => &std::fs::read(&source)?,
            };
            // Whole-file outputs share the same source-executability policy.
            #[cfg(unix)]
            let src_exec = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&source)?.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let src_exec = false;
            let input = match migration_entry {
                Some(authorization) => crate::ops::ModeInput::MigrationWrite {
                    authorization,
                    content,
                    permissions: crate::ops::WritePermissions::Source {
                        executable: src_exec,
                    },
                },
                None => crate::ops::ModeInput::Write {
                    content,
                    permissions: crate::ops::WritePermissions::Source {
                        executable: src_exec,
                    },
                },
            };
            crate::ops::plan_entry_op(&view, input)?
        }
        Ownership::Merge => {
            let payload = std::fs::read_to_string(&source)
                .map_err(|e| fail(format!("cannot read {}: {e}", source.display())))?;
            crate::ops::plan_entry_op(
                &view,
                crate::ops::ModeInput::Merge {
                    block_id,
                    payload: &payload,
                    permissions: crate::ops::WritePermissions::Preserve,
                },
            )?
        }
    };
    // a foreign destination blocks apply (the renderer shows the same
    // op as "needs --take-over")
    if op.authority() == Some(crate::ops::Authority::Foreign) {
        return Err(ExecError::Step {
            output: output.to_string(),
            step: "deploy".into(),
            detail: format!(
                "{} exists and was not deployed by gripsack — move it away or use --take-over",
                entry.to
            ),
        });
    }
    let (report, captured_prior) = crate::ops::execute_op(journal, op.as_executable()?)?;
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
            if let Some(prev_entry) = prev {
                out.push((*prev_entry).clone());
            }
        }
    }
    Ok((report.summary, report.kind))
}
