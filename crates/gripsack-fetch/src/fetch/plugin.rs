//! `plugin:` — the `gripfetch-*` protocol host (0002 §4, 0009 §2).
//!
//! The core spawns the fetcher, sends one JSON request on stdin, and
//! reads NDJSON messages back. The request carries `locked` when the
//! lockfile has a pin for this module — a plugin must be able to tell
//! first-fetch (resolve, TOFU) from pinned re-fetch (reproduce
//! exactly); for internal registries those are different code paths.
//!
//! Diagnostics are deserialized, not logged raw: warnings trace into
//! the run log coded; any error-severity diagnostic fails the fetch
//! and the CLI renders it through the same renderer as its own (0009
//! §2 rule 1). The payload identity is computed by the core
//! (canonical tree hash) — never the plugin's word.
//!
//! Robustness: stderr is drained on its own thread (a chatty plugin
//! fills the ~64KB pipe buffer and deadlocks otherwise), and the
//! whole exchange has a deadline — no unbounded hangs.

use super::FetchError;
use serde::Deserialize;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// Whole-exchange deadline for one plugin fetch (0007 §retries —
/// a stuck plugin is a failure, never a silent wait).
const PLUGIN_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Deserialize)]
struct PluginMessage {
    #[serde(rename = "type")]
    kind: String,
    diagnostic: Option<gripsack_ir::Diagnostic>,
    result: Option<PluginResult>,
}

#[derive(Deserialize)]
struct PluginResult {
    /// Informational only — the core recomputes identity from the
    /// staged bytes. Provenance is the valuable half: which registry,
    /// which mirror, which credential identity — it lands in the run
    /// log (0009 §2 rule 7).
    provenance: Option<serde_json::Value>,
    /// The `capabilities` op payload — declared rate budgets (0002
    /// §throttle), parsed separately from the fetch response.
    capabilities: Option<serde_json::Value>,
}

/// What a fetcher tells us about itself (the `capabilities` op —
/// 0002 §throttle). Unknown/unsupported op responses are tolerated:
/// an older plugin simply has no declared budgets.
#[derive(Deserialize, Default, Clone)]
pub(crate) struct Capabilities {
    #[serde(default)]
    throttle: std::collections::BTreeMap<String, String>,
}

/// Ask a fetcher for its capabilities once per process; failures
/// (old plugin, unknown op) mean "no declared budgets", never an
/// error. Rate budgets live in fetchers — this is how they arrive.
fn capabilities(name: &str, exe: &Path) -> Option<Capabilities> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, Option<Capabilities>>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));
    if let Some(entry) = cache.lock().expect("caps cache").get(name) {
        return entry.clone();
    }
    let caps = capabilities_exchange(exe);
    cache
        .lock()
        .expect("caps cache")
        .insert(name.to_string(), caps.clone());
    caps
}

/// A short one-shot exchange: `{"op":"capabilities"}` in, one
/// response out, 30s deadline. Same rules as the fetch exchange —
/// stderr drained, non-protocol lines ignored.
fn capabilities_exchange(exe: &Path) -> Option<Capabilities> {
    let mut child = std::process::Command::new(exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    use std::io::Write as _;
    writeln!(stdin, r#"{{"op":"capabilities"}}"#).ok()?;
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(30);
    let stdout = child.stdout.take()?;
    let mut result = None;
    for line in std::io::BufReader::new(stdout).lines() {
        if Instant::now() > deadline {
            break;
        }
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<PluginMessage>(&line) else {
            continue;
        };
        if message.kind == "response" {
            result = message
                .result
                .and_then(|r| r.capabilities)
                .and_then(|c| serde_json::from_value::<Capabilities>(c).ok());
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

pub(crate) fn fetch(
    name: &str,
    args: &serde_json::Value,
    dest: &Path,
    locked: Option<&serde_json::Value>,
) -> Result<String, FetchError> {
    let exe = crate::find_fetcher(name).ok_or_else(|| FetchError::Http {
        url: name.to_string(),
        reason: format!("gripfetch-{name} not found on PATH (or declare it in env.toml)"),
    })?;
    // rate budgets live in fetchers: register the plugin's declared
    // domains (capabilities op) and take a token per invocation
    if let Some(caps) = capabilities(name, &exe) {
        for (domain, budget) in &caps.throttle {
            crate::throttle::acquire_declared(domain, budget);
        }
    }
    let mut child = std::process::Command::new(&exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(FetchError::Io)?;

    let mut request = serde_json::json!({
        "op": "fetch",
        "args": args,
        "dest_dir": dest.to_string_lossy(),
    });
    if let Some(locked) = locked {
        request["locked"] = locked.clone();
    }
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let write_result = writeln!(stdin, "{request}").and_then(|_| stdin.flush());
    drop(stdin);
    write_result?;

    // stderr must drain concurrently — a plugin that chatters fills
    // the pipe buffer and both sides block (review finding F1).
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut reader = std::io::BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if buf.len() < 64 * 1024 {
                        buf.extend_from_slice(line.as_bytes());
                    }
                }
            }
        }
        buf
    });

    let deadline = Instant::now() + PLUGIN_TIMEOUT;
    let mut responded = false;
    let mut error_diagnostics = Vec::new();
    for line in std::io::BufReader::new(stdout).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<PluginMessage>(&line) else {
            continue; // tolerance: non-protocol lines are ignored (0009)
        };
        match message.kind.as_str() {
            "diagnostic" => {
                if let Some(d) = message.diagnostic {
                    if d.severity == gripsack_ir::Severity::Error {
                        error_diagnostics.push(d);
                    } else {
                        tracing::warn!(plugin = name, code = d.code.as_ref(), "{}", d.message);
                    }
                }
            }
            "progress" => tracing::info!(plugin = name, "{line}"),
            "response" => {
                responded = true;
                if let Some(provenance) = message.result.and_then(|r| r.provenance) {
                    tracing::info!(plugin = name, provenance = %provenance, "provenance");
                }
            }
            _ => {}
        }
        if responded {
            break;
        }
    }

    // Reap the child within the deadline; a plugin ignoring a closed
    // stdout gets killed, not waited on forever.
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_thread.join();
                return Err(FetchError::Http {
                    url: name.to_string(),
                    reason: format!(
                        "gripfetch-{name} exceeded the {}s exchange deadline",
                        PLUGIN_TIMEOUT.as_secs()
                    ),
                });
            }
        }
    };
    let stderr_buf = stderr_thread.join().unwrap_or_default();

    if !error_diagnostics.is_empty() {
        return Err(FetchError::Diagnostics(error_diagnostics));
    }
    if !status.success() || !responded {
        let tail = String::from_utf8_lossy(&stderr_buf);
        let tail = tail
            .lines()
            .last()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        return Err(FetchError::Http {
            url: name.to_string(),
            reason: format!(
                "gripfetch-{name} exited {status} without a response{}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(" — stderr tail: {tail}")
                }
            ),
        });
    }
    // identity is computed by the core — never the plugin's word
    Ok(gripsack_store::canonical_tree_hash(dest)?)
}
