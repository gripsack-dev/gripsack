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
//! fills the ~64KB pipe buffer and deadlocks otherwise); the request
//! is written from its own thread (the same buffer, the other
//! direction — a big `args` payload must not block before stdout is
//! read); stdout is read on a dedicated thread, one capped line at a
//! time, so the deadline can fire during a blocking read; a wedged
//! plugin is killed and reaped, never waited on forever.

use super::FetchError;
use serde::Deserialize;
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Whole-exchange deadline for one plugin fetch (0007 §retries —
/// a stuck plugin is a failure, never a silent wait).
const PLUGIN_TIMEOUT: Duration = Duration::from_secs(600);

const MIB: usize = 1024 * 1024;

/// Cap on one stdout line — a fetcher that never newlines would grow
/// the read buffer without bound; the deadline bounds time, not
/// memory.
const MAX_LINE: usize = MIB;

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
    /// The plugin's pin for THIS fetch: the upstream artifact's URL and
    /// version. Recorded in the lockfile so the next apply's `locked`
    /// tells the plugin exactly what to reproduce (0002 §4) — without
    /// this, locked could never arrive (gripfetch-apt review).
    url: Option<String>,
    version: Option<String>,
    /// The plugin's own tree-hash mirror — verified against the core's
    /// at write time (never trusted, but disagreement is a loud bug).
    sha256: Option<String>,
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

/// One stdout event off the reader thread: a line (newline stripped),
/// the end of the stream, or a line that blew the cap.
enum Read {
    Line(String),
    Eof,
    TooLong,
}

/// Read one line of at most MAX_LINE content bytes into `buf`.
/// `read_line`/`read_until` grow the buffer without bound — the cap
/// must be checked as bytes arrive, not after the line lands.
fn read_capped_line<R: BufRead>(reader: &mut R, buf: &mut Vec<u8>) -> std::io::Result<Read> {
    buf.clear();
    loop {
        let available = match reader.fill_buf() {
            Ok(a) => a,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                Read::Eof
            } else {
                Read::Line(stripped(buf))
            });
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(nl) => {
                if buf.len() + nl > MAX_LINE {
                    return Ok(Read::TooLong);
                }
                buf.extend_from_slice(&available[..=nl]);
                reader.consume(nl + 1);
                return Ok(Read::Line(stripped(buf)));
            }
            None => {
                if buf.len() + available.len() > MAX_LINE {
                    return Ok(Read::TooLong);
                }
                buf.extend_from_slice(available);
                let n = available.len();
                reader.consume(n);
            }
        }
    }
}

/// Lossy-utf8 (a broken fetcher's byte soup then fails the
/// deserialize downstream like any non-protocol line) with the
/// trailing newline and a CRLF carriage return stripped.
fn stripped(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf)
        .trim_end_matches(['\n', '\r'])
        .to_owned()
}

/// Time left before `deadline`, ZERO once it has passed —
/// `recv_timeout(ZERO)` reports Timeout immediately, which is
/// exactly the deadline semantics the exchanges want.
fn remaining_until(deadline: Instant) -> Duration {
    let now = Instant::now();
    if deadline > now {
        deadline - now
    } else {
        Duration::ZERO
    }
}

/// Read child stdout on a dedicated thread: a blocking `read_line`
/// can't observe a deadline, the consumer's `recv_timeout` can. io
/// errors end the stream like EOF — the consumer learns why from the
/// child's exit status.
fn spawn_line_reader(stdout: std::process::ChildStdout) -> Receiver<Read> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            match read_capped_line(&mut reader, &mut buf) {
                Ok(Read::Eof) | Err(_) => {
                    let _ = tx.send(Read::Eof);
                    break;
                }
                Ok(event) => {
                    if tx.send(event).is_err() {
                        break; // consumer gone; stop draining
                    }
                }
            }
        }
    });
    rx
}

/// Ask a fetcher for its capabilities once per process; failures
/// (old plugin, unknown op) mean "no declared budgets", never an
/// error. Rate budgets live in fetchers — this is how they arrive.
fn capabilities(name: &str, exe: &Path) -> Option<Capabilities> {
    static CACHE: std::sync::LazyLock<
        std::sync::Mutex<std::collections::BTreeMap<String, Option<Capabilities>>>,
    > = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));
    let cache = &*CACHE;
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
/// stderr nulled, non-protocol lines ignored.
fn capabilities_exchange(exe: &Path) -> Option<Capabilities> {
    capabilities_exchange_with(exe, Duration::from_secs(30))
}

/// The capabilities conversation, split out (explicit timeout) so
/// tests can drive a wedged fetcher quickly.
fn capabilities_exchange_with(exe: &Path, timeout: Duration) -> Option<Capabilities> {
    let mut child = match std::process::Command::new(exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return None,
    };
    let result = capabilities_body(&mut child, timeout);
    // every path reaps: an abandoned probe (early return, deadline,
    // response-then-linger) is a zombie otherwise — and the deadline
    // can't fire during a blocking read, the reader thread is what
    // makes it real
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn capabilities_body(child: &mut std::process::Child, timeout: Duration) -> Option<Capabilities> {
    // the probe is tiny, but write from a thread anyway: one rule for
    // both exchanges (a peer that never drains stdin can't wedge us
    // before it answers)
    let mut stdin = child.stdin.take()?;
    let req = r#"{"op":"capabilities"}"#.to_string();
    std::thread::spawn(move || {
        let _ = writeln!(stdin, "{req}");
    });
    let stdout = child.stdout.take()?;
    let rx = spawn_line_reader(stdout);
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = remaining_until(deadline);
        match rx.recv_timeout(remaining) {
            Ok(Read::Line(line)) => {
                let Ok(message) = serde_json::from_str::<PluginMessage>(&line) else {
                    continue;
                };
                if message.kind == "response" {
                    return message
                        .result
                        .and_then(|r| r.capabilities)
                        .and_then(|c| serde_json::from_value::<Capabilities>(c).ok());
                }
            }
            // eof, oversized line, or deadline — no declared budgets
            _ => return None,
        }
    }
}

/// What a plugin fetch yields: the core-computed payload identity plus
/// the plugin's reported pin (recorded, never trusted — enforcement
/// stays the core's tree hash).
#[derive(Debug)]
pub(crate) struct PluginFetch {
    pub hash: String,
    pub url: Option<String>,
    pub version: Option<String>,
}

pub(crate) fn fetch(
    name: &str,
    args: &serde_json::Value,
    dest: &Path,
    locked: Option<&serde_json::Value>,
) -> Result<PluginFetch, FetchError> {
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
    fetch_exchange(&mut child, name, &request, dest, PLUGIN_TIMEOUT)
}

/// The fetch conversation proper, split from `fetch` so tests can
/// drive an already-spawned child with a short deadline instead of
/// PLUGIN_TIMEOUT.
fn fetch_exchange(
    child: &mut std::process::Child,
    name: &str,
    request: &serde_json::Value,
    dest: &Path,
    timeout: Duration,
) -> Result<PluginFetch, FetchError> {
    // the request (args plus a locked pin) can outrun the ~64KB pipe
    // buffer — writing it inline blocks until the peer drains, and a
    // peer that answers before reading wedges us. write from a
    // thread; the pipe closes on drop, handing the peer EOF. never
    // joined: a grandchild holding the pipe open would outlive the
    // kill.
    if let Some(mut stdin) = child.stdin.take() {
        let req = request.to_string();
        std::thread::spawn(move || {
            let _ = writeln!(stdin, "{req}");
        });
    }
    // stderr must drain concurrently — a plugin that chatters fills
    // the pipe buffer and both sides block (review finding F1).
    let stderr = child.stderr.take().expect("piped stderr");
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
    let deadline_error = || FetchError::Http {
        url: name.to_string(),
        reason: format!(
            "gripfetch-{name} exceeded the {}s exchange deadline",
            timeout.as_secs()
        ),
    };
    let deadline = Instant::now() + timeout;
    let rx = spawn_line_reader(child.stdout.take().expect("piped stdout"));
    let mut responded = false;
    let mut pin: (Option<String>, Option<String>) = (None, None);
    let mut pin_sha: Option<String> = None;
    let mut error_diagnostics = Vec::new();
    loop {
        // the deadline lives HERE, not in the reader: recv_timeout
        // wakes while a blocking read never would
        let remaining = remaining_until(deadline);
        match rx.recv_timeout(remaining) {
            Ok(Read::Line(line)) => {
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
                                tracing::warn!(
                                    plugin = name,
                                    code = d.code.as_ref(),
                                    "{}",
                                    d.message
                                );
                            }
                        }
                    }
                    "progress" => tracing::info!(plugin = name, "{line}"),
                    "response" => {
                        responded = true;
                        if let Some(result) = message.result {
                            pin = (result.url, result.version);
                            pin_sha = result.sha256;
                            if let Some(provenance) = result.provenance {
                                tracing::info!(plugin = name, provenance = %provenance, "provenance");
                            }
                        }
                    }
                    _ => {}
                }
                if responded {
                    break;
                }
            }
            Ok(Read::Eof) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                return Err(reap(child, stderr_thread, deadline_error()));
            }
            Ok(Read::TooLong) => {
                return Err(reap(
                    child,
                    stderr_thread,
                    FetchError::Http {
                        url: name.to_string(),
                        reason: format!(
                            "gripfetch-{name} wrote a single line over the {} MiB cap",
                            MAX_LINE / MIB
                        ),
                    },
                ));
            }
        }
    }

    // Reap the child within the deadline; a plugin ignoring a closed
    // stdout gets killed, not waited on forever.
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            None => {
                return Err(reap(child, stderr_thread, deadline_error()));
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
    let hash = gripsack_store::canonical_tree_hash(dest)?;
    // …and a plugin that REPORTS a tree hash must agree with it (the
    // disagreement class that broke update→apply on cold stores):
    // fail at write time, not at the next cold apply
    if let Some(reported) = pin_sha
        && reported != hash
    {
        return Err(FetchError::Http {
            url: name.to_string(),
            reason: format!(
                "plugin-reported tree hash {reported} disagrees with the core's {hash} — \
                 the plugin's canonical-tree mirror is wrong (see the pinned reference \
                 vector in the conformance suite)"
            ),
        });
    }
    Ok(PluginFetch {
        hash,
        url: pin.0,
        version: pin.1,
    })
}

/// Kill and fully reap a wedged plugin before failing: the error is
/// the point, but an unreaped child is a zombie and its stderr drain
/// thread lives until the pipe closes.
fn reap(
    child: &mut std::process::Child,
    stderr_thread: std::thread::JoinHandle<Vec<u8>>,
    err: FetchError,
) -> FetchError {
    let _ = child.kill();
    let _ = child.wait();
    let _ = stderr_thread.join();
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    /// an executable /bin/sh script in a tempdir that outlives the
    /// child it spawns
    fn script(name: &str, body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{name}.sh"));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path)
    }

    fn spawn(exe: &Path) -> std::process::Child {
        std::process::Command::new(exe)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn silent_plugin_hits_the_exchange_deadline() {
        let (_dir, exe) = script("sleepy", "exec sleep 60");
        let mut child = spawn(&exe);
        let start = Instant::now();
        let err = fetch_exchange(
            &mut child,
            "sleepy",
            &serde_json::json!({"op": "fetch"}),
            Path::new("/nonexistent"),
            Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the deadline must fire long before the 60s child exits"
        );
        assert!(err.to_string().contains("exchange deadline"), "{err}");
    }

    #[test]
    fn oversized_plugin_line_is_refused_at_the_cap() {
        let (_dir, exe) = script(
            "chatty",
            "head -c 2097152 /dev/zero | tr '\\0' a\nexec sleep 60",
        );
        let mut child = spawn(&exe);
        let start = Instant::now();
        let err = fetch_exchange(
            &mut child,
            "chatty",
            &serde_json::json!({"op": "fetch"}),
            Path::new("/nonexistent"),
            Duration::from_secs(60),
        )
        .unwrap_err();
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the cap fires mid-line, not at the deadline"
        );
        assert!(err.to_string().contains("1 MiB cap"), "{err}");
    }

    #[test]
    fn well_behaved_plugin_round_trips() {
        let body = r#"read line
printf '{"type":"diagnostic","diagnostic":{"code":"W1","severity":"warning","message":"note it"}}\n'
printf '{"type":"response","result":{"url":"https://example/tarball","version":"1.2.3"}}\n'"#;
        let (_dir, exe) = script("good", body);
        let mut child = spawn(&exe);
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(dest.path().join("payload"), b"hello").unwrap();
        let got = fetch_exchange(
            &mut child,
            "good",
            &serde_json::json!({"op": "fetch", "args": {}, "dest_dir": dest.path().to_string_lossy()}),
            dest.path(),
            Duration::from_secs(60),
        )
        .unwrap();
        assert_eq!(got.url.as_deref(), Some("https://example/tarball"));
        assert_eq!(got.version.as_deref(), Some("1.2.3"));
        // identity stays the core's word: the reported pin is trusted,
        // the hash is recomputed from the staged tree
        assert_eq!(
            got.hash,
            gripsack_store::canonical_tree_hash(dest.path()).unwrap()
        );
    }

    #[test]
    fn capabilities_probe_times_out_on_a_silent_fetcher() {
        let (_dir, exe) = script("cap-sleepy", "exec sleep 60");
        let start = Instant::now();
        assert!(capabilities_exchange_with(&exe, Duration::from_secs(2)).is_none());
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the probe deadline must fire during the blocking read, not after it"
        );
    }

    #[test]
    fn capabilities_round_trip() {
        let body = r#"read line
printf '{"type":"response","result":{"capabilities":{"throttle":{"example.com":"10/min"}}}}'"#;
        let (_dir, exe) = script("cap-good", body);
        let caps = capabilities_exchange_with(&exe, Duration::from_secs(60)).expect("caps");
        assert_eq!(
            caps.throttle.get("example.com").map(String::as_str),
            Some("10/min")
        );
    }

    #[test]
    fn capped_reader_strips_frames_and_bounds() {
        let mut buf = Vec::new();
        let mut r = std::io::BufReader::new(&b"one\r\ntwo\n"[..]);
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::Line(l) if l == "one"
        ));
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::Line(l) if l == "two"
        ));
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::Eof
        ));
        // past the cap is refused before the newline ever arrives
        let oversized = vec![b'a'; MAX_LINE + 1];
        let mut r = std::io::BufReader::new(&oversized[..]);
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::TooLong
        ));
        // exactly at the cap still reads
        let capped = vec![b'a'; MAX_LINE];
        let mut r = std::io::BufReader::new(&capped[..]);
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::Line(l) if l.len() == MAX_LINE
        ));
    }
}
