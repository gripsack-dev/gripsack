//! Ownership keys distinguish a whole destination from one named managed
//! block. Legacy blocks retain their historical module-name identity.

use serde::{Deserialize, Deserializer, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// Safe marker token for a v5 workspace block. Authoring markers may contain
/// whitespace or Unicode; a versioned digest encoding cannot inject marker
/// syntax and case-folded spelling preserves the admitted collision rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ManagedBlockId(String);

impl ManagedBlockId {
    pub fn from_marker(marker: &str) -> io::Result<Self> {
        if marker.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty managed-block marker",
            ));
        }
        let normalized = marker.to_lowercase();
        Ok(Self(format!(
            "w1-{}",
            crate::hash::hex_sha256(normalized.as_bytes())
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ManagedBlockId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.len() != 67
            || !value.starts_with("w1-")
            || !value.as_bytes()[3..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(serde::de::Error::custom(
                "invalid persisted workspace block identity",
            ));
        }
        Ok(Self(value))
    }
}

/// Historical string modes remain readable. A workspace block is deliberately
/// a disjoint object shape: old binaries must reject it, not silently discard
/// its marker and reinterpret it as a module-named merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum StoredOwnership {
    Legacy(gripsack_ir::Ownership),
    ManagedBlock { managed_block: ManagedBlockId },
}

impl StoredOwnership {
    pub fn for_entry(
        policy: &gripsack_ir::Ownership,
        block: Option<&ManagedBlockId>,
    ) -> io::Result<Self> {
        match block {
            None => Ok(Self::Legacy(policy.clone())),
            Some(block) if *policy == gripsack_ir::Ownership::Merge => Ok(Self::ManagedBlock {
                managed_block: block.clone(),
            }),
            Some(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "block identity on non-merge ownership",
            )),
        }
    }

    pub fn policy(&self) -> gripsack_ir::Ownership {
        match self {
            Self::Legacy(policy) => policy.clone(),
            Self::ManagedBlock { .. } => gripsack_ir::Ownership::Merge,
        }
    }

    pub fn block_id(&self) -> Option<&ManagedBlockId> {
        match self {
            Self::Legacy(_) => None,
            Self::ManagedBlock { managed_block } => Some(managed_block),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OwnershipKey {
    destination: PathBuf,
    block: Option<String>,
}

impl OwnershipKey {
    pub fn new(destination: PathBuf, block: Option<&str>) -> Self {
        Self {
            destination,
            block: block.map(str::to_lowercase),
        }
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::Ownership;
    use serde_json::json;

    #[test]
    fn workspace_markers_cannot_be_reinterpreted_as_legacy_module_names() {
        let marker = ManagedBlockId::from_marker("Shell settings").unwrap();
        assert_eq!(
            marker,
            ManagedBlockId::from_marker("shell SETTINGS").unwrap()
        );
        let ownership = StoredOwnership::for_entry(&Ownership::Merge, Some(&marker)).unwrap();
        let wire = serde_json::to_value(&ownership).unwrap();
        assert!(serde_json::from_value::<Ownership>(wire.clone()).is_err());
        assert_eq!(
            serde_json::from_value::<StoredOwnership>(wire).unwrap(),
            ownership
        );
        assert_eq!(
            serde_json::from_value::<StoredOwnership>(json!("merge")).unwrap(),
            StoredOwnership::Legacy(Ownership::Merge)
        );
        for malformed in [
            json!({"managed_block": "../other"}),
            json!({"managed_block": marker, "mode": "merge"}),
            json!({"managed_block": null}),
        ] {
            assert!(serde_json::from_value::<StoredOwnership>(malformed).is_err());
        }
    }
}
