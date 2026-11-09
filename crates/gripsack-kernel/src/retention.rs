//! GC retention as a pure kernel (0046 §5.3, 0056 K4): admission, generation
//! pruning, the retained closure over admitted manifests, and the deletion
//! set over an admitted inventory. The GC machine (`gc.rs`) sequences these
//! decisions; this module never touches the filesystem.
//!
//! Proved (proofs/Kernel/Retention.lean, proofs/Kernel/Gc.lean):
//!
//! - nothing is planned while recovery or activation is pending, and a
//!   current generation missing from the inventory is refused
//!   (`admit_gc`'s biconditionals);
//! - pruned generations are the oldest excess of the inventory, never the
//!   current generation (`plan_prune`);
//! - a generation a retained manifest names as its predecessor is in the
//!   inventory and is retained (`closure`);
//! - the deletion set is exactly the candidates no root names — no path a
//!   retained manifest pins, no external root — by position and in order
//!   (`plan_delete`).
use crate::{GenerationId, GenerationList, generation::names_generation};
use alloc::vec::Vec;

/// GC admission (0045 F3): the destructive command's preconditions as a total
/// function, so a caller cannot forget a check.
pub enum GcAdmission {
    /// Inventory is sound and recovery is finished — plan away.
    Admitted,
    /// A run marker, journal entries, a quarantine or a pending activation
    /// exist: journaled prior blobs may be referenced by no manifest, so no
    /// deletion set is computable. Reconcile first.
    RecoveryPending,
    /// The current generation has no directory on disk — corruption;
    /// "cannot read" must never read as "nothing referenced".
    CorruptCurrent,
}

/// Admit or refuse destructive collection. Recovery state wins over inventory
/// shape: while recovery is pending, even a sound inventory proves nothing
/// about journal-only blobs.
pub fn admit_gc(
    recovery_pending: bool,
    current: Option<GenerationId>,
    inventory: &GenerationList,
) -> GcAdmission {
    if recovery_pending {
        return GcAdmission::RecoveryPending;
    }
    match current {
        Some(current) => {
            if contains_generation(inventory.as_slice(), current) {
                GcAdmission::Admitted
            } else {
                GcAdmission::CorruptCurrent
            }
        }
        None => GcAdmission::Admitted,
    }
}

/// Membership over generation identities.
pub fn contains_generation(haystack: &[GenerationId], needle: GenerationId) -> bool {
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i].same(needle) {
            return true;
        }
        i += 1;
    }
    false
}

/// Prune exactly the oldest excess prefix except current. Current protection
/// can retain one extra generation; it never shifts deletion to a newer one.
/// Inventory admission has already checked strict ascending order.
pub fn plan_prune(
    inventory: &GenerationList,
    current: Option<GenerationId>,
    keep: Option<usize>,
) -> Vec<GenerationId> {
    let generations = inventory.as_slice();
    let excess = match keep {
        Some(keep) if generations.len() > keep => generations.len() - keep,
        _ => 0,
    };
    let mut pruned: Vec<GenerationId> = Vec::new();
    let mut i = 0;
    while i < excess {
        let candidate = generations[i];
        if !names_generation(current, candidate) {
            pruned.push(candidate);
        }
        i += 1;
    }
    pruned
}

/// Byte-exact identity of two path identities.
pub fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Membership over path identities. Identities are owned byte vectors: a
/// slice of borrowed slices is a nested borrow, which the Aeneas
/// translation does not support.
pub fn contains_bytes(haystack: &[Vec<u8>], needle: &[u8]) -> bool {
    let mut i = 0;
    while i < haystack.len() {
        if same_bytes(&haystack[i], needle) {
            return true;
        }
        i += 1;
    }
    false
}

/// One generation's manifest as the host admitted it: what it pins.
pub struct ManifestFacts {
    pub id: GenerationId,
    /// The generations its directory-transition and migration receipts
    /// restore to: rolling back this generation needs them, so they live
    /// as long as it does.
    pub predecessors: Vec<GenerationId>,
    /// The paths it pins: deployed store payloads, their build closures and
    /// the prior blobs of its entries.
    pub roots: Vec<Vec<u8>>,
}

/// What collection does with a generation.
pub enum Fate {
    Retained,
    Retired,
}

impl Fate {
    pub fn retained(&self) -> bool {
        match self {
            Fate::Retained => true,
            Fate::Retired => false,
        }
    }
}

/// A read manifest and the fate the GC decided for its generation.
pub struct Manifest {
    pub facts: ManifestFacts,
    pub fate: Fate,
}

/// Which directory a collection candidate lives in.
pub enum Scope {
    /// `store/`: input-addressed payloads.
    Store,
    /// `prior/`: blobs of objects gripsack replaced.
    Prior,
    /// `generations/.staging-<N>-<pid>`: a publication that died before its
    /// rename. Nothing ever names one; under the lifecycle session no
    /// publisher is live.
    Staging,
}

/// The object the host inventoried at a candidate's name (filesystem and
/// inode). The kernel never interprets it; the driver re-checks it before
/// removing the name.
pub struct Stamp {
    pub filesystem: u64,
    pub inode: u64,
}

/// One entry of a collectable directory, as inventoried.
pub struct Candidate {
    pub scope: Scope,
    pub path: Vec<u8>,
    pub stamp: Stamp,
}

impl Candidate {
    pub fn path(&self) -> &[u8] {
        &self.path
    }
}

/// This manifest is retained and names `generation` as a predecessor.
pub fn needs(manifest: &Manifest, generation: GenerationId) -> bool {
    manifest.fate.retained() && contains_generation(&manifest.facts.predecessors, generation)
}

/// This manifest is retained and pins `path`.
pub fn pins(manifest: &Manifest, path: &[u8]) -> bool {
    manifest.fate.retained() && contains_bytes(&manifest.facts.roots, path)
}

/// Some retained manifest names `generation` as a predecessor.
pub fn required(manifests: &[Manifest], generation: GenerationId) -> bool {
    let mut i = 0;
    while i < manifests.len() {
        if needs(&manifests[i], generation) {
            return true;
        }
        i += 1;
    }
    false
}

/// Every predecessor is in the inventory.
pub fn known(predecessors: &[GenerationId], inventory: &[GenerationId]) -> bool {
    let mut i = 0;
    while i < predecessors.len() {
        if !contains_generation(inventory, predecessors[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// Why the decided fates do not close.
pub enum Unclosed {
    /// A retained generation names a predecessor that is not in the
    /// inventory: its rollback would have nothing to restore to.
    MissingPredecessor { generation: GenerationId },
    /// A retired generation is a retained one's predecessor.
    Required { generation: GenerationId },
}

/// The retained set is closed under predecessors: every retained
/// generation's predecessors are in the inventory, and none is retired.
pub fn closure(manifests: &[Manifest], inventory: &[GenerationId]) -> Option<Unclosed> {
    let mut i = 0;
    while i < manifests.len() {
        if let Some(unclosed) = closes(manifests, &manifests[i], inventory) {
            return Some(unclosed);
        }
        i += 1;
    }
    None
}

/// One manifest's part of the closure: a retained one's predecessors are
/// in the inventory, a retired one is no retained one's predecessor.
pub fn closes(
    manifests: &[Manifest],
    manifest: &Manifest,
    inventory: &[GenerationId],
) -> Option<Unclosed> {
    let generation = manifest.facts.id;
    match manifest.fate {
        Fate::Retained => {
            if known(&manifest.facts.predecessors, inventory) {
                None
            } else {
                Some(Unclosed::MissingPredecessor { generation })
            }
        }
        Fate::Retired => {
            if required(manifests, generation) {
                Some(Unclosed::Required { generation })
            } else {
                None
            }
        }
    }
}

/// A retained manifest pins `path`.
pub fn pinned(manifests: &[Manifest], path: &[u8]) -> bool {
    let mut i = 0;
    while i < manifests.len() {
        if pins(&manifests[i], path) {
            return true;
        }
        i += 1;
    }
    false
}

/// `path` is a root: a retained manifest pins it, or the host's external
/// roots (workspace and schedule roots) name it.
pub fn rooted(manifests: &[Manifest], external: &[Vec<u8>], path: &[u8]) -> bool {
    pinned(manifests, path) || contains_bytes(external, path)
}

/// The deletion set: the positions of the candidates that no root names, in
/// order.
pub fn plan_delete(
    manifests: &[Manifest],
    external: &[Vec<u8>],
    candidates: &[Candidate],
) -> Vec<usize> {
    let mut deletions: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < candidates.len() {
        if !rooted(manifests, external, candidates[i].path()) {
            deletions.push(i);
        }
        i += 1;
    }
    deletions
}
