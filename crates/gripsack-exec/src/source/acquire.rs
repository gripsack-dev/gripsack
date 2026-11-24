//! Immutable workspace content publication.
use crate::ctx::{Ctx, ExecError};
use std::path::Path;

pub(crate) fn publish(
    ctx: &Ctx,
    name: &str,
    stage: &Path,
    destination: &Path,
) -> Result<(), ExecError> {
    publish_with(ctx, name, stage, destination, true)
}

/// Portable imports record exact directory modes in their inventory and verify
/// them on every readmission, so their directories keep the recorded modes.
pub(crate) fn publish_unsealed(
    ctx: &Ctx,
    name: &str,
    stage: &Path,
    destination: &Path,
) -> Result<(), ExecError> {
    publish_with(ctx, name, stage, destination, false)
}

fn publish_with(
    ctx: &Ctx,
    name: &str,
    stage: &Path,
    destination: &Path,
    seal: bool,
) -> Result<(), ExecError> {
    let relative = destination
        .strip_prefix(&ctx.home)
        .map_err(|_| ExecError::Step {
            output: name.into(),
            step: "publish".into(),
            detail: format!(
                "store path {} is not under $GRIPSACK_HOME",
                destination.display()
            ),
        })?;
    let home = ctx.home_dir()?;
    gripsack_host::fs::publish_dir(home, stage, relative)?;
    // Published entries are immutable: seal directories too (files are already
    // read-only), so programs run from the store cannot add names such as
    // `__pycache__`. Maintenance removes entries through `remove_sealed_tree`.
    if seal {
        gripsack_host::fs::seal_tree(home, relative)?;
    }
    Ok(())
}
