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
    run(&uv, &["venv", venv.to_str().expect("utf8 path")])?;
    let mut install = vec![
        "pip".into(),
        "install".into(),
        "--python".into(),
        python.to_string_lossy().into_owned(),
        format!("gripsack=={core_version}"),
    ];
    install.extend(config.eval.deps.iter().cloned());
    install.extend(linter_packages.iter().map(|p| p.to_string()));
    let args: Vec<&str> = install.iter().map(String::as_str).collect();
    run(&uv, &args)?;
    Ok(python)
}

fn ensure_uv(home: &Path) -> io::Result<PathBuf> {
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

fn run(program: &Path, args: &[&str]) -> io::Result<()> {
    let status = std::process::Command::new(program).args(args).status()?;
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
