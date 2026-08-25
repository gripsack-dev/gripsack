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
use std::path::{Path, PathBuf};

/// The bundled pixi: per-platform pinned + sha256-verified through our
/// own fetcher into `$GRIPSACK_HOME/tools/` (see host.rs) — same
/// pattern as uv (0005 §3). Bundling is what makes `grip apply` one
/// command on a clean machine; the pin also makes the fetcher itself
/// reproducible, not just the packages.
use crate::host::PIXI_RELEASE;

fn ensure_pixi() -> Result<PathBuf, FetchError> {
    let dir = gripsack_store::gripsack_home()
        .join("tools")
        .join(format!("pixi-{}", PIXI_RELEASE.version));
    let pixi = dir.join("pixi");
    if pixi.exists() {
        return Ok(pixi);
    }
    let (url, sha) = crate::host::resolve(&PIXI_RELEASE)?;
    let spec = gripsack_ir::FetchSpec::Tarball {
        url,
        sha256: Some(sha.to_string()),
    };
    let staging = dir.with_extension("staging");
    let _ = std::fs::remove_dir_all(&staging);
    crate::fetch::fetch(&spec, &staging)?;
    std::fs::create_dir_all(&dir)?;
    // pixi's tarball is flat: a bare `pixi` binary at the root
    // (unlike uv's nested uv-<triple>/ layout)
    std::fs::rename(staging.join("pixi"), &pixi)?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(pixi)
}

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
    let pixi = ensure_pixi()?;
    let status = std::process::Command::new(pixi)
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
    // conda-meta is pixi's bookkeeping, not payload — and it embeds the
    // absolute PIXI_HOME path, which would make the same pin hash
    // differently per machine (cross-machine reproducibility, 0001
    // §3.4). Excluded from the harvest AND the identity.
    archive::copy_tree_filtered(&env_dir, dest, &["conda-meta"]).map_err(FetchError::Io)?;
    Ok(gripsack_store::canonical_tree_hash(dest)?)
}
