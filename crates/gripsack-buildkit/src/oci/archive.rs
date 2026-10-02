use super::{BlobDigest, OciError, OciLimits, model::Descriptor};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
};

#[derive(Debug, Clone, Copy)]
pub(super) struct Region {
    pub offset: u64,
    pub size: u64,
}
pub(super) struct ArchiveIndex {
    pub layout: Region,
    pub index: Region,
    pub blobs: BTreeMap<BlobDigest, Region>,
}
impl ArchiveIndex {
    pub fn scan(file: &mut File, limits: OciLimits) -> Result<Self, OciError> {
        let metadata = file.metadata()?;
        let size = metadata.len();
        if !metadata.is_file() || size > limits.archive_bytes || size % 512 != 0 {
            return Err(OciError::Invalid(
                "OCI archive is not a bounded complete tar file",
            ));
        }
        file.rewind()?;
        let mut blobs = BTreeMap::new();
        let mut names = BTreeSet::new();
        let mut layout = None;
        let mut index = None;
        let mut last_end = 0;
        let mut archive = tar::Archive::new(&mut *file);
        // OCI layout member names fit ordinary headers. Reject extension bodies
        // before tar may allocate them; layers use a separate bounded PAX pass.
        for (count, entry) in archive.entries_with_seek()?.raw(true).enumerate() {
            if count >= limits.entries {
                return Err(OciError::Invalid("OCI archive entry count exceeded"));
            }
            let mut entry = entry?;
            let kind = entry.header().entry_type().as_byte();
            let path = entry.path()?;
            let path = path
                .to_str()
                .ok_or(OciError::Invalid("OCI member path is not UTF-8"))?;
            let path = path.strip_prefix("./").unwrap_or(path);
            let region = Region {
                offset: entry.raw_file_position(),
                size: entry.size(),
            };
            last_end = region
                .offset
                .checked_add(
                    region
                        .size
                        .div_ceil(512)
                        .checked_mul(512)
                        .ok_or(OciError::Invalid("OCI member size overflow"))?,
                )
                .ok_or(OciError::Invalid("OCI member range overflow"))?;
            if last_end > size {
                return Err(OciError::Invalid("OCI member exceeds its archive"));
            }
            if !names.insert(path.to_owned()) {
                return Err(OciError::Invalid("duplicate OCI archive member"));
            }
            if kind == b'5' {
                if region.size != 0
                    || !matches!(
                        path.trim_end_matches('/'),
                        "." | "" | "blobs" | "blobs/sha256"
                    )
                {
                    return Err(OciError::Invalid("unexpected OCI archive directory"));
                }
                continue;
            }
            if !matches!(kind, 0 | b'0') {
                return Err(OciError::Invalid(
                    "OCI layout must contain only regular files and namespace directories",
                ));
            }
            match path {
                "oci-layout" => {
                    if layout.replace(region).is_some() {
                        return Err(OciError::Invalid("duplicate OCI layout"));
                    }
                }
                "index.json" => {
                    if index.replace(region).is_some() {
                        return Err(OciError::Invalid("duplicate OCI index"));
                    }
                }
                _ => {
                    let digest = BlobDigest::from_hex(
                        path.strip_prefix("blobs/sha256/")
                            .ok_or(OciError::Invalid("foreign file in OCI layout"))?,
                    )?;
                    if blobs.insert(digest, region).is_some() {
                        return Err(OciError::Invalid("duplicate OCI blob"));
                    }
                    let actual = hash(&mut entry)?;
                    if actual != digest {
                        return Err(OciError::Invalid(
                            "OCI blob bytes do not match their content address",
                        ));
                    }
                }
            }
        }
        end_marker(file, last_end)?;
        Ok(Self {
            layout: layout.ok_or(OciError::Invalid("missing OCI layout"))?,
            index: index.ok_or(OciError::Invalid("missing OCI index"))?,
            blobs,
        })
    }
    pub fn descriptor(&self, value: &Descriptor, media: &str) -> Result<Region, OciError> {
        if value.media_type != media {
            return Err(OciError::Invalid(
                "unsupported or substituted OCI media type",
            ));
        }
        let region = *self
            .blobs
            .get(&value.digest)
            .ok_or(OciError::Invalid("OCI descriptor names a missing blob"))?;
        if region.size != value.size {
            return Err(OciError::Invalid(
                "OCI descriptor size differs from its blob",
            ));
        }
        Ok(region)
    }
}

pub(super) fn end_marker(file: &mut File, last_end: u64) -> Result<(), OciError> {
    let size = file.metadata()?.len();
    if size % 512 != 0 || size.saturating_sub(last_end) < 1024 {
        return Err(OciError::Invalid("OCI tar lacks its complete end marker"));
    }
    file.seek(SeekFrom::Start(last_end))?;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if buffer[..count].iter().any(|byte| *byte != 0) {
            return Err(OciError::Invalid(
                "unexpected data after OCI tar end marker",
            ));
        }
    }
    Ok(())
}

pub(super) fn json<T: DeserializeOwned>(
    file: &mut File,
    region: Region,
    limit: u64,
) -> Result<T, OciError> {
    if region.size > limit {
        return Err(OciError::Invalid(
            "OCI JSON metadata exceeds its byte bound",
        ));
    }
    let length = usize::try_from(region.size)
        .map_err(|_| OciError::Invalid("OCI metadata size is not addressable"))?;
    file.seek(SeekFrom::Start(region.offset))?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub(super) fn hash(reader: &mut impl Read) -> Result<BlobDigest, OciError> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(BlobDigest::from_bytes(hash.finalize().into()))
}
