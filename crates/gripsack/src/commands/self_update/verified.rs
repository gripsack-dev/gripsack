//! Execute only verifier code compiled into this core, never a remote installer.

use gripsack_process::{Control, Limits, StopReason};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Release {
    pub version: String,
    pub manifest_sha256: String,
}

pub(super) fn admit(staging: &Path, version: Option<&str>, check: bool) -> Result<Release, String> {
    let installer = staging.join("verified-installer.sh");
    std::fs::write(&installer, include_bytes!("installer.sh"))
        .map_err(|error| format!("cannot stage embedded verifier: {error}"))?;
    let mut command = Command::new("sh");
    command.arg(&installer);
    if check {
        command.arg("--check");
    } else {
        command.arg("--stage").arg(staging);
    }
    if let Some(version) = version {
        command.arg("--version").arg(version);
    }
    let limits = Limits {
        timeout: Duration::from_secs(3600),
        operation_deadline: None,
        input_bytes: gripsack_process::InputByteLimit::new(0),
        line_bytes: gripsack_process::FrameByteLimit::new(8192),
        stdout_bytes: gripsack_process::StdoutByteLimit::new(8192),
        stderr_bytes: gripsack_process::StderrByteLimit::new(64 * 1024),
        retained_stderr_bytes: gripsack_process::RetainedStderrLimit::new(8192),
    };
    let mut output = Vec::new();
    let outcome = gripsack_process::run(&mut command, &[], limits, |line| {
        output.extend_from_slice(line);
        output.push(b'\n');
        Control::Continue
    })
    .map_err(|error| format!("cannot run embedded release verifier: {error}"))?;
    if !matches!(outcome.reason, StopReason::Exited)
        || !outcome.status.is_some_and(|status| status.success())
    {
        return Err(format!(
            "signed release admission failed ({:?}): {}",
            outcome.reason,
            String::from_utf8_lossy(&outcome.stderr)
        ));
    }
    let release: Release = serde_json::from_slice(&output)
        .map_err(|error| format!("invalid embedded verifier receipt: {error}"))?;
    if super::parse_version(&release.version).is_none()
        || release.manifest_sha256.len() != 64
        || !release
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid verified release identity".into());
    }
    Ok(release)
}
