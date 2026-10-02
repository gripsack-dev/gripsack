//! Generations on disk: the manifest of what each generation deployed,
//! and the flip (plan/0001 §3.5, 0008 §3).
//!
//! ```text
//! generations/
//! ├── 1/manifest.json     {number, modules: {name: {store_path, entries[]}}}
//! ├── 2/manifest.json
//! └── ...
//! current -> generations/2
//! ```

mod admission;
mod current;
mod environment;
pub use environment::{EnvironmentContribution, StructuredEnvironment};
mod inventory;
mod publication;
mod selection;
pub use current::{CommittedSelection, current, current_in, current_selection_in, flip};
pub(crate) use inventory::admit_manifest_at;
pub use inventory::{GenerationDirectory, admit_manifest, list, read_manifest};
pub use publication::{allocate, publish_generation, write_manifest};
pub(crate) use selection::{
    SelectionReservation, parse as parse_selection, reserve as reserve_selection,
};

use crate::GenerationId;
use crate::prior::Prior;
use gripsack_ir::Ownership;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One deployed file: where it went, how, and its canonical hash at
/// deploy time (drift detection compares against this — 0008 §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeployedEntry {
    /// The store-relative payload key (expanded; no placeholders).
    pub from: std::path::PathBuf,
    /// The DECLARED spelling (diagnostics/provenance only — never an
    /// ownership identity, 0035 F1).
    pub to: String,
    /// The canonical physical destination (0030 §P0-1) — THE
    /// ownership key: prune, rollback, lineage, and why-owns compare
    /// this, never the spelling. None only in pre-0.32 manifests;
    /// read-time upgrade canonicalizes from the spelling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<std::path::PathBuf>,
    #[serde(rename = "mode")]
    pub ownership: crate::ownership::StoredOwnership,
    /// Template vars at deploy time — rollback re-renders with these.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub vars: std::collections::BTreeMap<String, String>,
    /// What gripsack last WROTE — or, when `preserved_drift` is set,
    /// what it OBSERVED (0029 §2: one field used to mean both, and
    /// observed user bytes became overwrite authority on the next
    /// apply).
    /// Modal by ownership mode; constructible only from the typed
    /// producers (0035).
    pub hash: crate::hash::ManifestHash,
    /// The landed permission mode; merge records its hosting-file mode.
    /// Rollback restores it exactly. Older receipts may omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_mode: Option<u32>,
    /// Source executability at deployment (0041). Content-only updates
    /// retain acquired permissions; a source execute-bit change is explicit.
    /// Absence identifies legacy copy receipts hashed with the nominal source
    /// mode, even when takeover preserved different destination permissions.
    /// A recorded source bit also detects artifact executability tampering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_executable: Option<bool>,
    /// Pre-take-over state of this destination (0015 §4) — carried
    /// forward across EVERY generation of the ownership epoch (0029
    /// §1): an origin is forgotten only by a successful restore or an
    /// explicit forget, never by an ordinary later apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior: Option<Prior>,
    /// True when gripsack preserved user bytes instead of deploying
    /// (0029 §2): such an entry authorizes NOTHING — apply re-evaluates
    /// the drift fresh, prune and rollback never touch the file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preserved_drift: bool,
}

impl DeployedEntry {
    /// The ownership key: the recorded canonical key, or the spelling
    /// canonicalized on read (pre-0.32 manifests — the read-time
    /// upgrade; the next apply rewrites the record with a key).
    /// The ownership key as a path.
    pub fn key(&self) -> std::path::PathBuf {
        match &self.key {
            Some(k) => k.clone(),
            None => crate::paths::canonical_dest(&self.to)
                .unwrap_or_else(|_| std::path::PathBuf::from(&self.to)),
        }
    }

    pub fn merge_owner<'a>(&'a self, module: &'a str) -> &'a str {
        self.ownership
            .block_id()
            .map_or(module, crate::ownership::ManagedBlockId::as_str)
    }

    pub fn ownership_key(&self, module: &str) -> crate::ownership::OwnershipKey {
        crate::ownership::OwnershipKey::new(
            self.key(),
            (self.ownership.policy() == Ownership::Merge).then(|| self.merge_owner(module)),
        )
    }

    /// Compare a whole-file receipt, including pre-0043 template receipts.
    /// Legacy bytes-only hashes authorize nothing without a matching recorded
    /// mode. New writes always record the full file identity.
    pub fn matches_file(&self, bytes: &[u8], mode: u32) -> bool {
        !self.preserved_drift
            && (crate::canonical_bytes_identity(bytes, mode).as_str() == self.hash.as_str()
                || (self.ownership.policy() == Ownership::Template
                    && self.file_mode == Some(mode)
                    && crate::canonical_bytes_hash(bytes).as_str() == self.hash.as_str()))
    }
}

/// A module's recorded activation intent (0035 F9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentRecord {
    pub action: gripsack_ir::Action,
    pub trigger: gripsack_ir::Trigger,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleState {
    pub store_path: PathBuf,
    /// A store-only module: retain payload receipts, never destinations,
    /// activation intents, or shell-profile exports (0039).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub build_only: bool,
    #[serde(default)]
    pub entries: Vec<DeployedEntry>,
    /// The module's declared activation intents (0035 F9): the record
    /// lets the next generation fire on_remove hooks for modules that
    /// disappeared — without it the removal trigger is unknowable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intents: Vec<IntentRecord>,
    /// The verification receipt (0035 F2): fingerprints of the verify
    /// specs that PASSED over exactly this module state. Store
    /// presence is not a receipt; a failed run never writes one;
    /// changing a verifier invalidates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<Vec<String>>,
    /// Environment contributions, replayed into the shell profile at
    /// activation and rollback (0001 §3.10).
    #[serde(default)]
    pub env: Vec<EnvironmentContribution>,
    /// Content identity of the published tree (0014): present for
    /// content-addressed modules — store verify compares the live tree
    /// against this, no lockfile lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree256: Option<String>,
    /// Transitive build dependencies used by this module, in graph order.
    /// Retained generations keep these artifacts alive for history/rollback.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_closure: Vec<PathBuf>,
}

/// A generation: an immutable record of one profile state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Generation {
    #[serde(with = "crate::generation_wire")]
    pub number: GenerationId,
    pub modules: BTreeMap<String, ModuleState>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, path::Path};

    fn mk_gen(home: &Path, n: u64) -> Generation {
        let mut modules = BTreeMap::new();
        modules.insert(
            "helix".to_string(),
            ModuleState {
                store_path: home.join("store/abc-helix"),
                build_only: false,
                intents: vec![],
                verified: None,
                entries: vec![DeployedEntry {
                    from: "config.toml".into(),
                    to: "~/.config/helix/config.toml".into(),
                    key: None,
                    ownership: crate::StoredOwnership::Legacy(Ownership::TrackedCopy),
                    vars: Default::default(),
                    hash: crate::hash::ManifestHash::from_raw("d".repeat(64)),
                    file_mode: None,
                    source_executable: None,
                    prior: None,
                    preserved_drift: false,
                }],
                env: vec![],
                tree256: None,
                build_closure: vec![],
            },
        );
        Generation {
            number: GenerationId::new(n),
            modules,
        }
    }

    #[test]
    fn manifest_roundtrip_and_listing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cap = gripsack_fs::open_or_create(home).unwrap();
        write_manifest(&cap, &mk_gen(home, 1)).unwrap();
        write_manifest(&cap, &mk_gen(home, 2)).unwrap();
        assert_eq!(
            &*list(home).unwrap(),
            &[GenerationId::new(1), GenerationId::new(2)]
        );
        let read = read_manifest(home, GenerationId::new(1)).unwrap();
        assert_eq!(
            read.modules["helix"].entries[0].hash.as_str(),
            "d".repeat(64)
        );
    }

    #[test]
    fn flip_and_current() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cap = gripsack_fs::open_or_create(home).unwrap();
        write_manifest(&cap, &mk_gen(home, 1)).unwrap();
        write_manifest(&cap, &mk_gen(home, 2)).unwrap();
        assert_eq!(current(home).unwrap(), None);
        let first = crate::journal::begin_run(
            &cap,
            home,
            None,
            GenerationId::new(1),
            crate::journal::RunOp::Apply,
        )
        .unwrap();
        let committed = flip(first).unwrap();
        crate::journal::commit_run(committed).unwrap();
        assert_eq!(current(home).unwrap(), Some(GenerationId::new(1)));
        let second = crate::journal::begin_run(
            &cap,
            home,
            Some(GenerationId::new(1)),
            GenerationId::new(2),
            crate::journal::RunOp::Apply,
        )
        .unwrap();
        let committed = flip(second).unwrap();
        crate::journal::commit_run(committed).unwrap();
        assert_eq!(current(home).unwrap(), Some(GenerationId::new(2)));
    }

    /// 0027 §4: the persisted-generation boundary rejects identity
    /// mismatch, duplicate destinations, malformed hashes, and store
    /// paths outside $GRIPSACK_HOME/store.
    #[test]
    fn manifests_are_strictly_validated() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cap = gripsack_fs::open_or_create(home).unwrap();

        // number ≠ directory: a manifest claiming 1, filed under 7
        write_manifest(&cap, &mk_gen(home, 1)).unwrap();
        let wrong = mk_gen(home, 1);
        // written by hand — publish's no-clobber guards directories,
        // this test is about CONTENT identity
        let bad = home.join("generations/7");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            bad.join("manifest.json"),
            serde_json::to_string_pretty(&wrong).unwrap(),
        )
        .unwrap();
        let err = read_manifest(home, GenerationId::new(7)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // duplicate destinations (case-folded)
        let mut dup = mk_gen(home, 3);
        let entry = dup.modules["helix"].entries[0].clone();
        dup.modules
            .get_mut("helix")
            .unwrap()
            .entries
            .push(DeployedEntry {
                to: "~/.config/HELIX/config.toml".into(),
                ..entry
            });
        let d = home.join("generations/3");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("manifest.json"),
            serde_json::to_string_pretty(&dup).unwrap(),
        )
        .unwrap();
        let err = read_manifest(home, GenerationId::new(3)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // store path outside the store
        let mut outside = mk_gen(home, 4);
        outside.modules.get_mut("helix").unwrap().store_path = PathBuf::from("/etc/passwd");
        let d = home.join("generations/4");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("manifest.json"),
            serde_json::to_string_pretty(&outside).unwrap(),
        )
        .unwrap();
        let err = read_manifest(home, GenerationId::new(4)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// 0027 §8/§9: publishing an existing generation fails no-clobber;
    /// allocation survives gc of the tip via the high-water mark.
    #[test]
    fn publish_is_atomic_and_allocation_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cap = gripsack_fs::open_or_create(home).unwrap();
        publish_generation(&cap, &mk_gen(home, 1), Some("export A=1"), home).unwrap();
        assert!(home.join("generations/1/manifest.json").exists());
        assert!(home.join("generations/1/env/profile.sh").exists());
        // no staging residue
        assert!(!home.join("generations/.staging-1").exists());
        // no-clobber
        let err = publish_generation(&cap, &mk_gen(home, 1), None, home).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        // high-water: gc the tip, allocation still moves forward
        publish_generation(&cap, &mk_gen(home, 2), None, home).unwrap();
        publish_generation(&cap, &mk_gen(home, 3), None, home).unwrap();
        std::fs::remove_dir_all(home.join("generations/2")).unwrap();
        std::fs::remove_dir_all(home.join("generations/3")).unwrap();
        assert_eq!(allocate(home, &cap).unwrap(), GenerationId::new(4));
    }
}
