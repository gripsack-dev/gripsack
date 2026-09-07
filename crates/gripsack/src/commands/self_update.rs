//! Durable self-update for release installs; other channels retain delegation.

mod installation;
mod payload;

use crate::render::Palette;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Tarball,
    Brew,
    Cargo,
    Mise,
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

/// Retain the existing release ordering contract.
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
    match run(check_only) {
        Ok(line) => {
            if palette.enabled {
                println!("{}", line.green());
            } else {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if palette.enabled {
                eprintln!("{} {e}", "grip:".red().bold());
            } else {
                eprintln!("grip: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(check_only: bool) -> Result<String, String> {
    // Resolve symlink installations to their actual target and capture before
    // any network activity, including the Linux deleted-executable case.
    let exe = installation::current_target().map_err(|e| e.to_string())?;
    let channel = channel(&exe);
    let direct = matches!(channel, Channel::Tarball | Channel::Other);
    // Check-only and delegated installs neither acquire a lock nor stage data.
    let pinned = if direct && !check_only {
        Some(installation::Installation::pin(&exe).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let context = gripsack_fetch::FetchContext::default();
    let release = context
        .resolve_self_release()
        .map_err(|e| format!("cannot check for updates: {e}"))?;
    let latest = release.version.clone();
    if !direct || check_only {
        let current = env!("CARGO_PKG_VERSION");
        if !is_newer(&latest, current) {
            return Ok(format!("grip {current} is current"));
        }
        let delegated = match channel {
            Channel::Brew => Some(("homebrew", "brew upgrade --cask gripsack")),
            Channel::Cargo => Some(("cargo", "cargo install gripsack")),
            Channel::Mise => Some(("mise", "mise up gripsack")),
            _ => None,
        };
        return Ok(match delegated {
            Some((how, cmd)) => {
                format!("{current} → {latest} — you installed via {how}: run `{cmd}`")
            }
            None => format!("{current} → {latest} available (run `grip self-update`)"),
        });
    }

    let installed = pinned.expect("direct mutation pins the installation");
    let _lock = installed
        .lock()
        .map_err(|e| format!("cannot lock executable: {e}"))?;
    // The running process's build version is not evidence of what is now at
    // the install path. Probe and compare stable installed metadata under lock.
    let before = installed.snapshot().map_err(|e| e.to_string())?;
    let current = payload::version(&exe)?;
    installed
        .require_snapshot(&before)
        .map_err(|e| e.to_string())?;
    if !is_newer(&latest, &current) {
        return Ok(format!("grip {current} is current"));
    }

    let staging = tempfile::Builder::new()
        .prefix("grip-self-update-")
        .tempdir()
        .map_err(|e| format!("cannot stage self-update: {e}"))?;
    let spec = gripsack_ir::FetchSpec::Tarball {
        url: release.url,
        sha256: Some(release.sha256),
        api_url: release.api_url,
    };
    context
        .fetch(&spec, staging.path(), None)
        .map_err(|e| format!("download/verify failed: {e}"))?;
    let candidate = payload::select(staging.path()).map_err(|e| e.to_string())?;
    let candidate_version = payload::version(&candidate)?;
    if candidate_version.trim_start_matches('v') != latest.trim_start_matches('v') {
        return Err(format!(
            "release binary reports {candidate_version}, expected {latest}"
        ));
    }
    let mut source = std::fs::File::open(&candidate).map_err(|e| e.to_string())?;
    installed
        .require_snapshot(&before)
        .map_err(|e| e.to_string())?;
    if let Err(e) = installed.publish(&mut source) {
        if gripsack_fs::publication_occurred(&e) {
            return Err(format!(
                "executable changed to {latest}, but publication durability could not be confirmed: {e}; no rollback attempted"
            ));
        }
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            return Err(format!(
                "cannot replace {}: {e} — rerun with sudo, or reinstall via install.sh",
                exe.display()
            ));
        }
        return Err(format!("cannot replace {}: {e}", exe.display()));
    }
    let anchor: String = latest.chars().filter(|c| c.is_ascii_digit()).collect();
    Ok(format!(
        "updated {current} → {latest} — takes effect on next launch · what's new: gripsack.dev/docs/changelog.html#{anchor}"
    ))
}
