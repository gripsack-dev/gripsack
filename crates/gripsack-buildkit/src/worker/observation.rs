//! Shared admitted instance identity and resource observations. Docker and Lima
//! effects use the same durable manager/lease contract without compiling the
//! other platform's process adapter.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, num::NonZeroU16};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(super) struct ContainerId(String);
impl TryFrom<String> for ContainerId {
    type Error = std::io::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        gripsack_process::Sha256Digest::parse(&value)?;
        Ok(Self(value))
    }
}
impl From<ContainerId> for String {
    fn from(value: ContainerId) -> Self { value.0 }
}
impl ContainerId {
    pub(super) fn as_str(&self) -> &str { &self.0 }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContainerObservation {
    pub id: ContainerId,
    pub running: bool,
    pub image: String,
    pub nano_cpus: u64,
    pub memory: u64,
    /// Absent labels are never ownership evidence.
    pub labels: Option<BTreeMap<String, String>>,
}
const NANOS_PER_CPU: u64 = 1_000_000_000;
pub(super) fn nano_cpus(cpus: NonZeroU16) -> u64 { cpus.get() as u64 * NANOS_PER_CPU }
/// Never round a fractional or foreign resource allocation into a valid binding.
pub(super) fn cpus_from_nano(nano: u64) -> Option<NonZeroU16> {
    if nano % NANOS_PER_CPU != 0 { return None; }
    NonZeroU16::new(u16::try_from(nano / NANOS_PER_CPU).ok()?)
}
