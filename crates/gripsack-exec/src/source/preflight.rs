//! Layout evidence only: never execute recipes or verification programs.
use crate::ctx::ExecError;
use gripsack_ir::HostName;

#[derive(Debug, Default)]
pub struct LayoutEvidence {
    pub checked_paths: usize,
    pub deferred: Vec<DeferredLayoutCheck>,
}

#[derive(Debug)]
pub enum DeferredLayoutCheck {
    RecipeOutput(String),
    RuntimeVerification(&'static str),
    UnavailableArtifact,
}

impl LayoutEvidence {
    pub fn summary(&self) -> Option<String> {
        if self.checked_paths == 0 && self.deferred.is_empty() {
            return None;
        }
        let mut text = format!("layout: {} paths checked", self.checked_paths);
        for deferred in &self.deferred {
            match deferred {
                DeferredLayoutCheck::RecipeOutput(path) => {
                    text.push_str(&format!("; {path} deferred (recipe output)"))
                }
                DeferredLayoutCheck::RuntimeVerification(kind) => {
                    text.push_str(&format!("; {kind} deferred (runtime verification)"))
                }
                DeferredLayoutCheck::UnavailableArtifact => {
                    text.push_str("; deferred (matching artifact unavailable, no fetch performed)")
                }
            }
        }
        Some(text)
    }
}

/// Existing matching artifacts only. No resolver, download, build or publication.
pub fn inspect_known(
    ir: &gripsack_ir::Ir,
    repository: &crate::Repository,
    _host: &HostName,
    limits: gripsack_fetch::FetchLimits,
) -> Result<Vec<(String, LayoutEvidence)>, ExecError> {
    if let Some(diagnostic) =
        ir.workspace_execution_error(gripsack_ir::workspace::WorkspaceOperation::Plan)
    {
        return Err(ExecError::Gate(diagnostic));
    }
    crate::workspace::NativeProfiles::prepare_preview(
        ir,
        repository,
        &gripsack_store::gripsack_home(),
        &[],
        limits,
    )
    .map(|native| native.layout_evidence())
}
