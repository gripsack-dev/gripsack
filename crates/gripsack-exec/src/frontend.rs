//! Frontend runtime + embedded source (0005 §3 step 3, plan/0013
//! D2/D3): the TypeScript frontend evaluates under a sandboxed deno
//! subprocess. Only the RUNTIME provisions (deno, exactly like pixi —
//! pinned, sha256-verified); the frontend SOURCE — the driver and
//! @gripsack/core — is embedded in this binary and materialized under
//! `$GRIPSACK_HOME/frontend/ts-<version>/`, so the DSL version always
//! matches the core. A repo's own `node_modules/@gripsack/core`
//! install still wins when the driver resolves one (the
//! deliberate-pin rule, unchanged).

use std::io;
use std::path::{Path, PathBuf};

use gripsack_fetch::DENO_RELEASE;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/frontend_files.rs"));
}

/// The deno runtime for the frontend. Precedence:
///
/// 1. `GRIPSACK_DENO` — the deliberate override (site denos with
///    config the pinned download lacks; the doctor reports it);
/// 2. the pinned, sha256-verified download — the DEFAULT. Two
///    "identical" machines must eval through the same runtime; a
///    PATH deno used to win here silently, skewing exactly the
///    piece the pin exists to constrain;
/// 3. a deno on PATH (major ≥ 2) — last-resort fallback, used only
///    when the pinned one is unavailable (musl host, failed
///    download), and LOUD: a run-log warning, never a silent skew.
pub fn ensure_deno(home: &Path) -> io::Result<PathBuf> {
    if let Ok(deno) = std::env::var("GRIPSACK_DENO") {
        return Ok(PathBuf::from(deno));
    }
    if let Some(deno) = pinned_deno(home)? {
        return Ok(deno);
    }
    if let Some((deno, version)) = deno_on_path() {
        tracing::warn!(
            runtime = %version,
            "pinned deno unavailable — using deno {version} from PATH; \
             set GRIPSACK_DENO to make this deliberate"
        );
        return Ok(deno);
    }
    Err(io::Error::other(
        "no usable deno: the pinned runtime is unavailable on this platform and \
         none is on PATH — set GRIPSACK_DENO to point at one (see `grip doctor`)",
    ))
}

/// The pinned runtime: already provisioned, or downloaded (flock'd,
/// sha256-verified). Ok(None) when this platform can't have it
/// (musl) or the download failed and a caller fallback should get a
/// chance — the download error is preserved for the no-fallback case
/// by the caller re-trying and surfacing it.
fn pinned_deno(home: &Path) -> io::Result<Option<PathBuf>> {
    // deno ships glibc + macOS builds only: a downloaded binary on a
    // musl host (alpine, …) would fail at exec with an opaque loader
    // error — fail before the network round-trip, with the fix named
    if crate::facts::detect().libc.as_deref() == Some("musl") {
        return Err(io::Error::other(
            "deno ships no musl build — the eval sandbox needs glibc Linux or macOS \
             (see `grip doctor`)",
        ));
    }
    let dir = home
        .join("tools")
        .join(format!("deno-{}", DENO_RELEASE.version));
    let deno = dir.join("deno");
    if deno.exists() {
        return Ok(Some(deno));
    }
    // two concurrent applies may both provision: serialize the
    // download, then re-check — the loser of the race just uses the
    // winner's binary (e2e: concurrent applies, os error 26/2)
    let _provision_lock = crate::util::FlockGuard::acquire(home, "provision-deno")?;
    if deno.exists() {
        return Ok(Some(deno));
    }
    // dogfood: per-platform pinned + sha256-verified through our own
    // fetcher (host.rs)
    let (url, sha) = gripsack_fetch::resolve_host_asset(&DENO_RELEASE).map_err(io::Error::other)?;
    let spec = gripsack_ir::FetchSpec::Tarball {
        url,
        api_url: None,
        sha256: Some(sha.to_string()),
    };
    let staging = dir.with_file_name(format!(
        "{}.staging",
        dir.file_name().unwrap_or_default().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&staging);
    match gripsack_fetch::fetch(&spec, &staging).map_err(io::Error::other) {
        Ok(_) => {
            // the zip holds `deno` at the root (verified against the
            // v2.9.6 asset layout at pin time)
            std::fs::create_dir_all(&dir)?;
            std::fs::rename(staging.join("deno"), &deno)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&deno, std::fs::Permissions::from_mode(0o755))?;
            }
            #[cfg(target_os = "macos")]
            {
                // Gatekeeper marks downloaded archives' contents with
                // a quarantine xattr; an affected binary can be
                // killed or translocated at exec. The download is
                // sha256-verified, so the attribute carries no
                // security value here — strip it best-effort (the
                // xattr CLI ships with macOS; absence is harmless).
                let _ = std::process::Command::new("xattr")
                    .arg("-d")
                    .arg("com.apple.quarantine")
                    .arg(&deno)
                    .status();
            }
            let _ = std::fs::remove_dir_all(&staging);
            Ok(Some(deno))
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            // the caller falls back to a PATH deno (loudly); if there
            // is none, this error is the real cause — re-surface it
            if deno_on_path().is_some() {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

/// The minimum deno major a PATH fallback must report — the spawn
/// contract is written against deno 2.
const DENO_MIN_MAJOR: u64 = 2;
/// A runnable deno on PATH reporting a version >= DENO_MIN_MAJOR,
/// with that version string for the fallback warning.
fn deno_on_path() -> Option<(PathBuf, String)> {
    let out = std::process::Command::new("deno")
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // "deno 2.9.6 (release, …)" — the token after the name
    let version = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .map(str::to_string)?;
    let major = version.split('.').next()?.parse::<u64>().ok()?;
    (major >= DENO_MIN_MAJOR).then_some((PathBuf::from("deno"), version))
}

/// The embedded frontend, materialized under
/// `$GRIPSACK_HOME/frontend/ts-<version>/` — the typescript source
/// tree (driver + @gripsack/core) compiled into the binary, plus the
/// frontend's deno.json import map when the tree ships one. None when
/// this binary lacks the embed (crates.io builds don't carry the
/// repo's typescript tree). Per-file atomic writes with a marker
/// landing last: two concurrent grips can interleave, but never
/// observe a torn file; a partial materialization is redone next run.
pub fn ensure_ts_frontend(home: &Path, core_version: &str) -> io::Result<Option<PathBuf>> {
    if embedded::FRONTEND_FILES.is_empty() {
        return Ok(None);
    }
    let dir = home.join("frontend").join(format!("ts-{core_version}"));
    if dir.join(".complete").exists() {
        return Ok(Some(dir));
    }
    // same concurrent-apply race as ensure_deno: another grip may be
    // EXECUTING these files while we write them (ETXTBSY)
    let _materialize_lock = crate::util::FlockGuard::acquire(home, "provision-frontend")?;
    if dir.join(".complete").exists() {
        return Ok(Some(dir));
    }
    for (rel, source) in embedded::FRONTEND_FILES {
        let dest = dir.join(rel);
        gripsack_fs::atomic_write_at(&dest, source.as_bytes())?;
    }
    gripsack_fs::atomic_write_at(&dir.join(".complete"), b"ok\n")?;
    Ok(Some(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_deno_version_gate_matches_major() {
        for (line, ok) in [
            (
                "deno 2.9.6 (release, x86_64-unknown-linux-gnu) v8 13.x",
                true,
            ),
            ("deno 3.0.0 (release, aarch64-apple-darwin) v8 14.x", true),
            ("deno 1.46.3 (release, x86_64-unknown-linux-gnu) v8", false),
            ("garbage", false),
        ] {
            // mirror deno_on_path's parse over a fixed first line
            let major = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.split('.').next())
                .and_then(|m| m.parse::<u64>().ok());
            assert_eq!(
                major.is_some_and(|m| m >= DENO_MIN_MAJOR),
                ok,
                "line: {line}"
            );
        }
    }

    #[test]
    fn embed_materializes_tree_and_marker() {
        let home = tempfile::tempdir().unwrap();
        let dir = ensure_ts_frontend(home.path(), "9.9.9")
            .unwrap()
            .expect("embed present");
        assert!(dir.join(".complete").exists());
        // the driver is part of the embedded tree
        assert!(
            dir.join("src/cli.ts").is_file(),
            "driver src/cli.ts materialized"
        );
        // repeat is a no-op that still resolves
        let again = ensure_ts_frontend(home.path(), "9.9.9")
            .unwrap()
            .expect("embed present");
        assert_eq!(again, dir);
    }
}
