//! Evaluator policy capture and evaluation receipts. Running grip on a repo is
//! consent (#409): nothing here gates evaluation or stores approval state.
mod audit;
pub mod evaluation;
mod policy;
#[cfg(test)]
mod tests;
mod wire;

use crate::source_bundle::SourceBundle;
pub use policy::{
    EvaluationPolicy, EvaluationPolicyDigest, EvaluatorBudget, EvaluatorProfile, FrontendIdentity,
    NativeDeclarations, ProbeCapability,
};
use std::io;
pub use wire::GitProvenance;

fn repository<'a>(bundle: &'a SourceBundle, policy: &EvaluationPolicy) -> io::Result<&'a str> {
    let path = bundle
        .repository_identity()
        .to_str()
        .ok_or_else(|| invalid("repository identity is not UTF-8"))?;
    wire::validate_repository(path)?;
    policy.validate()?;
    if policy.frontend.bundle != bundle.digest() || policy.read_roots != bundle.inventory().roots()
    {
        return Err(invalid(
            "evaluation policy is not bound to this captured source bundle",
        ));
    }
    Ok(path)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
