//! Frontend environment provisioning (0005 §3 step 3): a python with
//! the `gripsack` package and the repo's `[eval] deps`, without asking
//! the user to manage it. uv is one static binary — we fetch it with
//! our own verified fetcher.
//!
//! Fast paths first, no network anywhere: a `python3` that already
//! imports gripsack wins; next the embedded frontend — the repo's own
//! `python/gripsack`, compiled into the binary and materialized as a
//! directory app (`python3 <dir>`) — so a config-only repo applies with
//! zero network and zero provisioning ("try gripsack on one dotfile"
//! must be practically lightweight, not just conceptually). Repos with
//! `[eval] deps` or packaged linters still provision
//! `$GRIPSACK_HOME/frontend/<hash>/` — a venv keyed by the package
//! version + deps, so spec changes rebuild and repeats are free.

use gripsack_config::EnvConfig;
use std::io;
use std::path::{Path, PathBuf};

use gripsack_fetch::UV_RELEASE;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/frontend_files.rs"));
}

/// The python command to evaluate with: an interpreter, plus the app
/// directory when the embedded frontend serves (`python3 <dir>` instead
/// of `python3 -m gripsack`).
pub struct FrontendPython {
    pub program: PathBuf,
    pub app_dir: Option<PathBuf>,
}

/// The python to evaluate with, provisioned only when the repo needs
/// more than the embedded frontend carries. Provisioning failures are
/// the caller's error to surface — never silently fall back to a
/// python3 that can't import gripsack.
pub fn ensure_python(
    home: &Path,
    config: &EnvConfig,
    core_version: &str,
) -> io::Result<FrontendPython> {
    if let Ok(python) = std::env::var("GRIPSACK_PYTHON") {
        return Ok(FrontendPython {
            program: PathBuf::from(python),
            app_dir: None,
        });
    }
    // A venv is only ever needed for repo-declared extras that pip
    // installs: eval deps and wheel linters. Repo-ref linters
    // (owner/repo@tag) resolve from the plugin store, not pip — they
    // must not force provisioning (caught by the linter-repo-ref e2e).
    let needs_venv = !config.eval.deps.is_empty()
        || config.linters.values().any(|l| {
            l.package
                .as_deref()
                .is_some_and(|p| gripsack_fetch::plugins::parse_ref(p).is_none())
        });
    if !needs_venv {
        if system_python_works() {
            return Ok(FrontendPython {
                program: PathBuf::from("python3"),
                app_dir: None,
            });
        }
        if let Some(app) = embedded_frontend(home, core_version)? {
            return Ok(FrontendPython {
                program: PathBuf::from("python3"),
                app_dir: Some(app),
            });
        }
    }
    Ok(FrontendPython {
        program: provision(home, config, core_version)?,
        app_dir: None,
    })
}

/// The embedded frontend, materialized as a directory app under
/// `$GRIPSACK_HOME/frontend/embed-<version>/` — `python3 <dir>` runs
/// its `__main__.py`, and the repo's modules import `gripsack` from it
/// (sys.path[0]). None when this binary lacks the embed (crates.io
/// builds) or python3 is missing/older than 3.10.
fn embedded_frontend(home: &Path, core_version: &str) -> io::Result<Option<PathBuf>> {
    if embedded::FRONTEND_FILES.is_empty() {
        return Ok(None);
    }
    let modern = std::process::Command::new("python3")
        .args([
            "-c",
            "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !modern {
        return Ok(None);
    }
    let dir = home.join("frontend").join(format!("embed-{core_version}"));
    let marker = dir.join(".complete");
    if marker.exists() {
        return Ok(Some(dir));
    }
    let pkg = dir.join("gripsack");
    std::fs::create_dir_all(&pkg)?;
    // Per-file atomic writes: two concurrent grips can interleave, but
    // never observe a torn file; the marker lands last, so a partial
    // materialization is redone next run.
    for (rel, source) in embedded::FRONTEND_FILES {
        let name = rel.rsplit('/').next().expect("file name");
        gripsack_store::atomic_write(&pkg.join(name), source.as_bytes())?;
    }
    gripsack_store::atomic_write(
        &dir.join("__main__.py"),
        b"from gripsack.__main__ import main\n\nmain()\n",
    )?;
    gripsack_store::atomic_write(&marker, b"ok\n")?;
    Ok(Some(dir))
}

fn system_python_works() -> bool {
    std::process::Command::new("python3")
        .args(["-c", "import gripsack"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn provision(home: &Path, config: &EnvConfig, core_version: &str) -> io::Result<PathBuf> {
    let uv = ensure_uv(home)?;
    // Registered linters are provisioned like eval deps (0010 §3):
    // pinned wheel packages only — `path` entries need no install, and
    // repo refs (owner/repo@tag) resolve from the plugin store.
    let linter_packages: Vec<&str> = config
        .linters
        .values()
        .filter_map(|l| l.package.as_deref())
        .filter(|p| gripsack_fetch::plugins::parse_ref(p).is_none())
        .collect();
    let spec = format!(
        "gripsack=={core_version};{};{}",
        config.eval.deps.join(";"),
        linter_packages.join(";")
    );
    let hash: String = {
        use sha2::{Digest, Sha256};
        Sha256::digest(spec.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .take(8)
            .collect()
    };
    let venv = home.join("frontend").join(hash);
    let python = venv.join("bin").join("python3");
    if python.exists() {
        return Ok(python);
    }
    run(&uv, &["venv", venv.to_str().expect("utf8 path")], home)?;
    let mut install = vec![
        "pip".into(),
        "install".into(),
        "--python".into(),
        python.to_string_lossy().into_owned(),
        format!("gripsack=={core_version}"),
    ];
    // GRIPSACK_EXTRA_INDEX: opt-in extra indexes (comma-separated),
    // e.g. https://gripsack.dev/simple for the griplint-* ecosystem
    // while PyPI's project-creation cap holds packages back. Opt-in,
    // NOT a default: a content-filtering proxy that 403s the index
    // (observed: gripsack.dev/simple behind Bloomberg's egress) is a
    // hard uv failure, which would break exactly the environments the
    // index exists to help. PyPI stays the primary everywhere.
    if let Ok(extra) = std::env::var("GRIPSACK_EXTRA_INDEX") {
        for index in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            install.push("--extra-index-url".into());
            install.push(index.into());
        }
    }
    install.extend(config.eval.deps.iter().cloned());
    install.extend(linter_packages.iter().map(|p| p.to_string()));
    let args: Vec<&str> = install.iter().map(String::as_str).collect();
    run(&uv, &args, home)?;
    Ok(python)
}

/// The minimum uv version a PATH uv must report to be preferred over
/// the pinned download (review's uv cluster: skew both ways — too old
/// is a bug, and a site config written for a newer uv breaks the pin).
const UV_MIN_VERSION: (u64, u64) = (0, 12);

/// "0.13.7" → (0, 13) — semver-lite for uv's MAJOR.MINOR scheme.
fn parse_version(v: &str) -> Option<(u64, u64)> {
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor))
}

fn ensure_uv(home: &Path) -> io::Result<PathBuf> {
    // GRIPSACK_UV is the explicit escape hatch (like GRIPSACK_PYTHON).
    if let Ok(uv) = std::env::var("GRIPSACK_UV") {
        return Ok(PathBuf::from(uv));
    }
    // A uv on PATH that satisfies the minimum understands the local
    // config already — prefer it; the pinned download is the fallback.
    if let Ok(uv) = uv_on_path() {
        return Ok(uv);
    }
    let dir = home
        .join("tools")
        .join(format!("uv-{}", UV_RELEASE.version));
    let uv = dir.join("uv");
    if uv.exists() {
        return Ok(uv);
    }
    // dogfood: per-platform pinned + sha256-verified through our own
    // fetcher (host.rs)
    let (url, sha) = gripsack_fetch::resolve_host_asset(&UV_RELEASE).map_err(io::Error::other)?;
    let spec = gripsack_ir::FetchSpec::Tarball {
        url,
        api_url: None,
        sha256: Some(sha.to_string()),
    };
    let staging = dir.with_extension("staging");
    let _ = std::fs::remove_dir_all(&staging);
    gripsack_fetch::fetch(&spec, &staging).map_err(io::Error::other)?;
    // the tarball contains uv-<triple>/{uv,uvx}
    let target = gripsack_fetch::AssetTarget::current()
        .expect("resolved above")
        .triple();
    let nested = staging.join(format!("uv-{target}"));
    std::fs::create_dir_all(&dir)?;
    std::fs::rename(nested.join("uv"), &uv)?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(uv)
}

/// A runnable uv on PATH reporting a version >= UV_MIN_VERSION.
fn uv_on_path() -> io::Result<PathBuf> {
    let out = std::process::Command::new("uv")
        .arg("--version")
        .output()
        .map_err(|e| io::Error::new(e.kind(), format!("no usable uv on PATH: {e}")))?;
    if !out.status.success() {
        return Err(io::Error::other("uv on PATH failed --version"));
    }
    let version = String::from_utf8_lossy(&out.stdout);
    let version = version.trim().trim_start_matches("uv ").to_string();
    // numeric compare, not string-prefix — 0.13/1.x are NEWER, not
    // older (the string-prefix check reintroduced the skew trap it
    // was built to avoid)
    match parse_version(&version) {
        Some(v) if v >= UV_MIN_VERSION => Ok(PathBuf::from("uv")),
        _ => Err(io::Error::other(format!(
            "uv on PATH is {version}, need >= {}.{} — provisioning the pinned one",
            UV_MIN_VERSION.0, UV_MIN_VERSION.1
        ))),
    }
}

fn run(program: &Path, args: &[&str], home: &Path) -> io::Result<()> {
    // run from $GRIPSACK_HOME, not the env repo — a stray uv.toml or
    // [tool.uv] in the repo must not silently apply to provisioning.
    // stderr comes back on failure — the CLI's corporate-environment
    // hints (index mirroring, SSL_CERT_FILE) match on it (review:
    // "I hit both failures and saw neither hint" — the hints never
    // fired because the error carried only the exit status)
    let out = std::process::Command::new(program)
        .args(args)
        .current_dir(home)
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        let tail = String::from_utf8_lossy(&out.stderr);
        // the last few lines, trimmed — one raw last line is often a
        // wrapped sentence fragment or a tool's own hint (review:
        // "a few lines of tail rather than one, trimmed")
        let tail = tail
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" | ")
            .chars()
            .take(400)
            .collect::<String>();
        Err(io::Error::other(format!(
            "{} {:?} exited {}{}",
            program.display(),
            args,
            out.status,
            if tail.is_empty() {
                String::new()
            } else {
                format!(" — {tail}")
            }
        )))
    }
}

// ── TypeScript frontend (bun) ───────────────────────────────────────

/// The bun runtime for the TypeScript frontend: GRIPSACK_BUN wins, a
/// bun on PATH next, the pinned download last (same precedence as uv —
/// a site bun with config the pinned one lacks must win).
pub fn ensure_bun(home: &Path) -> io::Result<PathBuf> {
    if let Ok(bun) = std::env::var("GRIPSACK_BUN") {
        return Ok(PathBuf::from(bun));
    }
    if std::process::Command::new("bun")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(PathBuf::from("bun"));
    }
    let dir = home
        .join("tools")
        .join(format!("bun-{}", gripsack_fetch::BUN_RELEASE.version));
    let bun = dir.join("bun");
    if bun.exists() {
        return Ok(bun);
    }
    let (url, sha) = gripsack_fetch::resolve_host_asset(&gripsack_fetch::BUN_RELEASE)
        .map_err(io::Error::other)?;
    let spec = gripsack_ir::FetchSpec::Tarball {
        url,
        api_url: None,
        sha256: Some(sha.to_string()),
    };
    let staging = dir.with_extension("staging");
    let _ = std::fs::remove_dir_all(&staging);
    gripsack_fetch::fetch(&spec, &staging).map_err(io::Error::other)?;
    // the zip contains bun-<target>/bun
    let target = gripsack_fetch::AssetTarget::current()
        .expect("resolved above")
        .bun_name();
    let nested = staging.join(format!("bun-{target}"));
    std::fs::create_dir_all(&dir)?;
    std::fs::rename(nested.join("bun"), &bun)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bun, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(bun)
}

/// The @gripsack/core package for the TypeScript frontend, pinned to
/// the core's version and installed into a spec-keyed dir — same model
/// as the python frontend venv.
pub fn ensure_ts_frontend(home: &Path, core_version: &str) -> io::Result<PathBuf> {
    // GRIPSACK_TS_FRONTEND: a directory containing node_modules/
    // @gripsack/core (dev checkouts, air-gapped mirrors) — the same
    // escape-hatch shape as GRIPSACK_PYTHON/GRIPSACK_UV/GRIPSACK_BUN
    if let Ok(dir) = std::env::var("GRIPSACK_TS_FRONTEND") {
        return Ok(PathBuf::from(dir));
    }
    let dir = home.join("frontend-ts").join(core_version);
    let pkg = dir.join("node_modules/@gripsack/core");
    if pkg.exists() {
        return Ok(dir);
    }
    let bun = ensure_bun(home)?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("package.json"),
        format!(
            "{{\"name\":\"gripsack-frontend\",\"private\":true,\"dependencies\":{{\"@gripsack/core\":\"{core_version}\"}}}}"
        ),
    )?;
    run(&bun, &["install", "--no-progress"], &dir)?;
    if !pkg.exists() {
        return Err(io::Error::other(format!(
            "bun install did not produce @gripsack/core@{core_version} — is the typescript frontend published?"
        )));
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::{UV_MIN_VERSION, parse_version};

    #[test]
    fn path_uv_preference_is_a_real_semver_compare() {
        for (v, preferred) in [
            ("0.11.15", false),
            ("0.12.5", true),
            ("0.13.0", true),
            ("0.20.0", true),
            ("1.0.0", true),
        ] {
            assert_eq!(
                parse_version(v).map(|p| p >= UV_MIN_VERSION),
                Some(preferred),
                "uv {v} preferred should be {preferred}"
            );
        }
    }
}
