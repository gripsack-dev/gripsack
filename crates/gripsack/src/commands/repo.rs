//! `--repo` resolution: local path, or a git URL cloned into a cache
//! under `$GRIPSACK_HOME/repos/` (0001 §5 — the bootstrap story:
//! `grip apply --repo git@github.com:you/myenv`).

use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::ExitCode;

/// Resolve an optional --repo spec to a directory with an env repo in it.
pub fn resolve(spec: Option<&str>) -> Result<PathBuf, ExitCode> {
    let Some(spec) = spec else {
        return std::env::current_dir().map_err(|e| {
            eprintln!("grip: cannot read current directory: {e}");
            ExitCode::FAILURE
        });
    };
    let path = PathBuf::from(spec);
    if path.exists() {
        return Ok(path);
    }
    if !looks_like_url(spec) {
        eprintln!("grip: no repo at {spec} (not a path, not a git URL)");
        return Err(ExitCode::FAILURE);
    }
    clone_or_update(spec)
}

fn looks_like_url(spec: &str) -> bool {
    spec.contains("://") || spec.starts_with("git@") || spec.ends_with(".git")
}

/// Cache dir per URL; re-runs pull (fast-forward only).
fn clone_or_update(url: &str) -> Result<PathBuf, ExitCode> {
    let hash: String = Sha256::digest(url.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .take(8)
        .collect();
    let dir = gripsack_store::gripsack_home().join("repos").join(hash);
    let ok = if dir.exists() {
        git(&[
            "-C",
            &dir.display().to_string(),
            "pull",
            "--ff-only",
            "--quiet",
        ])
    } else {
        let parent = dir.parent().expect("repos dir");
        let _ = std::fs::create_dir_all(parent);
        git(&["clone", "--quiet", url, &dir.display().to_string()])
    };
    if ok {
        Ok(dir)
    } else {
        eprintln!("grip: cannot fetch {url} (see output above)");
        Err(ExitCode::FAILURE)
    }
}

fn git(args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
