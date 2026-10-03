use super::{
    MAX_ROOT_ENTRIES, RootId, RootKind, directory, error, read_record, remove_build_staging,
    remove_root,
};
use crate::{ExecError, LifecycleSession};
use gripsack_buildkit::worker::WorkerQuiescence;
use std::path::Path;

#[derive(Debug, Default)]
pub struct BuildRecovery {
    pub retired: usize,
    pub live: usize,
    pub outside_fence: usize,
}

/// Retire only abandoned attempts covered by a real owned-daemon stop. A
/// namespace's durable owner nonce and epoch fence prevent a later incarnation
/// or a newly started build from being mistaken for that stopped worker.
pub fn recover_builder_roots(
    session: &LifecycleSession,
    quiescence: &WorkerQuiescence,
) -> Result<BuildRecovery, ExecError> {
    let home = gripsack_fs::open(session.home())?;
    let Some(processes) = directory(&home, RootKind::Process, false)? else {
        return Ok(BuildRecovery::default());
    };
    // Snapshot names before deleting paired records/locks. A readdir buffer may
    // otherwise report the lock just removed with the preceding JSON record.
    let mut names = Vec::new();
    for entry in processes.entries()? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(error("process root entry is not a regular file"));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| error("non-UTF-8 process root name"))?;
        let id = name
            .strip_suffix(".json")
            .or_else(|| name.strip_suffix(".flock"))
            .ok_or_else(|| error("unknown process root entry"))?;
        RootId::try_from(id.to_owned())?;
        names.push(name);
        if names.len() > MAX_ROOT_ENTRIES {
            return Err(error("process root inventory exceeds its entry bound"));
        }
    }
    names.sort_unstable();
    let mut result = BuildRecovery::default();
    for name in names.iter().filter(|name| name.ends_with(".json")) {
        let record = read_record(&processes, name, RootKind::Process, session.home())?
            .ok_or_else(|| error("process record became a lock"))?;
        let Some(build) = &record.build else {
            continue;
        }; // Native descendants need their own completion authority.
        let lock_name = format!("{}.flock", record.id.as_str());
        // Missing evidence is corruption, not permission to create a new lock
        // inode and declare an existing writer dead.
        gripsack_fs::open_file_nofollow(&processes, Path::new(&lock_name))?;
        let Some(_exclusive) =
            gripsack_fs::FlockGuard::try_acquire_in(&processes, record.id.as_str())?
        else {
            result.live += 1;
            continue;
        };
        if build
            .worker
            .is_some_and(|worker| !quiescence.retires(worker.scope, worker.binding.epoch))
        {
            result.outside_fence += 1;
            continue;
        }
        // No worker binding means submit was never authorized. Lowering also
        // inherits this root's lease, so a live native bridge keeps the lock.
        remove_build_staging(&home, &record.id)?;
        remove_root(session, RootKind::Process, &record.id)?;
        result.retired += 1;
    }
    // Registration publishes a record before starting a child. A lock left
    // without a record is either an interrupted registration or a completed
    // removal; it may be unlinked only after its inherited handles drain.
    for name in names.iter().filter(|name| name.ends_with(".flock")) {
        let id = name.strip_suffix(".flock").expect("selected lock suffix");
        match processes.symlink_metadata(format!("{id}.json")) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        match gripsack_fs::open_file_nofollow(&processes, Path::new(name)) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        if let Some(_exclusive) = gripsack_fs::FlockGuard::try_acquire_in(&processes, id)? {
            gripsack_fs::remove_file(&processes, Path::new(name))?;
            gripsack_fs::fsync_pinned_dir(&processes, Path::new("processes"))?;
        }
    }
    Ok(result)
}
