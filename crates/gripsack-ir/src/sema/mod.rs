//! Validate the current workspace before any realization or deployment.

mod workspace;
mod workspace_model;

use crate::{Diagnostic, Ir, Severity};

pub fn run(ir: &Ir) -> Vec<Diagnostic> {
    if let Err(diagnostic) = crate::authoring::admit(ir) {
        return vec![diagnostic];
    }
    let mut diagnostics = Vec::new();
    workspace_model::check(&ir.workspace, &mut diagnostics);
    diagnostics
}

pub fn check(json: &str) -> Result<Ir, Vec<Diagnostic>> {
    let ir = crate::parse(json).map_err(|diagnostic| vec![diagnostic])?;
    let mut diagnostics = run(&ir);
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        let owners = ir.output_owners();
        for diagnostic in &mut diagnostics {
            owners.attribute(diagnostic);
        }
        return Err(diagnostics);
    }
    Ok(ir)
}
