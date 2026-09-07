//! Strict release selection and bounded executable version evidence.

use gripsack_process::{Control, Limits, StopReason};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub(super) fn select(root: &Path) -> io::Result<PathBuf> {
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if entry.file_name() == "grip" && kind.is_file() {
            candidates.push(entry.path());
        }
        if kind.is_dir() {
            // Enumerate rather than exists()/is_file(): permission and I/O
            // failures must not silently alter the number of candidates.
            for child in std::fs::read_dir(entry.path())? {
                let child = child?;
                let kind = child.file_type()?;
                if child.file_name() == "grip" && kind.is_file() {
                    candidates.push(child.path());
                }
            }
        }
    }
    if candidates.len() != 1 {
        return Err(io::Error::other(format!(
            "the release tarball has no single regular grip binary (found {})",
            candidates.len()
        )));
    }
    Ok(candidates.remove(0))
}

pub(super) fn version(executable: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(executable).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return Err(format!(
            "{} is not a regular executable",
            executable.display()
        ));
    }
    let mut command = Command::new(executable);
    command.arg("--version");
    let limits = Limits {
        timeout: Duration::from_secs(10),
        input_bytes: 0,
        line_bytes: 1024,
        stdout_bytes: 4096,
        stderr_bytes: 4096,
        retained_stderr_bytes: 1024,
    };
    let mut lines = Vec::new();
    let outcome = gripsack_process::run(&mut command, &[], limits, |line| {
        lines.push(line.to_vec());
        Control::Continue
    })
    .map_err(|e| format!("cannot execute {} --version: {e}", executable.display()))?;
    if !matches!(outcome.reason, StopReason::Exited) || !outcome.status.is_some_and(|s| s.success())
    {
        return Err(format!(
            "{} --version failed ({:?}): {}",
            executable.display(),
            outcome.reason,
            String::from_utf8_lossy(&outcome.stderr)
        ));
    }
    parse_output(&lines)
        .ok_or_else(|| format!("{} returned an invalid grip version", executable.display()))
}

fn parse_output(lines: &[Vec<u8>]) -> Option<String> {
    let [line] = lines else {
        return None;
    };
    let text = std::str::from_utf8(line).ok()?;
    let mut words = text.split_whitespace();
    if words.next()? != "grip" {
        return None;
    }
    let version = words.next()?;
    if words.next().is_some() || super::parse_version(version).is_none() {
        return None;
    }
    Some(version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_nested_candidates_are_ambiguous() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("grip"), b"root").unwrap();
        std::fs::create_dir(root.path().join("release")).unwrap();
        std::fs::write(root.path().join("release/grip"), b"nested").unwrap();
        assert!(select(root.path()).is_err());
        std::fs::remove_file(root.path().join("grip")).unwrap();
        assert_eq!(
            select(root.path()).unwrap(),
            root.path().join("release/grip")
        );
    }

    #[test]
    fn symlink_files_and_symlink_directories_are_not_candidates() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("grip"), b"outside").unwrap();
        std::os::unix::fs::symlink(outside.path().join("grip"), root.path().join("grip")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("release")).unwrap();
        assert!(select(root.path()).is_err());
    }

    #[test]
    fn probe_requires_success_and_a_single_grip_version() {
        let root = tempfile::tempdir().unwrap();
        let exe = root.path().join("grip");
        std::fs::write(&exe, b"#!/bin/sh\nprintf 'grip 0.37.0\\n'\n").unwrap();
        assert!(version(&exe).is_err());
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(version(&exe).unwrap(), "0.37.0");
        std::fs::write(&exe, b"#!/bin/sh\nprintf 'grip 0.37.0\\n'\nexit 1\n").unwrap();
        assert!(version(&exe).is_err());
        assert!(parse_output(&[b"grip 0.37.0".to_vec(), b"extra".to_vec()]).is_none());
        assert!(parse_output(&[b"other 0.37.0".to_vec()]).is_none());
    }

    #[test]
    fn flooding_version_peer_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let exe = root.path().join("grip");
        std::fs::write(&exe, b"#!/bin/sh\nprintf 'grip 0.37.0\\n'\nwhile :; do printf 'unexpected output\\n'; done\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(version(&exe).is_err());
    }
}
