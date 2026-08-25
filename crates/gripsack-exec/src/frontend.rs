//! Frontend environment provisioning (0005 §3 step 3): a python with
//! the `gripsack` package and the repo's `[eval] deps`, without asking
//! the user to manage it. uv is one static binary — we fetch it with
//! our own verified fetcher.
//!
//! Fast path first: a `python3` that already imports gripsack wins.
//! Otherwise provision `$GRIPSACK_HOME/frontend/<hash>/` — a venv keyed
//! by the package version + deps, so spec changes rebuild and repeats
//! are free.

use gripsack_config::EnvConfig;
use std::io;
use std::path::{Path, PathBuf};

use gripsack_fetch::UV_RELEASE;

/// The python to evaluate with, provisioned if needed. Provisioning
/// failures are the caller's error to surface — never silently fall
/// back to a python3 that can't import gripsack.
pub fn ensure_python(home: &Path, config: &EnvConfig, core_version: &str) -> io::Result<PathBuf> {
    if let Ok(python) = std::env::var("GRIPSACK_PYTHON") {
        return Ok(PathBuf::from(python));
    }
    if system_python_works() && config.eval.deps.is_empty() {
        return Ok(PathBuf::from("python3"));
    }
    provision(home, config, core_version)
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
    // pinned packages only — `path` entries need no install.
    let linter_packages: Vec<&str> = config
        .linters
        .values()
        .filter_map(|l| l.package.as_deref())
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
        "--extra-index-url".into(),
        // griplint-* packages resolve here first — PyPI's project-
        // creation cap can stall them for days, and our index carries
        // versions PyPI may not have yet. Everything else falls
        // through to PyPI (the default index).
        "https://gripsack.dev/simple".into(),
        "--python".into(),
        python.to_string_lossy().into_owned(),
        format!("gripsack=={core_version}"),
    ];
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
    // [tool.uv] in the repo must not silently apply to provisioning
    let status = std::process::Command::new(program)
        .args(args)
        .current_dir(home)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} {:?} exited {status}",
            program.display(),
            args
        )))
    }
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
