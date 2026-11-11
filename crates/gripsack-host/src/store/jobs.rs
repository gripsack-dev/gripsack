//! Durable schedule identities. Scheduler argv never grants path authority.
use serde::{Deserialize, Deserializer, Serialize};
use std::{fmt, io};

/// Qualified adapter sequence: version/domain, ownership probes, reload and enable.
pub const REGISTER_PROCESS_COUNT: usize = 8;
/// Qualified removal sequence: version/domain, ownership probes, disable and reload.
pub const REMOVE_PROCESS_COUNT: usize = 6;

macro_rules! identity {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: &str) -> io::Result<Self> {
                if value.len() != 64
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        concat!(stringify!($name), " must be lowercase SHA256 hexadecimal"),
                    ));
                }
                Ok(Self(value.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(serde::de::Error::custom)
            }
        }
    };
}

identity!(JobId);
identity!(JobRevision);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_arguments_have_one_canonical_encoding() {
        let valid = "a".repeat(64);
        assert_eq!(JobId::parse(&valid).unwrap().as_str(), valid);
        assert_eq!(JobRevision::parse(&valid).unwrap().as_str(), valid);
        for invalid in [
            "A".repeat(64),
            "g".repeat(64),
            "a".repeat(63),
            "a".repeat(65),
            "../manifest.json".into(),
        ] {
            assert!(JobId::parse(&invalid).is_err());
            assert!(serde_json::from_value::<JobRevision>(serde_json::json!(invalid)).is_err());
        }
    }
}
