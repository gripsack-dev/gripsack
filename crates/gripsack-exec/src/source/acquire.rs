//! Immutable workspace content publication.
use crate::ctx::{Ctx, ExecError};
use std::path::Path;

pub(crate) fn publish(
    ctx: &Ctx,
    name: &str,
    stage: &Path,
    destination: &Path,
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
    gripsack_fs::publish_dir(ctx.home_dir()?, stage, relative)?;
    Ok(())
}
