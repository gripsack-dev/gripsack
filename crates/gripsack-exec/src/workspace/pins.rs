//! Portable workspace resolution state. Frozen acquisition reads the approved
//! repository snapshot; only explicit update publishes to the mutable repository.
//! A shared repository lock plus snapshot comparison prevents two homes from
//! silently replacing each other's platform resolutions.
use crate::{Ctx, ExecError, LifecycleSession, Repository};
use gripsack_ir::{
    workspace::WorkspacePlatform,
    workspace_v6::{
        LockedSource,
        lock::{DefinitionPins, LockedPin, WORKSPACE_LOCK_VERSION, WorkspaceLock, platform_key},
    },
};
use gripsack_process::Sha256Digest;
use std::{collections::BTreeMap, io::Read, path::Path};

const WORKSPACE_LOCK: &str = "gripsack.lock";
const MAX_WORKSPACE_LOCK_BYTES: u64 = 16 * 1024 * 1024;

pub(super) struct WorkspacePins {
    pub lock: Option<WorkspaceLock>,
    captured_bytes: Option<Sha256Digest>,
}
impl WorkspacePins {
    pub fn read(repository: &Repository) -> Result<Self, ExecError> {
        let directory = gripsack_fs::open(repository.contents())?;
        let Some(bytes) = read_bytes(&directory)? else {
            return Ok(Self {
                lock: None,
                captured_bytes: None,
            });
        };
        let text =
            std::str::from_utf8(&bytes).map_err(|_| failure("workspace", "lock is not UTF-8"))?;
        let lock = WorkspaceLock::from_json(text)
            .map_err(|error| failure("workspace", error.to_string()))?;
        Ok(Self {
            lock: Some(lock),
            captured_bytes: Some(Sha256Digest::of(&bytes)),
        })
    }

    pub fn admit_definitions(&self, definitions: &DefinitionPins) -> Result<(), ExecError> {
        if self
            .lock
            .as_ref()
            .is_some_and(|lock| &lock.definitions != definitions)
        {
            return Err(failure(
                "workspace",
                "captured frontend/import pins changed; run grip update explicitly",
            ));
        }
        Ok(())
    }

    /// Absence is distinct from corruption or changed intent. The acquisition
    /// owner permits an absent pin only for an already captured repository input.
    pub fn lookup(
        &self,
        platform: &WorkspacePlatform,
        output: &str,
        source: &LockedSource,
    ) -> Result<Option<&LockedPin>, ExecError> {
        let key = platform_key(platform);
        let pin = self
            .lock
            .as_ref()
            .and_then(|lock| lock.resolutions.get(&key))
            .and_then(|resolution| resolution.pins.iter().find(|pin| pin.output == output));
        if pin.is_some_and(|pin| !same_source_declaration(&pin.source, source)) {
            return Err(failure(
                output,
                "declared source differs from its lock; frozen acquisition cannot re-resolve it",
            ));
        }
        Ok(pin)
    }

    pub fn take_for_update(&mut self, definitions: DefinitionPins) -> WorkspaceLock {
        match self.lock.take() {
            Some(mut lock) => {
                lock.definitions = definitions;
                lock
            }
            None => WorkspaceLock {
                lock_version: WORKSPACE_LOCK_VERSION,
                definitions,
                resolutions: BTreeMap::new(),
            },
        }
    }

    pub fn publish(
        self,
        ctx: &Ctx,
        session: &LifecycleSession,
        mut lock: WorkspaceLock,
    ) -> Result<(), ExecError> {
        if session.home() != ctx.home {
            return Err(failure(
                "workspace",
                "lock publication has another home's lifecycle authority",
            ));
        }
        lock.validate()
            .map_err(|error| failure("workspace", error.to_string()))?;
        for resolution in lock.resolutions.values_mut() {
            resolution
                .pins
                .sort_unstable_by(|left, right| left.output.cmp(&right.output));
            resolution
                .transitive
                .sort_unstable_by(|left, right| left.name.cmp(&right.name));
        }
        let bytes = serde_json::to_vec_pretty(&lock)?;
        if bytes.len() as u64 > MAX_WORKSPACE_LOCK_BYTES {
            return Err(failure(
                "workspace",
                "updated lock exceeds its metadata byte bound",
            ));
        }
        let repository = gripsack_fs::open(ctx.repository.identity())?;
        gripsack_fs::create_dir_all(&repository, Path::new("locks"))?;
        let locks = gripsack_fs::open_dir_nofollow(&repository, Path::new("locks"))?;
        let _guard = gripsack_fs::FlockGuard::try_acquire_in(&locks, "workspace-write")?
            .ok_or_else(|| {
                failure(
                    "workspace",
                    "another workspace update owns lock publication",
                )
            })?;
        let current = read_bytes(&repository)?.as_deref().map(Sha256Digest::of);
        if current != self.captured_bytes {
            return Err(failure(
                "workspace",
                "lock changed after source capture; refusing to overwrite another update",
            ));
        }
        gripsack_fs::atomic_write(&repository, Path::new(WORKSPACE_LOCK), &bytes)?;
        Ok(())
    }
}

/// Captured Pixi input identities are resolution evidence, not declaration
/// fields. Frozen preparation and read-only inspection verify both inputs
/// separately before using the locked closure.
fn same_source_declaration(locked: &LockedSource, declared: &LockedSource) -> bool {
    match (locked, declared) {
        (LockedSource::PixiLock(locked), LockedSource::PixiLock(declared)) => {
            locked.manifest == declared.manifest
                && locked.lock == declared.lock
                && locked.environment == declared.environment
        }
        _ => locked == declared,
    }
}

fn read_bytes(directory: &gripsack_fs::Dir) -> Result<Option<Vec<u8>>, ExecError> {
    let file = match gripsack_fs::open_file_nofollow(directory, Path::new(WORKSPACE_LOCK)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() > MAX_WORKSPACE_LOCK_BYTES {
        return Err(failure("workspace", "lock exceeds its metadata byte bound"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_WORKSPACE_LOCK_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_WORKSPACE_LOCK_BYTES {
        return Err(failure(
            "workspace",
            "lock grew beyond its metadata byte bound",
        ));
    }
    Ok(Some(bytes))
}
fn failure(output: &str, detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        module: output.to_owned(),
        step: WORKSPACE_LOCK.into(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
