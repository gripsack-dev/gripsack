//! Generation identities and strictly ordered inventories. Serialization and
//! filesystem enumeration stay in the store; the kernel receives admitted
//! values.
//!
//! Proved (proofs/Kernel/Retention.lean): `GenerationList::new` admits a
//! vector exactly when its numbers are strictly ascending (hence
//! duplicate-free).
use alloc::vec::Vec;

pub struct GenerationId {
    number: u64,
}

impl GenerationId {
    /// Every wire u64 is an identity, including retained historical zero IDs.
    pub fn new(number: u64) -> Self {
        Self { number }
    }

    /// Numeric representation for wire encoding and inventory ordering only.
    pub fn value(self) -> u64 {
        self.number
    }

    /// Exact identity, as a kernel function the proofs can unfold.
    pub fn same(self, other: GenerationId) -> bool {
        self.number == other.number
    }

    pub fn checked_next(self) -> Option<Self> {
        if self.number == u64::MAX {
            None
        } else {
            Some(Self {
                number: self.number + 1,
            })
        }
    }

    pub fn checked_previous(self) -> Option<Self> {
        if self.number == 0 {
            None
        } else {
            Some(Self {
                number: self.number - 1,
            })
        }
    }
}

/// `current` names `candidate`: false when there is no current at all.
/// (The parameter is not called `generation`: Aeneas emits Rust locals and
/// module paths into one Lean namespace, and a local named like the module
/// shadows it.)
pub fn names_generation(current: Option<GenerationId>, candidate: GenerationId) -> bool {
    match current {
        Some(current) => current.same(candidate),
        None => false,
    }
}

/// Strictly ascending generation identities, numbers strictly increasing
/// from one position to the next.
pub fn strictly_ascending(ids: &[GenerationId]) -> bool {
    let mut i = 1;
    while i < ids.len() {
        if ids[i - 1].number >= ids[i].number {
            return false;
        }
        i += 1;
    }
    true
}

/// Admitted, strictly ascending and duplicate-free generation identities.
/// Raw vectors cannot enter retention policy without admission.
///
/// ```compile_fail
/// use gripsack_kernel::retention::plan_prune;
/// plan_prune(&vec![1, 2, 3], None, Some(1));
/// ```
pub struct GenerationList {
    ids: Vec<GenerationId>,
}

impl GenerationList {
    /// Admit a vector whose numbers strictly ascend; refuse everything else.
    pub fn new(ids: Vec<GenerationId>) -> Option<Self> {
        if strictly_ascending(&ids) {
            Some(Self { ids })
        } else {
            None
        }
    }

    pub fn as_slice(&self) -> &[GenerationId] {
        &self.ids
    }

    pub(crate) fn into_vec(self) -> Vec<GenerationId> {
        self.ids
    }
}
