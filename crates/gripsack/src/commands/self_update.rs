//! `grip self-update` — a package manager that can't update itself is
//! only half one (rootle's plans/0017 model, ported): tarball installs
//! self-update through our own verified fetcher; brew/cargo/mise
//! installs get their manager's command instead.

use crate::render::Palette;
use owo_colors::OwoColorize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// How this binary was installed — decides whether we swap it or
/// defer to the package manager that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    /// install.sh / release tarball — self-updates.
    Tarball,
    Brew,
    Cargo,
    Mise,
    /// Unknown layout — self-update conservatively.
    Other,
}

fn channel(exe: &Path) -> Channel {
    let s = exe.to_string_lossy();
    if s.contains(".cargo/bin") {
        Channel::Cargo
    } else if s.contains("Cellar") || s.contains("homebrew") || s.contains("linuxbrew") {
        Channel::Brew
    } else if s.contains("/mise/") {
        Channel::Mise
    } else if s.contains("/.local/") || s.contains("/usr/local/") {
        Channel::Tarball
    } else {
        Channel::Other
    }
}

/// (major, minor, patch) — suffixes don't order.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim_start_matches('v').split('.');
    Some((
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    ))
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

pub fn self_update(palette: Palette, check_only: bool) -> ExitCode {
    let colored = palette.enabled;
    match run(check_only) {
        Ok(line) => {
            if colored {
                println!("{}", line.green());
            } else {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if colored {
                eprintln!("{} {e}", "grip:".red().bold());
            } else {
                eprintln!("grip: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(check_only: bool) -> Result<String, String> {
    let current = env!("CARGO_PKG_VERSION");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let channel = channel(&exe);
    let release = gripsack_fetch::resolve_self_release()
        .map_err(|e| format!("cannot check for updates: {e}"))?;
    let latest = release.version.as_str();
    if !is_newer(latest, current) {
        return Ok(format!("grip {current} is current"));
    }
    let (how, cmd) = match channel {
        Channel::Brew => ("homebrew", "brew upgrade --cask gripsack"),
        Channel::Cargo => ("cargo", "cargo install gripsack"),
        Channel::Mise => ("mise", "mise up gripsack"),
        _ => ("", ""),
    };
    if !matches!(channel, Channel::Tarball | Channel::Other) {
        return Ok(format!(
            "{current} → {latest} — you installed via {how}: run `{cmd}`"
        ));
    }
    if check_only {
        return Ok(format!(
            "{current} → {latest} available (run `grip self-update`)"
        ));
    }

    // Dogfood: the same sha256-verified tarball fetch every module
    // gets, then a staged write + atomic rename over self (the running
    // process keeps the old inode; the next launch runs the new one).
    let spec = gripsack_ir::FetchSpec::Tarball {
        url: release.url,
        sha256: Some(release.sha256),
        api_url: release.api_url,
    };
    let staged_dir = exe_dir(&exe)?.join(".grip-self-update");
    let _ = std::fs::remove_dir_all(&staged_dir);
    std::fs::create_dir_all(&staged_dir).map_err(|e| e.to_string())?;
    gripsack_fetch::fetch(&spec, &staged_dir)
        .map_err(|e| format!("download/verify failed: {e}"))?;
    let new_grip = staged_dir.join("grip");
    if !new_grip.is_file() {
        let _ = std::fs::remove_dir_all(&staged_dir);
        return Err("the release tarball has no grip binary at its root".into());
    }
    let staged = exe.with_extension("update-tmp");
    std::fs::copy(&new_grip, &staged).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_dir_all(&staged_dir);
    std::fs::rename(&staged, &exe).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "cannot replace {}: {e} — rerun with sudo, or reinstall via install.sh",
                exe.display()
            )
        } else {
            format!("cannot replace {}: {e}", exe.display())
        }
    })?;
    // keepachangelog anchor: ## [0.16.3] → #0163
    let anchor: String = latest.chars().filter(|c| c.is_ascii_digit()).collect();
    Ok(format!(
        "updated {current} → {latest} — takes effect on next launch · what's new: gripsack.dev/docs/changelog.html#{anchor}"
    ))
}

fn exe_dir(exe: &Path) -> Result<PathBuf, String> {
    exe.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "the running binary has no parent directory".to_string())
}
