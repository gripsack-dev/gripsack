use crate::{identity::LlbVertexDigest, plan::MAX_PLAN_NODES};
use serde::de::{Error, SeqAccess, Visitor};
use std::fmt;

pub(super) fn vertices<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Vec<LlbVertexDigest>, D::Error> {
    struct Vertices;
    impl<'de> Visitor<'de> for Vertices {
        type Value = Vec<LlbVertexDigest>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded list of failed LLB vertex digests")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
            let mut vertices = Vec::new();
            while let Some(vertex) = input.next_element()? {
                if vertices.len() == MAX_PLAN_NODES + 1 {
                    return Err(A::Error::custom(
                        "failure vertex count exceeds definition bound",
                    ));
                }
                vertices.push(vertex);
            }
            Ok(vertices)
        }
    }
    decoder.deserialize_seq(Vertices)
}
