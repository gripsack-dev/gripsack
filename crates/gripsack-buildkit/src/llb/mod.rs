//! Independent validation of the actual upstream protobuf, not a second LLB
//! generator. Every accepted operation is related to its admitted plan node.
mod operations;
mod structure;
#[cfg(test)]
mod tests;
#[allow(clippy::all, unused_attributes, dead_code, non_snake_case)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/llb/mod.rs"));
}
use generated::ops as pb;

use crate::identity::{DefinitionDigest, ExporterDigest, LlbVertexDigest};
use crate::plan::{
    BuildPlan, ExporterPlan, MAX_DEFINITION_BYTES, MAX_PLAN_NODES, NodeIndex, Platform,
    ValidatedBuildPlan,
};
use crate::protocol::{Lowered, NodeWitness, SourceBinding};
use protobuf::Message;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("invalid LLB protobuf: {0}")]
    Decode(#[from] protobuf::Error),
    #[error("LLB does not implement the admitted plan: {0}")]
    Mismatch(&'static str),
    #[error("unsupported LLB field {field} in {message}")]
    UnsupportedField { message: &'static str, field: u32 },
}

/// Its private fields bind the exact checked bytes/options/source identities.
/// No public deserializer or constructor can manufacture execution authority.
#[derive(Debug)]
pub struct CheckedDefinition {
    definition: Vec<u8>,
    digest: DefinitionDigest,
    exporter: ExporterPlan,
    exporter_digest: ExporterDigest,
    platform: Platform,
    sources: Vec<SourceBinding>,
    witness: Vec<NodeWitness>,
}
impl CheckedDefinition {
    pub fn validate(plan: &ValidatedBuildPlan, lowered: Lowered) -> Result<Self, CheckError> {
        let expected = plan.plan();
        if lowered.definition.is_empty() || lowered.definition.len() > MAX_DEFINITION_BYTES {
            return Err(CheckError::Mismatch("definition byte bound"));
        }
        if lowered.exporter != expected.exporter {
            return Err(CheckError::Mismatch("exporter substitution"));
        }
        let definition = pb::Definition::parse_from_bytes(&lowered.definition)?;
        structure::known(&definition)?;
        if definition.def.is_empty() || definition.def.len() > MAX_PLAN_NODES + 1 {
            return Err(CheckError::Mismatch("definition vertex count"));
        }
        if lowered.witness.len() != expected.nodes.len() {
            return Err(CheckError::Mismatch("witness node coverage"));
        }
        let mut vertices = BTreeMap::new();
        let mut operations = Vec::with_capacity(definition.def.len());
        for bytes in &definition.def {
            let index = operations.len();
            if vertices.insert(LlbVertexDigest::of(bytes), index).is_some() {
                return Err(CheckError::Mismatch("duplicate serialized vertex"));
            }
            let op = pb::Op::parse_from_bytes(bytes)?;
            structure::known(&op)?;
            if op.inputs.len() > MAX_PLAN_NODES {
                return Err(CheckError::Mismatch("vertex input count"));
            }
            for input in &op.inputs {
                structure::known(input)?;
            }
            operations.push(op);
        }
        structure::metadata(&definition, &vertices)?;
        let mut used_vertices = BTreeSet::new();
        for (index, node) in expected.nodes.iter().enumerate() {
            let witness = &lowered.witness[index];
            if witness.node.index() != index || witness.output != 0 {
                return Err(CheckError::Mismatch("witness node/output substitution"));
            }
            let position = *vertices
                .get(&witness.vertex)
                .ok_or(CheckError::Mismatch("witness names absent vertex"))?;
            used_vertices.insert(position);
            let actual = &operations[position];
            let mut inputs = structure::Inputs::new(&actual.inputs, &lowered.witness, &vertices);
            operations::operation(expected, node, actual, &mut inputs)?;
            inputs.finish()?;
        }
        // BuildKit selects the last terminal operation. An extra root, omitted
        // validator edge or terminal replacement cannot hide behind a witness.
        let terminal_index = operations.len() - 1;
        let terminal = &operations[terminal_index];
        if terminal.op.is_some()
            || terminal.inputs.len() != 1
            || used_vertices.contains(&terminal_index)
        {
            return Err(CheckError::Mismatch("terminal root shape"));
        }
        structure::constraints(terminal, expected.platform, false)?;
        let mut inputs = structure::Inputs::new(&terminal.inputs, &lowered.witness, &vertices);
        inputs.bind(0, Some(expected.root))?;
        inputs.finish()?;
        used_vertices.insert(terminal_index);
        if used_vertices.len() != operations.len() {
            return Err(CheckError::Mismatch(
                "unexpected vertices outside the admitted graph",
            ));
        }
        // No post-check re-encoding: transport receives these same bytes.
        let digest = DefinitionDigest::of(&lowered.definition);
        let sources = expected
            .nodes
            .iter()
            .filter_map(|node| match node {
                crate::plan::Node::Local { name, digest } => Some(SourceBinding {
                    name: name.clone(),
                    digest: *digest,
                }),
                _ => None,
            })
            .collect();
        let exporter_digest = lowered.exporter.digest();
        Ok(Self {
            definition: lowered.definition,
            digest,
            exporter: lowered.exporter,
            exporter_digest,
            platform: expected.platform,
            sources,
            witness: lowered.witness,
        })
    }
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub fn witness(&self) -> &[NodeWitness] {
        &self.witness
    }
    pub(crate) fn repeats(&self, lowered: &Lowered) -> bool {
        self.definition == lowered.definition
            && self.exporter == lowered.exporter
            && self.witness == lowered.witness
    }
    pub(crate) fn into_request(
        self,
        identity: &crate::identity::AttemptIdentity,
        worker: crate::protocol::WorkerBinding,
    ) -> (crate::protocol::ExecuteRequest, Vec<NodeWitness>) {
        let request = crate::protocol::ExecuteRequest {
            protocol_version: crate::protocol::PROTOCOL_VERSION,
            session: identity.session.clone(),
            attempt: identity.attempt,
            epoch: identity.epoch,
            worker,
            definition: self.definition,
            definition_digest: self.digest,
            exporter: self.exporter,
            exporter_digest: self.exporter_digest,
            sources: self.sources,
            platform: self.platform,
        };
        (request, self.witness)
    }
}
