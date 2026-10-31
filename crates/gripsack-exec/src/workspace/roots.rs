//! Consumer roots share the existing lifecycle/GC exclusion. Wire records are
//! bounded, capability-read and admitted before deletion planning. A vanished
//! supervisor is not proof that every descendant is dead: unconfirmed process
//! roots remain until an explicitly confirmed cleanup operation removes them.
use crate::{ExecError, LifecycleSession};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
};

const ROOT_VERSION: u32 = 2;
const MAX_ROOT_BYTES: u64 = 1024 * 1024;
const ROOT_DIRECTORY: &str = "roots";
const MAX_ROOT_ENTRIES: usize = 100_000;
const MAX_RETAINED_OBJECTS: usize = 100_000;
/// Private native command projections retain the historical directory spelling.
pub(super) const BUILD_STAGING_DIRECTORY: &str = "workspace-staging";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootKind {
    Project,
    Process,
    Output,
    Job,
}
impl RootKind {
    fn directory(self) -> &'static str {
        match self {
            Self::Project => "projects",
            Self::Process => "processes",
            Self::Output => "outputs",
            Self::Job => "jobs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RootId(String);
impl RootId {
    pub fn from_identity(bytes: &[u8]) -> Self {
        Self(gripsack_store::hash::hex_sha256(bytes))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for RootId {
    type Error = std::io::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        gripsack_process::Sha256Digest::parse(&value)?;
        Ok(Self(value))
    }
}
impl From<RootId> for String {
    fn from(value: RootId) -> Self {
        value.0
    }
}

/// Retire a private native invocation projection only after its caller has
/// established that no supervised process still needs these inputs.
pub(super) fn remove_native_staging(home: &gripsack_fs::Dir, id: &RootId) -> Result<(), ExecError> {
    let parent = match gripsack_fs::open_dir_nofollow(home, Path::new(BUILD_STAGING_DIRECTORY)) {
        Ok(parent) => parent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    match parent.remove_dir_all(id.as_str()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    gripsack_fs::fsync_pinned_dir(&parent, Path::new("."))?;
    Ok(())
}

// Historical build metadata is admitted only for conservative retention.
// These records cannot confer worker control or authorize cleanup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedWorkerScope {
    home: gripsack_process::Sha256Digest,
    owner: gripsack_process::Sha256Digest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedWorkerBinding {
    instance: gripsack_process::Sha256Digest,
    epoch: std::num::NonZeroU64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedWorker {
    scope: RetainedWorkerScope,
    binding: RetainedWorkerBinding,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedAttemptIdentity {
    #[serde(deserialize_with = "retained_session")]
    session: String,
    attempt: std::num::NonZeroU64,
    epoch: std::num::NonZeroU64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildAttempt {
    identity: RetainedAttemptIdentity,
    worker: Option<RetainedWorker>,
}

fn retained_session<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<String, D::Error> {
    let value = String::deserialize(decoder)?;
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(serde::de::Error::custom(
            "session must be 1..=64 ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(value)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootFile<I = RootId, P = BTreeSet<PathBuf>, B = BuildAttempt> {
    version: u32,
    kind: RootKind,
    id: I,
    store_paths: P,
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<B>,
}

/// Constructed only while publication and collection share the same lock.
/// The producer/consumer adapter supplies the full retention closure; the
/// concrete closure campaign checks that inventory obligation separately.
pub struct RetentionSet {
    home: PathBuf,
    paths: BTreeSet<PathBuf>,
}
impl RetentionSet {
    pub fn admit(
        session: &LifecycleSession,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, ExecError> {
        let paths: BTreeSet<_> = paths.into_iter().collect();
        for path in &paths {
            admit_store_path(session.home(), path)?;
        }
        Ok(Self {
            home: session.home().to_owned(),
            paths,
        })
    }
    pub fn paths(&self) -> &BTreeSet<PathBuf> {
        &self.paths
    }
}
fn error(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        output: "workspace".into(),
        step: "retention".into(),
        detail: detail.into(),
    }
}
fn admit_store_path(home: &Path, path: &Path) -> Result<(), ExecError> {
    gripsack_store::paths::validate_store_root(home, path)?;
    if path.to_str().is_none() {
        return Err(error("root object must name a UTF-8 store child"));
    }
    Ok(())
}
fn directory(
    home: &gripsack_fs::Dir,
    kind: RootKind,
    create: bool,
) -> Result<Option<gripsack_fs::Dir>, ExecError> {
    if create {
        gripsack_fs::create_dir_all(home, Path::new(ROOT_DIRECTORY))?;
    }
    let root = match gripsack_fs::open_dir_nofollow(home, Path::new(ROOT_DIRECTORY)) {
        Ok(root) => root,
        Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if create {
        gripsack_fs::create_dir_all(&root, Path::new(kind.directory()))?;
    }
    match gripsack_fs::open_dir_nofollow(&root, Path::new(kind.directory())) {
        Ok(directory) => Ok(Some(directory)),
        Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn write_root(
    session: &LifecycleSession,
    kind: RootKind,
    id: &RootId,
    closure: &RetentionSet,
    build: Option<&BuildAttempt>,
) -> Result<(), ExecError> {
    if closure.home != session.home() {
        return Err(error("retention authority belongs to another home"));
    }
    let home = gripsack_fs::open_or_create(session.home())?;
    let dir =
        directory(&home, kind, true)?.ok_or_else(|| error("created root directory is absent"))?;
    let record = RootFile {
        version: ROOT_VERSION,
        kind,
        id,
        store_paths: &closure.paths,
        build,
    };
    let bytes = serde_json::to_vec(&record)?;
    if bytes.len() as u64 > MAX_ROOT_BYTES {
        return Err(error("root record exceeds its byte bound"));
    }
    gripsack_fs::atomic_write(&dir, Path::new(&format!("{}.json", id.as_str())), &bytes)?;
    Ok(())
}
pub fn project_id(project: &Path) -> Result<RootId, ExecError> {
    let canonical = project.canonicalize()?;
    Ok(RootId::from_identity(
        canonical.as_os_str().as_encoded_bytes(),
    ))
}
pub fn register_project_root(
    session: &LifecycleSession,
    id: &RootId,
    closure: &RetentionSet,
) -> Result<(), ExecError> {
    write_root(session, RootKind::Project, id, closure, None)
}
pub fn register_output_root(
    session: &LifecycleSession,
    id: &RootId,
    closure: &RetentionSet,
) -> Result<(), ExecError> {
    write_root(session, RootKind::Output, id, closure, None)
}
fn remove_root(session: &LifecycleSession, kind: RootKind, id: &RootId) -> Result<(), ExecError> {
    let home = gripsack_fs::open_or_create(session.home())?;
    if let Some(directory) = directory(&home, kind, false)? {
        for suffix in if kind == RootKind::Process {
            &["json", "flock"][..]
        } else {
            &["json"][..]
        } {
            match gripsack_fs::remove_file(
                &directory,
                Path::new(&format!("{}.{suffix}", id.as_str())),
            ) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        gripsack_fs::fsync_pinned_dir(&directory, Path::new(kind.directory()))?;
    }
    Ok(())
}

pub struct ProcessLease {
    home: PathBuf,
    id: RootId,
    lock: gripsack_fs::FlockGuard,
}
impl ProcessLease {
    pub fn register(session: &LifecycleSession, closure: &RetentionSet) -> Result<Self, ExecError> {
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce).map_err(std::io::Error::other)?;
        let id = RootId::from_identity(&nonce);
        let home = gripsack_fs::open_or_create(session.home())?;
        let directory = directory(&home, RootKind::Process, true)?
            .ok_or_else(|| error("created process root directory is absent"))?;
        let lock = gripsack_fs::FlockGuard::try_acquire_in(&directory, id.as_str())?
            .ok_or_else(|| error("new process root identity is already leased"))?;
        write_root(session, RootKind::Process, &id, closure, None)?;
        Ok(Self {
            home: session.home().to_owned(),
            id,
            lock,
        })
    }
    pub fn id(&self) -> &RootId {
        &self.id
    }
    pub fn duplicate_handle(&self) -> std::io::Result<std::fs::File> {
        self.lock.duplicate_handle()
    }
    /// Release an invocation that launched no subprocess. Artifact-only tasks
    /// have no process receipt to manufacture. Callers must not use this after
    /// handing a duplicate to process supervision, including failed launches.
    pub fn release_unlaunched(self, session: &LifecycleSession) -> Result<(), ExecError> {
        if self.home != session.home() {
            return Err(error(
                "unlaunched release requires this home's native invocation lease",
            ));
        }
        remove_root(session, RootKind::Process, &self.id)
    }
    /// Only the confirmed native completion path calls this. Drop alone leaves
    /// evidence intact, including after panic, crash or unknown cleanup.
    pub fn release(
        self,
        session: &LifecycleSession,
        receipt: &gripsack_process::ProcessReceipt,
    ) -> Result<(), ExecError> {
        if self.home != session.home() {
            return Err(error("process root belongs to another home"));
        }
        if receipt.disposition == gripsack_process::ProcessDisposition::CleanupFailure {
            return Err(error(
                "native cleanup is unconfirmed; process retention remains",
            ));
        }
        remove_root(session, RootKind::Process, &self.id)
    }
}

/// Called by GC using its already pinned home capability before any mutation.
pub(crate) fn inventory(
    home: &gripsack_fs::Dir,
    home_path: &Path,
) -> Result<BTreeSet<String>, ExecError> {
    let mut paths = BTreeSet::new();
    let roots = match gripsack_fs::open_dir_nofollow(home, Path::new(ROOT_DIRECTORY)) {
        Ok(roots) => roots,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(paths),
        Err(error) => return Err(error.into()),
    };
    for entry in roots.entries()? {
        let name = entry?.file_name();
        if !["projects", "processes", "outputs", "jobs"]
            .iter()
            .any(|known| name == *known)
        {
            return Err(error("unknown namespace in authoritative consumer roots"));
        }
    }
    let mut entries = 0;
    for kind in [
        RootKind::Project,
        RootKind::Process,
        RootKind::Output,
        RootKind::Job,
    ] {
        let dir = match gripsack_fs::open_dir_nofollow(&roots, Path::new(kind.directory())) {
            Ok(dir) => dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in dir.entries()? {
            entries += 1;
            if entries > MAX_ROOT_ENTRIES {
                return Err(error(
                    "authoritative root inventory exceeds its entry bound",
                ));
            }
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| error("non-UTF-8 authoritative root name"))?;
            let Some(record) = read_record(&dir, name, kind, home_path)? else {
                continue;
            };
            for path in record.store_paths {
                let normalized = home_path
                    .join(gripsack_store::STORE_DIR)
                    .join(path.file_name().expect("admitted direct store child"));
                paths.insert(
                    normalized
                        .into_os_string()
                        .into_string()
                        .map_err(|_| error("non-UTF-8 store root"))?,
                );
                if paths.len() > MAX_RETAINED_OBJECTS {
                    return Err(error("consumer retention closure exceeds its object bound"));
                }
            }
        }
    }
    Ok(paths)
}

fn read_record(
    directory: &gripsack_fs::Dir,
    name: &str,
    kind: RootKind,
    home: &Path,
) -> Result<Option<RootFile>, ExecError> {
    if let Some(id) = name.strip_suffix(".flock") {
        RootId::try_from(id.to_owned())?;
        gripsack_fs::open_file_nofollow(directory, Path::new(name))?;
        return Ok(None);
    }
    let id = name
        .strip_suffix(".json")
        .ok_or_else(|| error("unknown entry in authoritative root directory"))?;
    let expected = RootId::try_from(id.to_owned())?;
    let file = gripsack_fs::open_file_nofollow(directory, Path::new(name))?;
    if file.metadata()?.len() > MAX_ROOT_BYTES {
        return Err(error("root record exceeds its byte bound"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_ROOT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ROOT_BYTES {
        return Err(error("root record grew beyond its byte bound"));
    }
    let record: RootFile = serde_json::from_slice(&bytes)?;
    if record.version != ROOT_VERSION || record.kind != kind || record.id != expected {
        return Err(error("root record version/namespace/identity mismatch"));
    }
    if record.build.is_some() && kind != RootKind::Process {
        return Err(error("non-process root contains builder authority"));
    }
    for path in &record.store_paths {
        admit_store_path(home, path)?;
    }
    Ok(Some(record))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn historical_build_roots_preserve_strict_metadata_admission() {
        let digest = "a".repeat(64);
        let build = json!({
            "identity": {"session": "old-session_1", "attempt": 1, "epoch": 2},
            "worker": {
                "scope": {"home": digest, "owner": digest},
                "binding": {"instance": digest, "epoch": 3}
            }
        });
        let admitted: BuildAttempt = serde_json::from_value(build.clone()).unwrap();
        assert_eq!(serde_json::to_value(admitted).unwrap(), build);

        for (path, invalid) in [
            ("/identity/session", json!("../escape")),
            ("/identity/attempt", json!(0)),
            ("/identity/epoch", json!(0)),
            ("/worker/binding/epoch", json!(0)),
            ("/worker/binding/instance", json!("not-a-digest")),
            ("/worker/scope/home", json!("A".repeat(64))),
        ] {
            let mut corrupt = build.clone();
            *corrupt.pointer_mut(path).unwrap() = invalid;
            assert!(
                serde_json::from_value::<BuildAttempt>(corrupt).is_err(),
                "{path}"
            );
        }
        let mut unknown = build;
        unknown["worker"]["control"] = json!("start");
        assert!(serde_json::from_value::<BuildAttempt>(unknown).is_err());
    }
}
