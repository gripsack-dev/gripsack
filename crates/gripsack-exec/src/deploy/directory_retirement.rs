//! Undeclaring a migrated directory ends the original ownership epoch, not
//! merely the child-file epoch. Restore the retained predecessor's user origin.
use crate::ExecError;
use gripsack_store as store;

pub(crate) fn retire(
    journal: &store::journal::JournalRun<'_>,
    receipt: &store::journal::directory::DirectoryTransition,
) -> Result<(), ExecError> {
    let predecessor =
        store::generations::admit_manifest(journal.home_path(), receipt.predecessor_generation)?;
    let (owner, state, entry) = predecessor
        .modules
        .iter()
        .find_map(|(owner, state)| {
            state
                .entries
                .iter()
                .find(|entry| entry.key() == receipt.destination)
                .map(|entry| (owner, state, entry))
        })
        .ok_or_else(|| {
            std::io::Error::other("directory retirement lost original ownership lineage")
        })?;
    if entry.preserved_drift
        || entry.ownership.policy() != gripsack_ir::Ownership::Owned
        || state.store_path.join(&entry.from) != receipt.predecessor_target
    {
        return Err(
            std::io::Error::other("directory retirement predecessor differs from receipt").into(),
        );
    }
    store::journal::reverse_directory(journal, receipt)?;
    if let Some(operation) =
        crate::ops::plan_remove_op(owner, entry, &state.store_path, journal.home())?
    {
        if matches!(operation.kind(), crate::ops::OpKind::Preserved) {
            return Err(std::io::Error::other(
                "directory predecessor drifted during retirement; evidence retained",
            )
            .into());
        }
        crate::ops::execute_op(journal, operation.as_executable()?)?;
    }
    Ok(())
}
