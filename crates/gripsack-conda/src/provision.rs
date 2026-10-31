//! Lazy provisioning of the separately released Conda helper. The shared
//! FetchContext bootstrap verifies compiled-in pins on every use, including
//! offline reuse. A mirror changes the origin only; explicit helper selection
//! remains operator authority in the client.

use gripsack_fetch::FetchContext;
use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// A provisioned helper: exact pinned bytes in a durable slot.
pub struct ProvisionedHelper {
    pub path: PathBuf,
    pub pin: String,
}

/// This core's sole approved helper identity for the current native platform.
/// Resolving the compiled-in release table performs no IO and does not consult
/// operator overrides or mirrors. Portable publishers cannot select this pin.
pub fn approved_pin() -> Result<gripsack_process::Sha256Digest, io::Error> {
    let (_, pin) = gripsack_fetch::host::resolve(&gripsack_fetch::host::CONDA_RELEASE)
        .map_err(io::Error::other)?;
    gripsack_process::Sha256Digest::parse(pin)
}

/// Provision this core release's helper through the shared bounded bootstrap.
pub fn ensure(home: &Path, context: &FetchContext) -> Result<ProvisionedHelper, io::Error> {
    let helper = gripsack_fetch::conda_helper::ensure(
        home,
        context,
        Instant::now() + Duration::from_secs(900),
    )
    .map_err(io::Error::other)?;
    Ok(ProvisionedHelper {
        path: helper.path,
        pin: helper.pin,
    })
}
