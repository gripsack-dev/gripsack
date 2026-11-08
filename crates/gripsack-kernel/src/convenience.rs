//! Production conveniences for kernel types: formatting, parsing, ordering,
//! hashing and iteration. Nothing here is a decision and nothing here is
//! reached from the decision modules, so the Charon translation
//! (`[package.metadata.charon] start_from` in Cargo.toml) never sees it.
//! Ordering and hashing agree with `GenerationId::same`/`value` by
//! construction: both read the one `u64`.
use crate::{GenerationId, GenerationList};
use alloc::vec::Vec;
use core::{cmp::Ordering, fmt, hash, ops::Deref, str::FromStr};

impl fmt::Display for GenerationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(f)
    }
}

impl FromStr for GenerationId {
    type Err = core::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self::new)
    }
}

impl Ord for GenerationId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value().cmp(&other.value())
    }
}

impl PartialOrd for GenerationId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl hash::Hash for GenerationId {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.value().hash(state);
    }
}

impl Deref for GenerationList {
    type Target = [GenerationId];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl IntoIterator for GenerationList {
    type Item = GenerationId;
    type IntoIter = alloc::vec::IntoIter<GenerationId>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'a> IntoIterator for &'a GenerationList {
    type Item = &'a GenerationId;
    type IntoIter = core::slice::Iter<'a, GenerationId>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl GenerationList {
    /// The admitted identities, for callers that need to own them again.
    pub fn to_vec(&self) -> Vec<GenerationId> {
        self.as_slice().to_vec()
    }
}
