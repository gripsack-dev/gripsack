use super::{CheckError, LlbVertexDigest, MAX_PLAN_NODES, NodeIndex, NodeWitness, Platform, pb};
use gripsack_policy::buildkit::{InputBinding, VertexIndex, input_binding};
use protobuf::Message;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn known(message: &impl Message) -> Result<(), CheckError> {
    if let Some((field, _)) = message.special_fields().unknown_fields().iter().next() {
        return Err(CheckError::UnsupportedField {
            message: std::any::type_name_of_val(message),
            field,
        });
    }
    Ok(())
}

pub(super) fn constraints(
    op: &pb::Op,
    expected: Platform,
    required: bool,
) -> Result<(), CheckError> {
    if let Some(constraints) = op.constraints.as_ref() {
        known(constraints)?;
        if !constraints.filter.is_empty() {
            return Err(CheckError::Mismatch("unexpected worker constraints"));
        }
    }
    match op.platform.as_ref() {
        Some(platform) => {
            known(platform)?;
            if platform.os != "linux"
                || platform.architecture != expected.architecture.as_str()
                || !platform.variant.is_empty()
                || !platform.os_version.is_empty()
                || !platform.os_features.is_empty()
            {
                return Err(CheckError::Mismatch("execution platform substitution"));
            }
        }
        None if required => {
            return Err(CheckError::Mismatch("missing explicit execution platform"));
        }
        None => {}
    }
    Ok(())
}

pub(super) fn metadata(
    definition: &pb::Definition,
    vertices: &BTreeMap<LlbVertexDigest, usize>,
) -> Result<(), CheckError> {
    if let Some(source) = definition.source.as_ref() {
        known(source)?;
        if source.locations.len() > vertices.len() {
            return Err(CheckError::Mismatch("source-map cardinality"));
        }
        let mut locations = BTreeSet::new();
        for entry in &source.locations {
            known(entry)?;
            if !vertices.contains_key(entry.key.as_str()) || !locations.insert(entry.key.as_str()) {
                return Err(CheckError::Mismatch(
                    "source map names an absent or duplicate vertex",
                ));
            }
            let empty = entry
                .value
                .as_ref()
                .ok_or(CheckError::Mismatch("missing source-map location entry"))?;
            known(empty)?;
        }
    }
    if definition.metadata.len() > vertices.len() {
        return Err(CheckError::Mismatch("metadata cardinality"));
    }
    let mut keys = BTreeSet::new();
    for entry in &definition.metadata {
        known(entry)?;
        if !vertices.contains_key(entry.key.as_str()) || !keys.insert(entry.key.as_str()) {
            return Err(CheckError::Mismatch(
                "metadata names an absent or duplicate vertex",
            ));
        }
        let value = entry
            .value
            .as_ref()
            .ok_or(CheckError::Mismatch("missing metadata value"))?;
        known(value)?;
        if value.ignore_cache {
            return Err(CheckError::Mismatch("unchecked cache policy"));
        }
        let mut descriptions = BTreeSet::new();
        for description in &value.description {
            known(description)?;
            if !descriptions.insert(description.key.as_str()) {
                return Err(CheckError::Mismatch("duplicate diagnostic metadata key"));
            }
        }
        let mut capabilities = BTreeSet::new();
        for capability in &value.caps {
            known(capability)?;
            if !capability.value
                || !capabilities.insert(capability.key.as_str())
                || !matches!(
                    capability.key.as_str(),
                    "source.image"
                        | "source.local"
                        | "source.local.unique"
                        | "source.local.sharedkeyhint"
                        | "file.base"
                        | "platform"
                        | "constraints"
                        | "meta.description"
                        | "exec.meta.base"
                        | "exec.meta.network"
                        | "exec.meta.security"
                        | "exec.mount.bind"
                        | "exec.meta.setsdefaultpath"
                        | "exec.meta.removemountstubs.recursive"
                )
            {
                return Err(CheckError::Mismatch("unsupported LLB capability"));
            }
        }
    }
    Ok(())
}

/// Match actual input slots to the exact selected producer digest/output.
/// A fixed stack bitmap tracks coverage without allocating per vertex.
pub(super) struct Inputs<'a> {
    actual: &'a [pb::Input],
    witness: &'a [NodeWitness],
    vertices: &'a BTreeMap<LlbVertexDigest, usize>,
    used: [bool; MAX_PLAN_NODES],
}
impl<'a> Inputs<'a> {
    pub(super) fn new(
        actual: &'a [pb::Input],
        witness: &'a [NodeWitness],
        vertices: &'a BTreeMap<LlbVertexDigest, usize>,
    ) -> Self {
        Self {
            actual,
            witness,
            vertices,
            used: [false; MAX_PLAN_NODES],
        }
    }
    pub(super) fn bind(
        &mut self,
        slot: i64,
        expected: Option<NodeIndex>,
    ) -> Result<(), CheckError> {
        let Some(expected) = expected else {
            return if slot == -1 {
                Ok(())
            } else {
                Err(CheckError::Mismatch("scratch input substitution"))
            };
        };
        let index =
            usize::try_from(slot).map_err(|_| CheckError::Mismatch("negative input index"))?;
        let actual = self
            .actual
            .get(index)
            .ok_or(CheckError::Mismatch("input index out of bounds"))?;
        let expected = self
            .witness
            .get(expected.index())
            .ok_or(CheckError::Mismatch("missing producer witness"))?;
        let actual_vertex = *self
            .vertices
            .get(actual.digest.as_str())
            .ok_or(CheckError::Mismatch("dangling vertex digest"))?;
        let expected_vertex = *self
            .vertices
            .get(&expected.vertex)
            .ok_or(CheckError::Mismatch("dangling producer witness"))?;
        if !input_binding(
            InputBinding {
                vertex: VertexIndex::new(actual_vertex),
                output: actual.index,
            },
            InputBinding {
                vertex: VertexIndex::new(expected_vertex),
                output: expected.output,
            },
        ) {
            return Err(CheckError::Mismatch(
                "producer edge or output index substitution",
            ));
        }
        self.used[index] = true;
        Ok(())
    }
    pub(super) fn finish(self) -> Result<(), CheckError> {
        if self.used[..self.actual.len()].iter().all(|used| *used) {
            Ok(())
        } else {
            Err(CheckError::Mismatch("unexpected producer edge"))
        }
    }
}
