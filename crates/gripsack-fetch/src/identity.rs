//! Transport bytes and canonical payload trees are different pin domains.

use gripsack_store::hash::PayloadHash;
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadHash(String);

impl DownloadHash {
    pub fn parse(value: &str) -> std::io::Result<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "download hash must be 64 lowercase hexadecimal digits",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn finish(hasher: Sha256) -> Self {
        use std::fmt::Write;
        let mut value = String::with_capacity(64);
        for byte in hasher.finalize() {
            write!(&mut value, "{byte:02x}").expect("writing a string");
        }
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DownloadHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<DownloadHash> for String {
    fn from(hash: DownloadHash) -> Self {
        hash.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchIdentity {
    Download(DownloadHash),
    Tree(PayloadHash),
}

impl FetchIdentity {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Download(hash) => hash.as_str(),
            Self::Tree(hash) => hash.as_str(),
        }
    }
}

impl fmt::Display for FetchIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<FetchIdentity> for String {
    fn from(identity: FetchIdentity) -> Self {
        match identity {
            FetchIdentity::Download(hash) => hash.into(),
            FetchIdentity::Tree(hash) => hash.into(),
        }
    }
}
