//! Native compatibility of an independently reconstructed final-prefix artifact.
use super::{
    CondaRuntimeReceipt, ExecError, WorkspacePlatform, failure, host_virtual_packages,
    native_location, subdir,
};
use gripsack_ir::workspace_model::lock::LockedCondaEnvironment;
use std::path::Path;

pub(in crate::workspace) fn admit_native(
    receipt: &CondaRuntimeReceipt,
    home: &Path,
    target: &WorkspacePlatform,
    payload: &Path,
) -> Result<(), ExecError> {
    let output = "native Conda package";
    let (key, prefix) = native_location(output, home, &receipt.closure)?;
    if receipt.platform != receipt.closure.platform
        || subdir(output, target)? != receipt.platform
        || receipt.materialization != key
        || receipt.final_prefix != prefix
        || payload.canonicalize()? != prefix
    {
        return Err(failure(
            output,
            "runtime receipt does not name this home's derived native prefix and selected platform",
        ));
    }
    admit_host_requirements(&receipt.closure, target)
}

/// Check the complete locked runtime requirements against measured host facts
/// before either offline installation or managed use. Historical solver facts
/// and environment overrides never supply recipient capabilities.
pub(in crate::workspace) fn admit_host_requirements(
    locked: &LockedCondaEnvironment,
    target: &WorkspacePlatform,
) -> Result<(), ExecError> {
    let output = "native Conda package";
    if subdir(output, target)? != locked.platform {
        return Err(failure(
            output,
            "locked Conda platform differs from the native target",
        ));
    }
    let mut facts = host_virtual_packages(output, target)?;
    // Solving intentionally uses the portable architecture baseline. Native
    // admission measures the real CPU; no lock or CONDA_OVERRIDE_* is authority.
    let architecture = gripsack_conda::virtuals::measured_architecture()
        .map_err(|error| failure(output, error.to_string()))?;
    if let Some(fact) = facts.iter_mut().find(|fact| fact.name == "__archspec") {
        *fact = architecture;
    } else {
        facts.push(architecture);
    }
    gripsack_conda::evaluate_requirements(&locked.packages, &facts)
        .map_err(|error| failure(output, error.to_string()))?;
    gripsack_conda::virtuals::evaluate_system_requirements(&locked.system_requirements, &facts)
        .map_err(|error| failure(output, error.to_string()))
}
