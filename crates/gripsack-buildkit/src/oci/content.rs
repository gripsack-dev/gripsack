use super::{
    BlobDigest, FileKind, OciError, OciLimits, ValidatedImage, archive::ArchiveIndex, resolve,
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
};

impl ValidatedImage {
    /// Read one regular file through the verified image namespace. Rehash both
    /// archive blobs and selected bytes; a substituted archive cannot inherit
    /// this capability. Neither symlinks nor extraction ever touch host paths.
    pub fn read_file(
        &self,
        archive: &mut File,
        path: &str,
        max_bytes: u64,
        limits: OciLimits,
    ) -> Result<Vec<u8>, OciError> {
        let (_, entry) = resolve(&self.files, path)?;
        let FileKind::File { size, digest } = entry.kind else {
            return Err(OciError::Invalid(
                "requested OCI content is not a regular file",
            ));
        };
        if size > max_bytes || size > usize::MAX as u64 {
            return Err(OciError::Invalid(
                "requested OCI content exceeds its read bound",
            ));
        }
        let index = ArchiveIndex::scan(archive, limits)?;
        let mut remaining = limits.expanded_bytes;
        let mut bytes = Vec::with_capacity(size as usize);
        for layer in self.layers.iter().rev() {
            let region = index
                .blobs
                .get(layer)
                .ok_or(OciError::Invalid("verified layer is absent from archive"))?;
            archive.seek(SeekFrom::Start(region.offset))?;
            let source = Read::by_ref(archive).take(region.size);
            let decoder = flate2::bufread::GzDecoder::new(BufReader::new(source));
            let mut bounded = decoder.take(remaining);
            {
                let mut layer = tar::Archive::new(&mut bounded);
                for entry in layer.entries()? {
                    let mut entry = entry?;
                    if !entry.header().entry_type().is_file() || entry.size() != size {
                        continue;
                    }
                    bytes.clear();
                    entry.read_to_end(&mut bytes)?;
                    if bytes.len() as u64 == size
                        && BlobDigest::from_bytes(Sha256::digest(&bytes).into()) == digest
                    {
                        return Ok(bytes);
                    }
                }
            }
            remaining = bounded.limit();
            if remaining == 0 {
                return Err(OciError::Invalid(
                    "OCI content read exceeds expanded byte bound",
                ));
            }
        }
        Err(OciError::Invalid(
            "verified file bytes are absent from archive",
        ))
    }
}
