//! The gripfetch NDJSON adapter. Process ownership, bounded framing and final
//! cleanup belong to gripsack_process; payload identity belongs to the core.

mod capabilities;
mod protocol;
#[cfg(test)]
mod tests;

use crate::{FetchError, FetchLimits};
use gripsack_process::{Control, Limits, StopReason};
use gripsack_store::hash::PayloadHash;
use protocol::PluginMessage;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const PLUGIN_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub(crate) struct PluginFetch {
    pub tree: PayloadHash,
    pub url: Option<String>,
    pub version: Option<String>,
}

pub(crate) fn fetch(
    name: &str,
    args: &serde_json::Value,
    dest: &Path,
    locked: Option<&serde_json::Value>,
    limits: FetchLimits,
) -> Result<PluginFetch, FetchError> {
    let exe = crate::find_fetcher(name).ok_or_else(|| {
        failure(
            name,
            format!("gripfetch-{name} not found on PATH (or declare it in env.toml)"),
        )
    })?;
    if let Some(caps) = capabilities::get(name, &exe) {
        for (domain, budget) in &caps.throttle {
            crate::throttle::acquire_declared(domain, budget);
        }
    }
    let request = protocol::FetchRequest {
        op: "fetch",
        args,
        dest_dir: dest.to_string_lossy(),
        locked,
    };
    fetch_exchange(
        &mut Command::new(exe),
        name,
        &request,
        dest,
        PLUGIN_TIMEOUT,
        limits,
    )
}

fn failure(name: &str, reason: String) -> FetchError {
    FetchError::Http {
        url: name.to_owned(),
        reason,
    }
}

fn fetch_exchange(
    command: &mut Command,
    name: &str,
    request: &impl serde::Serialize,
    dest: &Path,
    timeout: Duration,
    limits: FetchLimits,
) -> Result<PluginFetch, FetchError> {
    let mut responded = false;
    let mut result = None;
    let mut diagnostics = Vec::new();
    let mut diagnostic_count = 0usize;
    let mut too_many_diagnostics = false;
    let mut input = gripsack_process::InputBuffer::new(Limits::default().input_bytes);
    serde_json::to_writer(&mut input, request).map_err(|error| failure(name, error.to_string()))?;
    std::io::Write::write_all(&mut input, b"\n")?;
    let outcome = gripsack_process::run(
        command,
        input.as_bytes(),
        Limits {
            timeout,
            ..Limits::default()
        },
        |line| {
            let line = String::from_utf8_lossy(line);
            let Ok(message) = serde_json::from_str::<PluginMessage>(&line) else {
                return Control::Continue;
            };
            match message.kind.as_str() {
                "diagnostic" => {
                    diagnostic_count += 1;
                    if diagnostic_count > 1024 {
                        too_many_diagnostics = true;
                        return Control::Response;
                    }
                    if let Some(d) = message.diagnostic {
                        if d.severity == gripsack_ir::Severity::Error {
                            diagnostics.push(d);
                        } else {
                            tracing::warn!(plugin = name, code = d.code.as_ref(), "{}", d.message);
                        }
                    }
                }
                "progress" => tracing::info!(plugin = name, "{line}"),
                "response" => {
                    responded = true;
                    result = message.result;
                    if let Some(provenance) = result.as_ref().and_then(|r| r.provenance.as_ref()) {
                        tracing::info!(plugin = name, provenance = %provenance, "provenance");
                    }
                    return Control::Response;
                }
                _ => {}
            }
            Control::Continue
        },
    )
    .map_err(FetchError::Io)?;

    // A response is not permission to suppress input, output, deadline or
    // cleanup failures. In particular Cleanup(Exited) is not Exited.
    if !matches!(outcome.reason, StopReason::Exited) {
        let reason = match &outcome.reason {
            StopReason::Deadline => format!(
                "gripfetch-{name} exceeded the {}s exchange deadline",
                timeout.as_secs()
            ),
            StopReason::LineLimit => {
                format!("gripfetch-{name} wrote a single line over the 1 MiB cap")
            }
            other => format!("gripfetch-{name} exchange failed: {other:?}"),
        };
        return Err(failure(name, reason));
    }
    if too_many_diagnostics {
        return Err(failure(
            name,
            "plugin exceeded the 1024 diagnostic cap".into(),
        ));
    }
    if !diagnostics.is_empty() {
        return Err(FetchError::Diagnostics(diagnostics));
    }
    if !responded || !outcome.status.as_ref().is_some_and(|s| s.success()) {
        let stderr = String::from_utf8_lossy(&outcome.stderr);
        let tail: String = stderr
            .lines()
            .last()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        let status = outcome
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "?".into());
        return Err(failure(
            name,
            format!(
                "gripfetch-{name} exited {status} without a response{}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(" — stderr tail: {tail}")
                }
            ),
        ));
    }
    crate::fetch::archive::validate_tree(dest, limits)?;
    let tree = gripsack_store::canonical_tree_hash(dest)?;
    let result = result.unwrap_or_default();
    if let Some(reported) = result.sha256
        && reported != tree.as_str()
    {
        return Err(failure(
            name,
            format!(
                "plugin-reported tree hash {reported} disagrees with the core's {tree} — \
             the plugin's canonical-tree mirror is wrong (see the pinned reference \
             vector in the conformance suite)"
            ),
        ));
    }
    Ok(PluginFetch {
        tree,
        url: result.url,
        version: result.version,
    })
}
