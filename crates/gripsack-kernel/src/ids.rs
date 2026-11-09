//! Kernel-facing identities (plan/0056 §4): interned, `Copy`, no strings,
//! paths or maps. The host interns destination paths to [`DestId`]s, hashes
//! content to [`Digest`]s and numbers outstanding commands with [`CmdId`]s;
//! the kernel only compares them.
//!
//! Generation and transaction identities already exist with their proved
//! decisions: [`GenId`] is [`GenerationId`] and [`TxId`] is
//! [`TransactionId`]. The aliases name them as the plan's interface does
//! without a second type.
//!
//! Under proof since F2 step 2: the recover machine (`recover.rs`) carries
//! [`DestId`] and [`Digest`], so the translation reaches them from there.
//! [`CmdId`] is not used yet: a machine has one command outstanding and
//! checks each event against its phase instead.
use crate::{GenerationId, selection::TransactionId};

/// `GenId` of the plan's interface: one generation number.
pub type GenId = GenerationId;
/// `TxId` of the plan's interface: one never-reused transaction identity.
pub type TxId = TransactionId;

/// One admitted destination, interned by the host. The kernel never sees
/// the path; equal IDs mean the host's key equivalence judged the paths
/// equal (S6).
pub struct DestId {
    index: u32,
}

impl DestId {
    pub fn new(index: u32) -> Self {
        Self { index }
    }

    /// The interning index, for the host's table lookup only.
    pub fn value(self) -> u32 {
        self.index
    }

    pub fn same(self, other: DestId) -> bool {
        self.index == other.index
    }
}

pub const DIGEST_BYTES: usize = 32;

/// A SHA-256 content digest. Equal digests are taken to mean equal bytes
/// (S7: collision resistance, an assumption the proofs list, not prove).
pub struct Digest {
    bytes: [u8; DIGEST_BYTES],
}

impl Digest {
    pub fn from_bytes(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8; DIGEST_BYTES] {
        &self.bytes
    }

    /// Byte-exact identity, as a loop the proofs can reason about.
    pub fn same(&self, other: &Digest) -> bool {
        let mut i = 0;
        while i < DIGEST_BYTES {
            if self.bytes[i] != other.bytes[i] {
                return false;
            }
            i += 1;
        }
        true
    }
}

/// One issued command. A machine has at most one effect outstanding, so the
/// driver answers each `Command` with an `Event` carrying the same ID.
pub struct CmdId {
    number: u32,
}

impl CmdId {
    pub fn new(number: u32) -> Self {
        Self { number }
    }

    pub fn value(self) -> u32 {
        self.number
    }

    pub fn same(self, other: CmdId) -> bool {
        self.number == other.number
    }

    pub fn checked_next(self) -> Option<Self> {
        if self.number == u32::MAX {
            None
        } else {
            Some(Self {
                number: self.number + 1,
            })
        }
    }
}
