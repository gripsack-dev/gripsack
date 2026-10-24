//! Durable self-update for release installs; other channels retain delegation.

mod installation;
mod payload;
mod verified;

use crate::render::Palette;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::ExitCode;

#[derive(Debug, clap::Args)]
pub struct SelfUpdateArgs {
    /// Verify release availability without replacing the installed executable.
    #[arg(long, conflicts_with = "reinstall")]
    check: bool,
    /// Reverify and atomically replace the same signed version.
    #[arg(long, conflicts_with = "check")]
    reinstall: bool,
    /// Select an exact stable release; older state readers are never installed.
    #[arg(long, value_parser = requested_version)]
    version: Option<String>,
}

fn requested_version(value: &str) -> Result<String, String> {
    let Some((major, minor, patch)) = parse_version(value) else {
        return Err("expected a stable version such as 0.47.0".into());
    };
    let canonical = format!("{major}.{minor}.{patch}");
    if canonical != value {
        return Err("version must use canonical major.minor.patch spelling".into());
    }
    Ok(canonical)
}

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
    let version = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    it.next().is_none().then_some(version)
}

fn should_replace(args: &SelfUpdateArgs, selected: &str, current: &str) -> Result<bool, String> {
    let selected_version = parse_version(selected).ok_or("selected release version is invalid")?;
    let current_version = parse_version(current).ok_or("installed version is invalid")?;
    if args.version.is_some() && selected_version < current_version {
        return Err(format!(
            "refusing to replace {current} with older state reader {selected}; self-update does not downgrade"
        ));
    }
    Ok(selected_version > current_version
        || (args.reinstall && selected_version == current_version))
}

pub fn self_update(palette: Palette, args: SelfUpdateArgs) -> ExitCode {
    match run(&args) {
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

fn run(args: &SelfUpdateArgs) -> Result<String, String> {
    // Resolve symlink installations to their actual target and capture before
    // any network activity, including the Linux deleted-executable case.
    let exe = installation::current_target().map_err(|e| e.to_string())?;
    let channel = channel(&exe);
    let direct = matches!(channel, Channel::Tarball | Channel::Other);
    if args.reinstall && !direct {
        return Err("--reinstall applies only to direct release installations; use the installation manager's repair operation".into());
    }
    // Check-only and delegated installs do not lock or stage an executable;
    // signature verification still needs private temporary metadata.
    let pinned = if direct && !args.check {
        Some(installation::Installation::pin(&exe).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let staging = tempfile::Builder::new()
        .prefix("grip-self-update-")
        .tempdir()
        .map_err(|e| format!("cannot stage self-update: {e}"))?;
    let release = verified::admit(staging.path(), args.version.as_deref(), true)?;
    let latest = release.version;
    if !direct || args.check {
        let current = env!("CARGO_PKG_VERSION");
        if !should_replace(args, &latest, current)? {
            return Ok(format!("grip {current} is current"));
        }
        let delegated = match channel {
            Channel::Brew => Some(("homebrew", "brew upgrade gripsack")),
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
    if !should_replace(args, &latest, &current)? {
        return Ok(format!("grip {current} is current"));
    }

    let admitted = verified::admit(staging.path(), Some(&latest), false)?;
    if admitted.version != latest || admitted.manifest_sha256 != release.manifest_sha256 {
        return Err("release manifest changed between selection and admission".into());
    }
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
    if args.reinstall && parse_version(&latest) == parse_version(&current) {
        return Ok(format!(
            "reinstalled {latest} — verified replacement takes effect on next launch"
        ));
    }
    let anchor: String = latest.chars().filter(|c| c.is_ascii_digit()).collect();
    Ok(format!(
        "updated {current} → {latest} — takes effect on next launch · what's new: gripsack.dev/docs/changelog.html#{anchor}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(version: Option<&str>, reinstall: bool) -> SelfUpdateArgs {
        SelfUpdateArgs {
            check: false,
            reinstall,
            version: version.map(str::to_owned),
        }
    }

    #[test]
    fn same_version_replacement_requires_explicit_reinstall() {
        assert!(!should_replace(&options(None, false), "0.47.0", "0.47.0").unwrap());
        assert!(should_replace(&options(Some("0.47.0"), true), "0.47.0", "0.47.0").unwrap());
        assert!(should_replace(&options(None, false), "0.48.0", "0.47.0").unwrap());
    }

    #[test]
    fn explicit_old_reader_is_never_a_reinstall() {
        for reinstall in [false, true] {
            assert!(
                should_replace(&options(Some("0.46.0"), reinstall), "0.46.0", "0.47.0").is_err()
            );
        }
        assert!(!should_replace(&options(None, false), "0.46.0", "0.47.0").unwrap());
    }

    #[test]
    fn requested_versions_cannot_be_paths_refs_or_noncanonical_aliases() {
        for invalid in [
            "../0.47.0",
            "v0.47.0",
            "00.47.0",
            "0.47.0-beta",
            "0.47.0\n",
            "0.47",
            "0.47.0.1",
        ] {
            assert!(requested_version(invalid).is_err(), "{invalid:?}");
        }
    }
}
