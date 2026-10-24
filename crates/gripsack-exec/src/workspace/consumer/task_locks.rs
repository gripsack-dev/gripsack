//! Canonical all-or-release mutation admission. Each persistent claim names
//! the supervised process-root lease, not a PID, timestamp or task revision.
//! Drop releases kernel handles, NEVER durable claims: a crash/unknown cleanup
//! must continue excluding conflicts even after the supervisor has disappeared.
use super::{failure, task_report::InvocationId};
use crate::ExecError;
use gripsack_ir::workspace_model::{MutationLockScope, WorkspaceMutationLock};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) enum LockConflict {
    Held,
    Unknown,
}
struct ClaimedKey {
    directory: gripsack_fs::Dir,
    name: String,
    _guard: gripsack_fs::FlockGuard,
}
pub(super) struct MutationClaims {
    keys: Vec<ClaimedKey>,
}
impl MutationClaims {
    pub fn acquire(
        home: &Path,
        locks: &[WorkspaceMutationLock],
        invocation: &InvocationId,
        process_root: &str,
    ) -> Result<Result<Self, LockConflict>, ExecError> {
        let user = if locks
            .iter()
            .any(|lock| lock.scope == MutationLockScope::User)
        {
            Some(user_directory()?)
        } else {
            None
        };
        Self::acquire_at(home, user.as_deref(), locks, invocation, process_root)
    }
    fn acquire_at(
        home: &Path,
        user: Option<&Path>,
        locks: &[WorkspaceMutationLock],
        invocation: &InvocationId,
        process_root: &str,
    ) -> Result<Result<Self, LockConflict>, ExecError> {
        let mut ordered = BTreeMap::new();
        for lock in locks {
            let directory = match lock.scope {
                MutationLockScope::Store => home.join("task-mutation-locks"),
                MutationLockScope::User => user
                    .ok_or_else(|| failure("missing user lock authority"))?
                    .to_owned(),
            };
            ordered.insert((lock.scope, lock.key.as_str()), directory);
        }
        let mut keys = Vec::with_capacity(ordered.len());
        for ((scope, key), path) in ordered {
            let scope = match scope {
                MutationLockScope::Store => "store",
                MutationLockScope::User => "user",
            };
            let name = format!(
                "{scope}-{}",
                gripsack_store::hash::hex_sha256(key.as_bytes())
            );
            let directory = gripsack_fs::open_or_create(&path)?;
            let guard = gripsack_fs::FlockGuard::try_acquire_in(&directory, &name)?;
            let held = guard.is_none();
            let unknown = match directory.symlink_metadata(format!("{name}.json")) {
                Ok(_) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error.into()),
            };
            if !gripsack_policy::task::admit_conflict(held, unknown) {
                return Ok(Err(if held {
                    LockConflict::Held
                } else {
                    LockConflict::Unknown
                }));
            }
            keys.push(ClaimedKey {
                directory,
                name,
                _guard: guard.expect("admitted unlocked key"),
            });
        }
        // No effects were launched while taking a prefix of the lock set.
        // Publish claims only after every key is held; rollback this invocation's
        // published prefix on an IO failure. Pre-existing claims are untouched.
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 1, "invocation": invocation, "store": home,
            "process_root": process_root,
        }))?;
        for (published, key) in keys.iter().enumerate() {
            if let Err(error) = gripsack_fs::atomic_write_with_mode(
                &key.directory,
                Path::new(&format!("{}.json", key.name)),
                &bytes,
                0o600,
            ) {
                // Publication may have happened before a failed directory
                // fsync. We own every key and observed no previous record.
                for previous in &keys[..=published] {
                    match gripsack_fs::remove_file(
                        &previous.directory,
                        Path::new(&format!("{}.json", previous.name)),
                    ) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                    gripsack_fs::fsync_pinned_dir(&previous.directory, Path::new("."))?;
                }
                return Err(error.into());
            }
        }
        Ok(Ok(Self { keys }))
    }
    /// Caller has confirmed every launched child's supervised cleanup (or
    /// knows no child was launched). Unknown outcomes must never call this.
    pub fn release(self) -> Result<(), ExecError> {
        for key in &self.keys {
            remove_claim(key)?;
        }
        Ok(())
    }
}
fn remove_claim(key: &ClaimedKey) -> Result<(), ExecError> {
    gripsack_fs::remove_file(&key.directory, Path::new(&format!("{}.json", key.name)))?;
    gripsack_fs::fsync_pinned_dir(&key.directory, Path::new("."))?;
    Ok(())
}
fn user_directory() -> Result<PathBuf, ExecError> {
    // HOME/XDG overrides cannot split one Unix user's exclusion namespace.
    // The account database, not repository command env, identifies this scope.
    let mut storage = vec![0u8; 64 * 1024];
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            &mut record,
            storage.as_mut_ptr().cast(),
            storage.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || record.pw_dir.is_null() {
        return Err(failure(
            "cannot resolve user mutation-lock namespace from account database",
        ));
    }
    use std::os::unix::ffi::OsStrExt;
    let bytes = unsafe { std::ffi::CStr::from_ptr(record.pw_dir) }.to_bytes();
    let home = Path::new(std::ffi::OsStr::from_bytes(bytes));
    if !home.is_absolute() {
        return Err(failure("user mutation-lock home must be absolute"));
    }
    Ok(home.join(".local/state/gripsack/task-mutation-locks"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::Span;
    fn lock(key: &str) -> WorkspaceMutationLock {
        WorkspaceMutationLock {
            scope: MutationLockScope::Store,
            key: key.into(),
            span: Span {
                file: "tasks.ts".into(),
                line: 1,
                col: None,
            },
        }
    }
    #[test]
    fn complete_set_rolls_back_and_unknown_claims_never_age_out() {
        let temp = tempfile::tempdir().unwrap();
        let id = InvocationId::parse(&"a".repeat(64)).unwrap();
        // Canonical declared key order encounters "a" before held "z",
        // regardless of request order; a failed prefix must be released.
        let held = MutationClaims::acquire_at(temp.path(), None, &[lock("z")], &id, "root")
            .unwrap()
            .ok()
            .unwrap();
        assert!(matches!(
            MutationClaims::acquire_at(temp.path(), None, &[lock("z"), lock("a")], &id, "other")
                .unwrap(),
            Err(LockConflict::Held)
        ));
        let independent = MutationClaims::acquire_at(temp.path(), None, &[lock("a")], &id, "third")
            .unwrap()
            .ok()
            .unwrap();
        independent.release().unwrap();
        drop(held); // supervisor death: kernel lock gone, durable protection stays.
        assert!(matches!(
            MutationClaims::acquire_at(temp.path(), None, &[lock("z")], &id, "retry").unwrap(),
            Err(LockConflict::Unknown)
        ));
    }
    #[test]
    fn same_key_deduplicates_and_distinct_scopes_do_not_alias() {
        let store = tempfile::tempdir().unwrap();
        // An explicitly colocated store cannot collapse the two namespaces.
        let user = store.path().join("task-mutation-locks");
        let id = InvocationId::parse(&"b".repeat(64)).unwrap();
        let held = MutationClaims::acquire_at(
            store.path(),
            Some(&user),
            &[lock("x"), lock("x")],
            &id,
            "root",
        )
        .unwrap()
        .ok()
        .unwrap();
        let mut scoped = lock("x");
        scoped.scope = MutationLockScope::User;
        MutationClaims::acquire_at(store.path(), Some(&user), &[scoped], &id, "other")
            .unwrap()
            .ok()
            .unwrap()
            .release()
            .unwrap();
        held.release().unwrap();
        MutationClaims::acquire_at(store.path(), None, &[lock("x")], &id, "again")
            .unwrap()
            .ok()
            .unwrap()
            .release()
            .unwrap();
    }
}
