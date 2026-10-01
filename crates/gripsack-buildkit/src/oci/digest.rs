use gripsack_process::Sha256Digest;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, io};

/// OCI descriptors and DiffIDs share SHA-256 spelling, but remain distinct from
/// portable artifact tree keys and native archive receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlobDigest([u8; 32]);
impl BlobDigest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self { Self(bytes) }
    pub fn parse(value: &str) -> io::Result<Self> {
        let raw = value.strip_prefix("sha256:").ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "OCI digest must use sha256"))?;
        Self::from_hex(raw)
    }
    pub(super) fn from_hex(value: &str) -> io::Result<Self> {
        Ok(Self(*Sha256Digest::parse(value)?.as_bytes()))
    }
}
impl fmt::Display for BlobDigest {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("sha256:")?;
        for byte in self.0 { write!(out, "{byte:02x}")?; }
        Ok(())
    }
}
impl Serialize for BlobDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> { serializer.collect_str(self) }
}
impl<'de> Deserialize<'de> for BlobDigest {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = BlobDigest;
            fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result { out.write_str("a lowercase OCI SHA-256 digest") }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> { BlobDigest::parse(value).map_err(E::custom) }
        }
        decoder.deserialize_str(Visitor)
    }
}
