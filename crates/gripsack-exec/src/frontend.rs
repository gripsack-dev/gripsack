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

const UV_VERSION: &str = "0.12.5";
const UV_SHA256: &str = "a4742988791c9aeae68c78150d6cba762062ad2a47e53738c2779d2b596bfcdb";

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
    let spec = format!("gripsack=={core_version};{}", config.eval.deps.join(";"));
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
    let args: Vec<&str> = install.iter().map(String::as_str).collect();
    run(&uv, &args)?;
    Ok(python)
}

fn ensure_uv(home: &Path) -> io::Result<PathBuf> {
    let dir = home.join("tools").join(format!("uv-{UV_VERSION}"));
    let uv = dir.join("uv");
    if uv.exists() {
        return Ok(uv);
    }
    // dogfood: pinned + sha256-verified through our own fetcher
    let url = format!(
        "https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/uv-x86_64-unknown-linux-musl.tar.gz"
    );
    let spec = gripsack_ir::FetchSpec::Tarball {
        url,
        sha256: Some(UV_SHA256.to_string()),
    };
    let staging = dir.with_extension("staging");
    let _ = std::fs::remove_dir_all(&staging);
    gripsack_fetch::fetch(&spec, &staging).map_err(io::Error::other)?;
    // the tarball contains uv-x86_64-unknown-linux-musl/{uv,uvx}
    let nested = staging.join("uv-x86_64-unknown-linux-musl");
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
