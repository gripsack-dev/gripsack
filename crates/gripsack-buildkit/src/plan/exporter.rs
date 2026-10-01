//! One fixed OCI export profile. No arbitrary exporter attributes, registry
//! publication, compression negotiation or ambient image configuration.
use super::admission::{absolute_path, environment_key};
use crate::identity::ExporterDigest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_IMAGE_CONFIG_BYTES: usize = 64 * 1024;
const MAX_CONFIG_ENTRIES: usize = 4096;
// Changing a digest-affecting exporter option requires a new profile domain.
const OCI_PROFILE: &[u8] = b"gripsack-oci-v1\0compatibility=30\0media=oci\0compression=gzip:6\0force=true\0epoch=0\0rewrite=true\0tar=true\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExporterPlan {
    Local,
    Oci { config: ImageConfig },
}

/// Explicit OCI runtime configuration. The platform is the checked plan's
/// platform; rootfs/history/digests come from the exporter, never the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageConfig {
    pub entrypoint: Vec<String>,
    pub args: Vec<String>,
    /// Unique KEY=value entries, sorted by key, with no inherited base Env.
    pub env: Vec<String>,
    pub cwd: String,
    /// Numeric uid:gid, independent of mutable image passwd/group databases.
    pub user: String,
}

impl ExporterPlan {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Local => Ok(()),
            Self::Oci { config } => config.validate(),
        }
    }

    /// Length-prefixed UTF-8 avoids cross-language JSON escaping differences.
    /// Stream into the digest rather than copying an encoded configuration.
    pub fn digest(&self) -> ExporterDigest {
        let Self::Oci { config } = self else {
            return ExporterDigest::of(br#"{"kind":"local"}"#);
        };
        let mut hash = Sha256::new();
        hash.update(OCI_PROFILE);
        for values in [&config.entrypoint, &config.args, &config.env] {
            hash.update((values.len() as u64).to_le_bytes());
            for value in values {
                hash_string(&mut hash, value);
            }
        }
        hash_string(&mut hash, &config.cwd);
        hash_string(&mut hash, &config.user);
        ExporterDigest::from_bytes(hash.finalize().into())
    }
}

impl ImageConfig {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if !absolute_path(&self.cwd) {
            return Err("image working directory must be normalized and absolute");
        }
        let Some((uid, gid)) = self.user.split_once(':') else {
            return Err("image user must be an explicit numeric uid:gid");
        };
        if !numeric_id(uid) || !numeric_id(gid) {
            return Err("image user must be a canonical numeric uid:gid");
        }
        if self.entrypoint.first().is_some_and(|program| !absolute_path(program)) {
            return Err("image entrypoint must name an absolute executable");
        }
        let mut bytes = self.cwd.len() + self.user.len();
        for values in [&self.entrypoint, &self.args, &self.env] {
            if values.len() > MAX_CONFIG_ENTRIES {
                return Err("image configuration entry count exceeds its bound");
            }
            for value in values {
                bytes = bytes.checked_add(value.len()).ok_or("image configuration size overflow")?;
                if value.contains('\0') || bytes > MAX_IMAGE_CONFIG_BYTES {
                    return Err("image configuration contains NUL or exceeds its byte bound");
                }
            }
        }
        if bytes > MAX_IMAGE_CONFIG_BYTES {
            return Err("image configuration exceeds its byte bound");
        }
        let mut previous = None;
        for entry in &self.env {
            let Some((key, _)) = entry.split_once('=') else {
                return Err("image environment binding must be KEY=value");
            };
            if !environment_key(key) || previous.is_some_and(|key_before| key_before >= key) {
                return Err("image environment keys must be valid, unique and sorted");
            }
            previous = Some(key);
        }
        Ok(())
    }
}

fn numeric_id(value: &str) -> bool {
    !value.is_empty()
        && (value == "0" || !value.starts_with('0'))
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u32>().is_ok()
}
fn hash_string(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value.as_bytes());
}
