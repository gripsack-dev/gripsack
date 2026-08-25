//! `pixi:` — conda packages via pixi into an isolated PIXI_HOME,
//! harvested into the store. Pinning: explicit `==<version>` plus the
//! resulting tree hash (verified against the lock by the executor).
//!
//! PIXI_HOME is FIXED at `$GRIPSACK_HOME/tools/pixi` — pixi embeds
//! its home path in the harvested tree (conda-meta), so a per-run
//! temp dir makes the same pin hash differently every apply and the
//! lock can never hold (review finding F7). pixi is a host tool, like
//! git: gripsack runs it, doesn't provision it. Its environment is
//! inherited (SSL_CERT_FILE flows); concurrent installs serialize on
//! the `pixi-lock` resource when declared (0007 §4).

use super::FetchError;
use super::archive;
use std::path::Path;

pub(crate) fn fetch(
    package: &str,
    version: Option<&str>,
    dest: &Path,
) -> Result<String, FetchError> {
    let spec = match version {
        Some(v) => format!("{package}=={v}"),
        None => package.to_string(),
    };
    let pixi_home = gripsack_store::gripsack_home().join("tools/pixi");
    let status = std::process::Command::new("pixi")
        .args(["global", "install", &spec])
        .env("PIXI_HOME", &pixi_home)
        .status()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => FetchError::Http {
                url: package.to_string(),
                reason: "pixi not found on PATH — the pixi fetcher needs it as a host tool (https://pixi.sh)".to_string(),
            },
            _ => FetchError::Io(e),
        })?;
    if !status.success() {
        return Err(FetchError::Http {
            url: package.to_string(),
            reason: format!("pixi global install exited {status}"),
        });
    }
    let env_dir = pixi_home.join("envs").join(package);
    if !env_dir.exists() {
        return Err(FetchError::Http {
            url: package.to_string(),
            reason: format!("no pixi env at {}", env_dir.display()),
        });
    }
    archive::copy_tree(&env_dir, dest).map_err(FetchError::Io)?;
    Ok(gripsack_store::canonical_tree_hash(dest)?)
}
