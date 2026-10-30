//! Captured declarations and receipt-bound native inputs, never evaluator state.
use super::{JobId, JobRevision};
use crate::{
    ExecError,
    workspace::{artifact, realize::Realization},
};
use gripsack_ir::{
    Ir,
    workspace_model::{WorkspaceCalendar, identity::PackageDigest},
};
use gripsack_process::{ProgramIdentity, Sha256Digest};
use gripsack_store as store;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

pub(super) const MAX_CAPTURE_BYTES: u64 = 16 * 1024 * 1024;
pub(super) const CAPTURE_VERSION: u32 = 1;

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    pub version: u32,
    pub job: JobId,
    pub task: String,
    pub trigger: WorkspaceCalendar,
    pub runner: PathBuf,
    pub unit_directory: PathBuf,
    pub checkout: PathBuf,
    pub ir: Ir,
    pub programs: Vec<Vec<ProgramIdentity>>,
    pub packages: BTreeMap<String, PackageDigest>,
    pub receipts: BTreeMap<PackageDigest, Sha256Digest>,
    pub inputs: BTreeMap<String, PathBuf>,
    pub roots: BTreeMap<PathBuf, store::hash::PayloadHash>,
}
impl Snapshot {
    pub fn realization(&self, home: &Path) -> Result<Realization<'_>, ExecError> {
        let mut admission = crate::workspace::sharing::ReadmissionScope::new(
            home,
            gripsack_fetch::FetchLimits::default(),
        );
        let mut restoration =
            artifact::hook::ReceiptRestoration::new(&self.receipts, &mut admission);
        let packages = self
            .packages
            .iter()
            .map(|(name, identity)| {
                restoration
                    .restore(*identity)
                    .map(|package| (name.as_str(), package))
            })
            .collect::<Result<_, _>>()?;
        Ok(Realization {
            packages,
            inputs: self
                .inputs
                .iter()
                .map(|(name, path)| (name.as_str(), path.clone()))
                .collect(),
        })
    }
    pub fn encode(&self) -> Result<Vec<u8>, ExecError> {
        // A bounded writer prevents a hostile declaration from allocating an
        // arbitrarily large serialized capture before the size check.
        struct Encoder(Vec<u8>);
        impl std::io::Write for Encoder {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_CAPTURE_BYTES as usize - self.0.len() {
                    return Err(std::io::Error::other(
                        "scheduled capture exceeds byte budget",
                    ));
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut encoder = Encoder(Vec::new());
        serde_json::to_writer(&mut encoder, self)?;
        Ok(encoder.0)
    }
}

pub(crate) fn inspect_capture(
    home: &Path,
    path: &Path,
    job: &JobId,
    revision: &JobRevision,
) -> Result<serde_json::Value, ExecError> {
    Ok(serde_json::to_value(read_metadata(
        home, path, job, revision,
    )?)?)
}

pub(super) fn read(
    home: &Path,
    path: &Path,
    job: &JobId,
    revision: &JobRevision,
) -> Result<Snapshot, ExecError> {
    let snapshot = read_metadata(home, path, job, revision)?;
    for (root, digest) in &snapshot.roots {
        if store::canonical_tree_hash(root)? != *digest {
            return Err(invalid("scheduled retained input identity differs"));
        }
    }
    Ok(snapshot)
}

fn read_metadata(
    home: &Path,
    path: &Path,
    job: &JobId,
    revision: &JobRevision,
) -> Result<Snapshot, ExecError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("capture has no retained root"))?;
    store::paths::validate_store_root(home, parent)?;
    let directory = artifact::retained_directory(home, parent)?
        .ok_or_else(|| invalid("scheduled capture root is missing"))?;
    let name = path
        .file_name()
        .ok_or_else(|| invalid("capture has no basename"))?;
    let mut file = gripsack_fs::open_file_nofollow(&directory, Path::new(name))?;
    if file.metadata()?.len() > MAX_CAPTURE_BYTES {
        return Err(invalid("scheduled capture exceeds byte budget"));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_CAPTURE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CAPTURE_BYTES
        || store::hash::hex_sha256(&bytes) != revision.as_str()
    {
        return Err(invalid("scheduled capture revision differs"));
    }
    let snapshot: Snapshot = serde_json::from_slice(&bytes)?;
    if snapshot.version != CAPTURE_VERSION
        || &snapshot.job != job
        || !snapshot.checkout.is_absolute()
    {
        return Err(invalid("scheduled capture protocol or identity differs"));
    }
    for root in snapshot.roots.keys() {
        store::paths::validate_store_root(home, root)?;
    }
    for path in snapshot
        .inputs
        .values()
        .chain(std::iter::once(&snapshot.runner))
    {
        if !snapshot.roots.keys().any(|root| path.starts_with(root)) {
            return Err(invalid(
                "scheduled input lies outside captured retention closure",
            ));
        }
    }
    Ok(snapshot)
}
fn invalid(message: &str) -> ExecError {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}
