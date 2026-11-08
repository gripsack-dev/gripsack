//! Exact lifecycle selection identity, separate from the selected generation,
//! and the commit classifier over it (0019, 0026 §4, 0028). The store admits
//! pointer/marker bytes and allocates transaction identities; this module
//! compares only admitted values and never touches the filesystem.
//!
//! Proved (proofs/Kernel/Selection.lean, CLASSIFY-001): `classify` returns
//! Committed iff current = target; Uncommitted iff fresh with no current or
//! current = previous ≠ target; Ambiguous otherwise. Numeric order never
//! decides — equality is exact identity, transaction bytes included.
use crate::GenerationId;

/// The commit classification of an interrupted run: recovery compares the
/// run marker's declared transaction against the live `current` pointer by
/// EXACT identity. A crashed roll-forward has current < target before the
/// flip and is not committed by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// The flip landed: the generation owns the truth; journal cleanup only.
    Committed,
    /// The flip never landed: restore every journaled prior.
    Uncommitted,
    /// Neither: corruption or tampering — recovery changes nothing and keeps
    /// the journal intact (fail closed).
    Ambiguous,
}

pub const TRANSACTION_ID_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionId {
    bytes: [u8; TRANSACTION_ID_BYTES],
}

impl TransactionId {
    pub fn from_bytes(bytes: [u8; TRANSACTION_ID_BYTES]) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8; TRANSACTION_ID_BYTES] {
        &self.bytes
    }

    /// Byte-exact identity, as a kernel loop the proofs can reason about.
    pub fn same(&self, other: &TransactionId) -> bool {
        let mut i = 0;
        while i < TRANSACTION_ID_BYTES {
            if self.bytes[i] != other.bytes[i] {
                return false;
            }
            i += 1;
        }
        true
    }
}

/// Historical pointers remain generation-scoped. New pointers bind a unique
/// transaction even when rollback reactivates the already-current generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionIdentity {
    Legacy {
        generation: GenerationId,
    },
    Transaction {
        generation: GenerationId,
        transaction: TransactionId,
    },
}

impl SelectionIdentity {
    pub fn legacy(generation: GenerationId) -> Self {
        Self::Legacy { generation }
    }

    pub fn transaction(generation: GenerationId, transaction: TransactionId) -> Self {
        Self::Transaction {
            generation,
            transaction,
        }
    }

    pub fn generation(&self) -> GenerationId {
        match self {
            Self::Legacy { generation } => *generation,
            Self::Transaction { generation, .. } => *generation,
        }
    }

    pub fn transaction_id(&self) -> Option<&TransactionId> {
        match self {
            Self::Legacy { .. } => None,
            Self::Transaction { transaction, .. } => Some(transaction),
        }
    }

    /// Exact identity: same shape, same generation and (for transactions)
    /// the same transaction bytes. A legacy pointer never equals a
    /// transaction pointer of the same generation.
    pub fn same(&self, other: &SelectionIdentity) -> bool {
        match (self, other) {
            (Self::Legacy { generation: a }, Self::Legacy { generation: b }) => a.same(*b),
            (
                Self::Transaction {
                    generation: a,
                    transaction: ta,
                },
                Self::Transaction {
                    generation: b,
                    transaction: tb,
                },
            ) => a.same(*b) && ta.same(tb),
            _ => false,
        }
    }
}

/// One observed transaction, identity roles named explicitly. Values are
/// owned copies of the admitted marker and pointer: the classifier compares
/// identities, it never holds the store's state.
#[derive(Debug, Clone, Copy)]
pub struct RecoveryFacts {
    pub previous: Option<SelectionIdentity>,
    pub target: SelectionIdentity,
    pub current: Option<SelectionIdentity>,
}

pub fn classify(facts: RecoveryFacts) -> Classification {
    match facts.current {
        Some(current) => {
            if current.same(&facts.target) {
                Classification::Committed
            } else {
                match facts.previous {
                    Some(previous) => {
                        if current.same(&previous) {
                            Classification::Uncommitted
                        } else {
                            Classification::Ambiguous
                        }
                    }
                    None => Classification::Ambiguous,
                }
            }
        }
        None => match facts.previous {
            None => Classification::Uncommitted,
            Some(_) => Classification::Ambiguous,
        },
    }
}
