//! Admitted prior identities and private capability-backed blob storage.
//! Both generation restoration and journal recovery use this boundary.
//! Wire hashes/modes remain compatible with valid historical records.

use gripsack_fs::Dir;
use serde::{Deserialize, Deserializer, Serialize};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Raw SHA-256 of pre-adoption file bytes, not a payload/manifest hash.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct PriorBlobId(String);

impl PriorBlobId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// For inventory/report comparison only. Effects use `directory`/`read_blob`.
    pub fn path_in(&self, home: &Path) -> PathBuf {
        home.join("prior").join(&self.0)
    }
}

impl TryFrom<String> for PriorBlobId {
    type Error = io::Error;

    fn try_from(value: String) -> io::Result<Self> {
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prior blob identity must be exactly 64 ASCII hexadecimal characters",
            ));
        }
        Ok(Self(value))
    }
}

impl<'de> Deserialize<'de> for PriorBlobId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Full saved Unix permissions, distinct from a file-type bitfield.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FileMode(u32);

impl FileMode {
    pub fn bits(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for FileMode {
    type Error = io::Error;

    fn try_from(mode: u32) -> io::Result<Self> {
        if mode > 0o7777 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid prior file mode",
            ));
        }
        Ok(Self(mode))
    }
}

impl<'de> Deserialize<'de> for FileMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(u32::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// The pre-adoption object carried through every generation of its ownership
/// epoch. Absent is represented by the enclosing Option, never invented on read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Prior {
    File { hash: PriorBlobId, mode: FileMode },
    Symlink { target: String },
}

/// Pin the prior directory itself. A planted directory symlink is not authority
/// to restore, chmod or collect its target, even inside the Gripsack home.
pub fn directory(home: &Dir) -> io::Result<Dir> {
    gripsack_fs::open_dir_nofollow(home, Path::new("prior"))
}

/// Read only the named admitted blob through a pinned capability and verify its
/// bytes before allowing either a restore intent or a destination mutation.
pub fn read_blob(home: &Dir, identity: &PriorBlobId) -> io::Result<Vec<u8>> {
    let directory = directory(home)?;
    let mut file = gripsack_fs::open_file_nofollow(&directory, Path::new(identity.as_str()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if !crate::hash::hex_sha256(&bytes).eq_ignore_ascii_case(identity.as_str()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prior blob content hash mismatch",
        ));
    }
    Ok(bytes)
}

/// Preserve originals at 0600 in a 0700 directory; deduplicate by raw bytes.
/// Corrupt existing regular bytes are quarantined, never accepted by pathname.
pub fn store_blob(home: &Dir, bytes: &[u8]) -> io::Result<PriorBlobId> {
    let identity = PriorBlobId(crate::hash::hex_sha256(bytes));
    let directory = crate::private_state::ensure_directory(home, Path::new("prior"))?;
    match gripsack_fs::open_file_nofollow(&directory, Path::new(identity.as_str())) {
        Ok(mut file) => {
            let mut existing = Vec::new();
            file.read_to_end(&mut existing)?;
            if existing == bytes {
                crate::private_state::restrict_file(&file, Path::new(identity.as_str()))?;
                return Ok(identity);
            }
            gripsack_fs::rename(
                &directory,
                Path::new(identity.as_str()),
                &directory,
                &PathBuf::from(format!("{}.corrupt", identity.as_str())),
            )?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    gripsack_fs::atomic_write_with_mode(&directory, Path::new(identity.as_str()), bytes, 0o600)?;
    Ok(identity)
}

#[cfg(test)]
mod tests;
