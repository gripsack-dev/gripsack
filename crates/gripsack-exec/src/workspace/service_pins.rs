//! Immutable OCI references are recorded only by explicit workspace update.
//! Service startup reads the same approved lock/definition boundary as packages.
use super::{definitions::captured_definitions, pins::WorkspacePins, selection::Selection};
use crate::{Ctx, ExecError};
use gripsack_ir::{
    Ir,
    workspace::{PlatformArch, PlatformOs, WorkspacePlatform},
    workspace_model::{
        LockedSource, ServiceOutput, WorkspaceOutput,
        lock::{LockedPin, ResolvedPinFields},
    },
};

pub(super) fn platform() -> WorkspacePlatform {
    WorkspacePlatform {
        os: PlatformOs::Linux,
        arch: PlatformArch::X86_64,
        abi: None,
        minimum_os: None,
    }
}

pub(super) fn pin(service: &ServiceOutput) -> Result<LockedPin, ExecError> {
    let digest = service
        .image
        .rsplit_once("@sha256:")
        .map(|(_, digest)| digest)
        .filter(|_| gripsack_ir::workspace_model::valid_service_image(&service.image))
        .ok_or_else(|| failure("service image must be an immutable name@sha256 reference"))?;
    Ok(LockedPin {
        output: service.name.clone(),
        source: LockedSource::OciImage {
            reference: service.image.clone(),
        },
        resolved: ResolvedPinFields {
            url: Some(service.image.clone()),
            sha256: Some(digest.to_owned()),
            ..Default::default()
        },
        conda: None,
    })
}

pub(crate) fn admit(ir: &Ir, ctx: &Ctx, names: &[String]) -> Result<(), ExecError> {
    let all;
    let names = if names.is_empty() {
        all = ir
            .workspace
            .outputs
            .iter()
            .filter_map(|output| match output {
                WorkspaceOutput::Service(service) => Some(service.name.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        &all
    } else {
        names
    };
    let selected = Selection::admit(&ir.workspace, names)?;
    let pins = WorkspacePins::read(&ctx.repository)?;
    pins.admit_definitions(&captured_definitions(&ctx.repository)?)?;
    for name in &selected.required {
        if let WorkspaceOutput::Service(service) = selected.outputs[name] {
            let expected = pin(service)?;
            let actual = pins.lookup(&platform(), name, &expected.source)?
                .ok_or_else(|| failure("service image has no frozen pin; run grip update and approve the changed lock before grip up"))?;
            if actual != &expected {
                return Err(failure(
                    "service image pin differs from its admitted declaration; explicit grip update is required",
                ));
            }
        }
    }
    Ok(())
}
fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        output: "services".into(),
        step: "frozen image".into(),
        detail: detail.into(),
    }
}
