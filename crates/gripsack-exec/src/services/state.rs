//! Durable local ownership intent. Records contain no resolved environment,
//! engine inspection payloads or application logs. External outcomes are observed.
use super::{ServiceError, failure};
use crate::{Ctx, LifecycleSession};
use gripsack_process::Sha256Digest;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
};

const DIRECTORY: &str = "services";
const RECORD_VERSION: u32 = 1;
const MAX_STATE_BYTES: u64 = 1024 * 1024;
pub(super) const MAX_SERVICES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceServiceId(Sha256Digest);
impl WorkspaceServiceId {
    pub fn for_context(ctx: &Ctx) -> Result<Self, ServiceError> {
        let home = std::fs::canonicalize(&ctx.home).unwrap_or_else(|_| ctx.home.clone());
        let repo = repository_path(ctx.repository.identity())?;
        Ok(Self(Sha256Digest::of(&serde_json::to_vec(&(
            "gripsack.services.v1",
            home,
            repo,
        ))?)))
    }
    pub fn as_string(&self) -> String {
        self.0.to_string()
    }
    pub fn resource(&self, kind: &str, name: &str) -> String {
        let member = Sha256Digest::of(name.as_bytes()).to_string();
        format!(
            "gripsack-{}-{kind}-{}",
            &self.as_string()[..24],
            &member[..20]
        )
    }
    fn path(&self) -> PathBuf {
        Path::new(DIRECTORY).join(format!("{}.json", self.0))
    }
}

// Cleanup names an existing ownership namespace, not executable source. Resolve
// the surviving ancestor if the checkout was removed; never create it or guess
// through a missing symlink/parent traversal.
fn repository_path(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                for name in missing.into_iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(ancestor)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "broken repository alias; use its original canonical path for service cleanup",
                    ));
                }
                let Some(std::path::Component::Normal(name)) = ancestor.components().next_back()
                else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "use the original absolute repository path for removed-checkout service cleanup",
                    ));
                };
                missing.push(name);
                ancestor = ancestor
                    .parent()
                    .expect("normal final component has a parent");
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::repository_path;
    #[test]
    fn removed_checkout_keeps_its_recorded_namespace() {
        let parent = tempfile::tempdir().unwrap();
        let checkout = parent.path().join("project with spaces");
        std::fs::create_dir(&checkout).unwrap();
        let before = repository_path(&checkout).unwrap();
        std::fs::remove_dir(&checkout).unwrap();
        assert_eq!(repository_path(&checkout).unwrap(), before);
        assert!(!checkout.exists());
    }
    #[test]
    fn dangling_alias_does_not_become_a_different_namespace() {
        let parent = tempfile::tempdir().unwrap();
        let alias = parent.path().join("alias");
        std::os::unix::fs::symlink(parent.path().join("gone"), &alias).unwrap();
        assert!(repository_path(&alias).is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ServiceRecord {
    pub name: String,
    pub revision: Sha256Digest,
    /// None means an authorized create may not have returned an outcome yet.
    pub container: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ServiceState {
    pub version: u32,
    pub workspace: WorkspaceServiceId,
    pub owner: Sha256Digest,
    pub engine: String,
    pub socket: PathBuf,
    pub services: Vec<ServiceRecord>,
}
impl ServiceState {
    pub fn new(
        workspace: WorkspaceServiceId,
        engine: &super::engine::Engine,
    ) -> Result<Self, ServiceError> {
        let mut entropy = [0u8; 32];
        getrandom::fill(&mut entropy).map_err(std::io::Error::other)?;
        Ok(Self {
            version: RECORD_VERSION,
            workspace,
            owner: Sha256Digest::of(&entropy),
            engine: engine.identity.clone(),
            socket: engine.socket.clone(),
            services: Vec::new(),
        })
    }
    pub fn read(ctx: &Ctx, id: &WorkspaceServiceId) -> Result<Option<Self>, ServiceError> {
        let home = match gripsack_fs::open(&ctx.home) {
            Ok(home) => home,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let file = match gripsack_fs::open_file_nofollow(&home, &id.path()) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(failure("service ownership record exceeds its byte bound"));
        }
        let state: Self = serde_json::from_slice(&bytes)?;
        let mut names = BTreeSet::new();
        if state.version != RECORD_VERSION
            || &state.workspace != id
            || state.engine.is_empty()
            || !state.socket.is_absolute()
            || state.services.len() > MAX_SERVICES
            || state.services.iter().any(|record| {
                record.name.is_empty()
                    || !names.insert(&record.name)
                    || record.container.as_ref().is_some_and(|id| {
                        id.len() != 64
                            || !id
                                .bytes()
                                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    })
            })
        {
            return Err(failure(
                "invalid service ownership record; no engine resources are authorized",
            ));
        }
        Ok(Some(state))
    }
    pub fn persist(&self, ctx: &Ctx, session: &LifecycleSession) -> Result<(), ServiceError> {
        if session.home() != ctx.home {
            return Err(failure(
                "service ownership publication has a foreign lifecycle session",
            ));
        }
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(failure("service ownership record exceeds its byte bound"));
        }
        let directory = ctx.home_dir()?;
        let path = self.workspace.path();
        drop(
            gripsack_fs::StagedFileWrite::with_mode(directory, &path, &bytes, 0o600)?
                .sync_file()?
                .publish()?
                .sync_parent()?,
        );
        Ok(())
    }
    pub fn admit_engine(&self, engine: &super::engine::Engine) -> Result<(), ServiceError> {
        if self.engine != engine.identity || self.socket != engine.socket {
            return Err(failure(
                "Docker endpoint/engine differs from the recorded owner; refusing to adopt resources from another engine",
            ));
        }
        Ok(())
    }
}
