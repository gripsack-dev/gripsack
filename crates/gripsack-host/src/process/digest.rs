//! Canonical SHA-256 receipt values. Decoding cannot construct a malformed
//! digest; hashing large executables remains streaming at the admission seam.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::{fmt, io};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn parse(text: &str) -> io::Result<Self> {
        fn digit(byte: u8) -> Option<u8> {
            match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            }
        }
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "SHA-256 digest must be 64 lowercase hexadecimal characters",
            )
        };
        if text.len() != 64 {
            return Err(invalid());
        }
        let mut bytes = [0; 32];
        for (out, pair) in bytes.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
            *out = digit(pair[0]).ok_or_else(invalid)? << 4 | digit(pair[1]).ok_or_else(invalid)?;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = Sha256Digest;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a canonical SHA-256 digest")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Sha256Digest::parse(value).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}
