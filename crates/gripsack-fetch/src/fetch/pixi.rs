//! `pixi:` — conda packages via pixi into an isolated PIXI_HOME,
//! harvested into the store. Pinning: explicit `==<version>` plus the
//! resulting tree hash (verified against the lock by the executor).

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
    let pixi_home = std::env::temp_dir().join(format!("gripsack-pixi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pixi_home);
    let status = std::process::Command::new("pixi")
        .args(["global", "install", &spec])
        .env("PIXI_HOME", &pixi_home)
        .status()?;
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
    let _ = std::fs::remove_dir_all(&pixi_home);
    Ok(gripsack_store::canonical_tree_hash(dest)?)
}
