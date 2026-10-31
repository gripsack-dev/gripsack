//! Current executable authoring admission, separate from retained wire decoding.
//! Historical variants remain available to receipt/identity readers, never as
//! a request to recreate retired production or select another host backend.

use crate::workspace::{PlatformAbi, PlatformArch, PlatformOs, WorkspacePlatform};
use crate::workspace_model::{AcquisitionSource, WorkspaceOutput, WorkspaceProducer};
use crate::{Diagnostic, Ir, Span, codes};

pub(crate) fn admit(ir: &Ir) -> Result<(), Diagnostic> {
    if ir.ir_version != crate::IR_VERSION {
        return Err(crate::parse::version_error(u64::from(ir.ir_version)));
    }
    for output in &ir.workspace.outputs {
        match output {
            WorkspaceOutput::Recipe(_) | WorkspaceOutput::Image(_) => {
                return Err(retired(
                    output.span(),
                    "recipe and OCI image authoring is retired",
                ));
            }
            WorkspaceOutput::Package(package) => {
                platform(&package.target, &package.span)?;
                match &package.producer {
                    WorkspaceProducer::Recipe { .. } => {
                        return Err(retired(
                            &package.span,
                            "recipe package producers are retired",
                        ));
                    }
                    WorkspaceProducer::Provider { provider } => {
                        if let AcquisitionSource::CondaEnvironment(source) = provider {
                            for target in &source.platforms {
                                platform(target, &source.span)?;
                            }
                        }
                    }
                }
            }
            WorkspaceOutput::Environment(environment) => {
                platform(&environment.target, &environment.span)?;
            }
            WorkspaceOutput::Task(_)
            | WorkspaceOutput::Schedule(_)
            | WorkspaceOutput::Check(_)
            | WorkspaceOutput::Profile(_)
            | WorkspaceOutput::Service(_)
            | WorkspaceOutput::Hook(_) => {}
        }
    }
    Ok(())
}

fn platform(target: &WorkspacePlatform, span: &Span) -> Result<(), Diagnostic> {
    if target.os != PlatformOs::Linux
        || target.arch != PlatformArch::X86_64
        || target.abi == Some(PlatformAbi::Darwin)
    {
        return Err(retired(
            span,
            "current targets must be Linux/WSL x86_64 with a Linux ABI",
        ));
    }
    Ok(())
}

fn retired(span: &Span, message: &str) -> Diagnostic {
    Diagnostic::error(codes::WORKSPACE_EXEC_UNAVAILABLE, message)
        .with_label(Some(span.clone()), "declared here")
        .with_help("use a direct fetched, imported, Conda or Pixi package; retained generations remain readable without rebuilding")
}
