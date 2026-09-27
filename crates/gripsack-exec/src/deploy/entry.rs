//! One prepared file through the shared ownership planner and journal.
use super::{dest_capability, dest_resolves_into, observe};
use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use std::path::Path;

pub(crate) struct DeploymentInput<'a> {
    pub owner: &'a str,
    pub store_path: &'a Path,
    pub entry: &'a Entry,
    pub previous: &'a std::collections::BTreeMap<store::OwnershipKey, &'a store::DeployedEntry>,
    pub version: Option<&'a str>,
    pub block_id: Option<&'a store::ManagedBlockId>,
}

/// One content-backed file; workspace and legacy callers share every
/// observation, authority decision, mutation and receipt constructor.
pub(crate) fn deploy_entry(
    out: &mut Vec<store::DeployedEntry>,
    ctx: &Ctx,
    input: DeploymentInput<'_>,
) -> Result<(String, ReportKind), ExecError> {
    let DeploymentInput {
        owner: module,
        store_path,
        entry,
        previous,
        version,
        block_id,
    } = input;
    let ownership = store::StoredOwnership::for_entry(&entry.mode, block_id)?;
    let prepared =
        crate::source::payload_source(store_path, &entry.from, version).map_err(|error| {
            ExecError::Step {
                module: module.into(),
                step: "deploy".into(),
                detail: error.to_string(),
            }
        })?;
    let from = prepared.relative;
    // Entry content is the store payload — always. The publish step
    // stages every repo-referenced `from` into the store, so a store
    // miss means a stale store (e.g. a config tree that gained a file
    // under an unmoved pin): that is an integrity failure, never a
    // reason to reach into the repo checkout and deploy a path the
    // store never published.
    let source = prepared.path;
    // the canonical physical key (0030 §P0-1): one observation, one
    // transition, one journal key per physical object
    let dest = store::canonical_dest(&entry.to).map_err(|e| ExecError::Step {
        module: module.to_string(),
        step: "deploy".into(),
        detail: format!("destination {:?}: {e}", entry.to),
    })?;
    let key = store::OwnershipKey::new(
        dest.clone(),
        (entry.mode == Ownership::Merge)
            .then(|| block_id.map_or(module, store::ManagedBlockId::as_str)),
    );
    let prev = previous.get(&key).copied();
    let fail = |detail: String| ExecError::Step {
        module: module.to_string(),
        step: "deploy".into(),
        detail,
    };
    // A destination resolving INTO the env repo turns a deploy into a
    // delete: a symlinked ancestor dir (a leftover from another
    // provisioner) lands the write inside the checkout and the module
    // eats its own source. The repo is never a legitimate target.
    //
    // One exception: an `owned` destination that is ITSELF a symlink
    // into the repo — almost certainly an artifact an older gripsack
    // wrote when config deployed straight from the checkout. Owned
    // semantics replace the link (nothing is ever written THROUGH
    // it), so swapping it for a store link is safe and is the only
    // migration path forward; refusing here stranded every config
    // module that predates the store (first apply after upgrade,
    // forever, with an error that pointed at the module instead of
    // the stale link).
    let dest_is_symlink = dest
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink());
    let owned_replace_ok = matches!(entry.mode, Ownership::Owned) && dest_is_symlink;
    if dest_resolves_into(&dest, &ctx.repo) && !owned_replace_ok {
        let hint = if dest_is_symlink {
            "\n  hint: the destination is a symlink into the repo — likely left by an \
             older gripsack that deployed config from the checkout; remove it and \
             re-apply, or declare the entry `owned` so gripsack replaces it"
        } else {
            ""
        };
        return Err(fail(format!(
            "{} resolves inside the env repo ({}) — refusing to deploy into the source checkout{hint}",
            entry.to,
            ctx.repo.display()
        )));
    }
    // Expansion is total: a placeholder surviving to deploy means a
    // {version} with no locked tag or a substitution bug — never a
    // path worth linking
    if from.contains('{') {
        return Err(fail(format!(
            "{} still contains a placeholder after expansion (from {})",
            from, entry.from
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
        module,
        entry,
        dest: dest.clone(),
        home: &ctx.home,
        observed: {
            let (dest_dir, dest_name) = dest_capability(&dest)?;
            observe(&dest_dir, &dest_name)?
        },
        prev,
        take_over: ctx.takes_over(&entry.to),
    };
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
            crate::ops::plan_entry_op(
                &view,
                crate::ops::ModeInput::Write {
                    content,
                    permissions: crate::ops::WritePermissions::Source {
                        executable: src_exec,
                    },
                },
            )?
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
            module: module.to_string(),
            step: "deploy".into(),
            detail: format!(
                "{} exists and was not deployed by gripsack — move it away or use --take-over",
                entry.to
            ),
        });
    }
    let (report, captured_prior) = crate::ops::execute_op(ctx.home_dir()?, op.as_executable()?)?;
    // the manifest entry: what the op produces, or the previous entry
    // carried forward (satisfied)
    match op.produces() {
        Some(produced) => {
            out.push(store::DeployedEntry {
                // the EXPANDED key — rollback and store verify re-join
                // it against the store path verbatim
                from: std::path::PathBuf::from(&from),
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
