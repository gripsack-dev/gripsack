//! Explicit numeric wire seam; no serializer dependency enters policy proofs.
use crate::GenerationId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(crate) fn serialize<S: Serializer>(
    id: &GenerationId,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    id.value().serialize(serializer)
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<GenerationId, D::Error> {
    u64::deserialize(deserializer).map(GenerationId::new)
}
