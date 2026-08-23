//! `plugin:` — the `gripfetch-*` protocol host (0002 §4, 0009 §2).
//!
//! The core spawns the fetcher, sends one JSON request on stdin, and
//! reads NDJSON messages back: diagnostics are logged (coded, so the
//! run log keeps them), `response` ends the exchange. Exit status is
//! not the contract — but a nonzero exit without a response synthesizes
//! an error with the stderr tail (0009 §2 rule 5). The payload identity
//! is computed by the core (canonical tree hash) — never the plugin's
//! word.

use super::FetchError;
use serde::Deserialize;
use std::io::{BufRead, Write};
use std::path::Path;

#[derive(Deserialize)]
struct PluginMessage {
    #[serde(rename = "type")]
    kind: String,
    diagnostic: Option<serde_json::Value>,
}

pub(crate) fn fetch(
    name: &str,
    args: &serde_json::Value,
    dest: &Path,
) -> Result<String, FetchError> {
    let exe = crate::find_fetcher(name).ok_or_else(|| FetchError::Http {
        url: name.to_string(),
        reason: format!("gripfetch-{name} not found on PATH (or declare it in env.toml)"),
    })?;
    let mut child = std::process::Command::new(&exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(FetchError::Io)?;

    let request = serde_json::json!({
        "op": "fetch",
        "args": args,
        "dest_dir": dest.to_string_lossy(),
    });
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let write_result = writeln!(stdin, "{request}").and_then(|_| stdin.flush());
    drop(stdin);
    write_result?;

    let mut responded = false;
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
                    tracing::warn!(plugin = name, "{d}");
                }
            }
            "progress" => tracing::info!(plugin = name, "{line}"),
            "response" => responded = true,
            _ => {}
        }
        if responded {
            break;
        }
    }

    let output = child.wait_with_output()?;
    if !output.status.success() || !responded {
        let tail = String::from_utf8_lossy(&output.stderr);
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
                "gripfetch-{name} exited {} without a response{}",
                output.status,
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                }
            ),
        });
    }
    // identity is computed by the core — never the plugin's word
    Ok(gripsack_store::canonical_tree_hash(dest)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn fake_plugin(dir: &Path, body: &str) -> PathBuf {
        let exe = dir.join("gripfetch-fake");
        std::fs::write(&exe, body).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }

    static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_path<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        let _guard = PATH_LOCK.lock().unwrap();
        let original = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", format!("{}:/usr/bin:/bin", dir.display())) };
        let result = f();
        match original {
            Some(v) => unsafe { std::env::set_var("PATH", v) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        result
    }

    const GOOD: &str = r#"#!/bin/sh
read line
dest=$(echo "$line" | sed -n 's/.*"dest_dir": *"\([^"]*\)".*/\1/p')
mkdir -p "$dest"
echo "plugin payload" > "$dest/payload.txt"
echo '{"type": "response"}'
"#;

    #[test]
    fn plugin_fetch_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        fake_plugin(dir.path(), GOOD);
        let dest = dir.path().join("out");
        let hash = with_path(dir.path(), || fetch("fake", &serde_json::json!({}), &dest)).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("payload.txt")).unwrap(),
            "plugin payload\n"
        );
        assert_eq!(hash, gripsack_store::canonical_tree_hash(&dest).unwrap());
    }

    #[test]
    fn dying_plugin_reports_stderr_tail() {
        let dir = tempfile::tempdir().unwrap();
        fake_plugin(
            dir.path(),
            "#!/bin/sh\necho 'registry exploded' >&2\nexit 3\n",
        );
        let dest = dir.path().join("out");
        let err =
            with_path(dir.path(), || fetch("fake", &serde_json::json!({}), &dest)).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("registry exploded"), "{text}");
    }
}
