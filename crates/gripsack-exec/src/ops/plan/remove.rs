//! Drift-guarded undeclaration and prior restoration planning.
use super::*;

/// A Remove op (prune-on-undeclare, rollback's current-only
/// destinations): removal authority is the manifest entry, the intent
/// is the prior's restoration or removal (0026 §6). None when nothing
/// needs doing; Preserved when the drift guard keeps the destination
/// (a drifted merge block, a modified copy — the user's now).
pub(crate) fn plan_remove_op(
    output: &str,
    entry: &store::DeployedEntry,
    store_path: &Path,
    home: &gripsack_fs::Dir,
) -> Result<Option<Op>, ExecError> {
    let fail = |detail: String| ExecError::Step {
        output: output.to_string(),
        step: "plan".into(),
        detail,
    };
    let dest = entry.key();
    let dest_dir = match gripsack_fs::open(dest.parent().unwrap_or(Path::new("."))) {
        Ok(directory) => directory,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(fail(format!("cannot inspect {} parent: {error}", entry.to))),
    };
    let dest_name = PathBuf::from(
        dest.file_name()
            .ok_or_else(|| fail("destination has no file name".into()))?,
    );
    let observed = store::journal::live_identity(&dest_dir, &dest_name)
        .map_err(|e| fail(format!("cannot inspect {}: {e}", entry.to)))?;
    let kept = |note: String| {
        tracing::warn!("{note}");
        Ok(Some(Op::new(
            output.to_string(),
            dest.clone(),
            entry.to.clone(),
            entry.ownership.policy().clone(),
            OpKind::Preserved,
            None,
            observed.clone(),
            match &observed {
                Some(o) => Intended::Object(o.clone()),
                None => Intended::Removed,
            },
            None,
            None,
        )))
    };
    if entry.ownership.policy() == Ownership::Merge {
        // the file is foreign — prune removes only our block, and only
        // if the block is still what we deployed
        let existing = match crate::deploy::observe(&dest_dir, &dest_name)? {
            Some(crate::deploy::Observation::File { bytes, mode }) => (bytes, mode),
            None => return Ok(None),
            _ => return kept(format!("kept {} — hosting file replaced", entry.to)),
        };
        let (bytes, splice_mode) = existing;
        let existing = String::from_utf8(bytes).map_err(|e| fail(e.to_string()))?;
        let blocks =
            crate::managed_blocks::ManagedBlockSet::parse(&existing, entry.merge_owner(output))
                .map_err(|e| fail(e.to_string()))?;
        match blocks.blocks() {
            [] => return Ok(None),
            _ if blocks.intact(entry, splice_mode) => {
                let new = blocks
                    .remove()
                    .map_err(|error| fail(error.to_string()))?
                    .expect("intact implies a block");
                let intended = if new.is_empty() {
                    Intended::Removed
                } else {
                    Intended::Object(ObjectIdentity::File(store::canonical_bytes_identity(
                        new.as_bytes(),
                        splice_mode,
                    )))
                };
                return Ok(Some(Op::new(
                    output.to_string(),
                    dest,
                    entry.to.clone(),
                    entry.ownership.policy().clone(),
                    OpKind::Remove(RemoveTarget {
                        entry: entry.clone(),
                        store_path: store_path.to_path_buf(),
                    }),
                    Some(Authority::Update),
                    observed,
                    intended,
                    None,
                    None,
                )));
            }
            _ => {
                return kept(format!(
                    "kept {} — block or hosting-file mode modified since deploy",
                    entry.to
                ));
            }
        }
    }
    // the drift guard runs FIRST (0026 §6) — a kept destination is
    // never journaled at all
    if !crate::deploy::intact_deployed(&dest, entry, store_path) {
        if dest.symlink_metadata().is_ok() {
            return kept(format!("kept {} — modified since deploy", entry.to));
        }
        return Ok(None); // already gone
    }
    let intended =
        crate::deploy::restore::prune_intent(entry, home).map_err(|e| fail(format!("{e}")))?;
    Ok(Some(Op::new(
        output.to_string(),
        dest,
        entry.to.clone(),
        entry.ownership.policy().clone(),
        OpKind::Remove(RemoveTarget {
            entry: entry.clone(),
            store_path: store_path.to_path_buf(),
        }),
        Some(Authority::Update),
        observed,
        intended,
        None,
        None,
    )))
}
