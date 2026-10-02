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

/// Provision this core release's helper through the shared bounded bootstrap.
pub fn ensure(home: &Path, context: &FetchContext) -> Result<ProvisionedHelper, io::Error> {
    let helper = gripsack_fetch::bridge::ensure_conda(
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
