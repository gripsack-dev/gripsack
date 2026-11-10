//! Removal of deployed destinations (prune-on-undeclare, 0006) with
//! exact-target guards (0030 §15).

use gripsack_store as store;
use gripsack_store::Ownership;
use store::journal::Install;

/// What pruning or rolling back a deployed destination installs (0015 §4):
/// the recorded prior when one exists ("your original files have been
/// restored"), with its recorded mode exactly (0027 §6); otherwise the
/// destination is removed — a merge entry removes only its own block from
/// the foreign file. The live object is the deployed one: the plan's K1
/// decision observed it, and the journal's leaf protocol re-checks that
/// observation immediately before acting (0030 §15's exact-target guard).
pub fn removal<'a>(
    entry: &'a store::DeployedEntry,
    output: &'a str,
    home: &gripsack_fs::Dir,
) -> std::io::Result<Install<'a>> {
    use gripsack_host::storage::Permissions;
    if let Some(prior) = &entry.prior {
        return Ok(match prior {
            store::Prior::File { hash, mode } => Install::File {
                bytes: store::prior::read_blob(home, hash)?,
                mode: Permissions::new(mode.bits()),
            },
            store::Prior::Symlink { target } => Install::Link {
                target: target.into(),
            },
        });
    }
    if entry.ownership.policy() != Ownership::Merge {
        return Ok(Install::Remove);
    }
    Ok(Install::Splice(Box::new(
        move |bytes: &[u8], mode: Option<Permissions>| {
            let Some(mode) = mode else {
                return Ok(None);
            };
            let existing = std::str::from_utf8(bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let owner = entry.merge_owner(output);
            let blocks = crate::managed_blocks::ManagedBlockSet::parse(existing, owner)
                .map_err(std::io::Error::other)?;
            // the kernel's K1 for blocks: only an intact block is removed
            match blocks.decide(
                Some(crate::managed_blocks::record(entry)),
                &gripsack_kernel::block::Host::File { mode: mode.bits() },
                &gripsack_kernel::block::BlockIntent::Remove,
            ) {
                gripsack_kernel::block::BlockDecision::Remove => {
                    let new = blocks
                        .remove(owner)
                        .map_err(std::io::Error::other)?
                        .expect("intact implies a block");
                    Ok((!new.is_empty()).then(|| (new.into_bytes(), mode)))
                }
                _ => Err(std::io::Error::other(
                    "the managed block is no longer intact; it is kept",
                )),
            }
        },
    )))
}
