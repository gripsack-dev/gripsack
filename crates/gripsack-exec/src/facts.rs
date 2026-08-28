//! Host facts (plan/0013 D4): detected once, in the core, injected
//! into the frontend via the inputs envelope. The frontend never
//! self-detects — one detector feeding one frontend is what deleted
//! the dual-detection bug class by construction.

use std::sync::LazyLock;

/// What the core can observe about the host. `libc` is `None` when
/// undetectable — never guessed.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HostFacts {
    /// `std::env::consts::OS` (linux, macos, …).
    pub os: &'static str,
    /// `std::env::consts::ARCH` (x86_64, aarch64, …).
    pub arch: &'static str,
    /// `"glibc-<ver>"` (ldd --version), `"musl"` (loader path tell),
    /// `"darwin"` on macOS, `None` if undetectable.
    pub libc: Option<String>,
    /// gethostname — informational for frontend code; host *selection*
    /// (which hosts/<name>.ts loads) stays the CLI's job.
    pub hostname: String,
}

/// Facts are constant for the process lifetime — detect once.
pub fn detect() -> &'static HostFacts {
    static FACTS: LazyLock<HostFacts> = LazyLock::new(|| HostFacts {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        libc: detect_libc(),
        hostname: hostname(),
    });
    &FACTS
}

fn detect_libc() -> Option<String> {
    match std::env::consts::OS {
        "macos" => Some("darwin".into()),
        "linux" => {
            // the musl loader's presence is the musl tell (alpine,
            // void-musl, …); checked first because musl's own ldd
            // exists and would otherwise parse as a version
            let loader = format!("/lib/ld-musl-{}.so.1", std::env::consts::ARCH);
            if std::path::Path::new(&loader).exists() {
                return Some("musl".into());
            }
            glibc_version().map(|v| format!("glibc-{v}"))
        }
        _ => None,
    }
}

/// `ldd --version`'s first line carries the version.
fn glibc_version() -> Option<String> {
    let out = std::process::Command::new("ldd")
        .arg("--version")
        .output()
        .ok()?;
    parse_glibc_line(String::from_utf8_lossy(&out.stdout).lines().next()?)
}

/// "ldd (Ubuntu GLIBC 2.39-0ubuntu8) 2.39" → "2.39": the trailing
/// token, only when it looks numeric and the line names glibc.
fn parse_glibc_line(line: &str) -> Option<String> {
    if !line.contains("GLIBC") && !line.contains("GNU libc") {
        return None;
    }
    line.split_whitespace()
        .next_back()
        .filter(|tok| tok.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: buf is a valid, exclusively-borrowed byte buffer of
        // the length passed alongside it
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            if end > 0
                && let Ok(name) = std::str::from_utf8(&buf[..end])
            {
                return name.to_string();
            }
        }
        "default".into()
    }
    #[cfg(not(unix))]
    {
        "default".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_match_the_running_host() {
        let facts = detect();
        assert_eq!(facts.os, std::env::consts::OS);
        assert_eq!(facts.arch, std::env::consts::ARCH);
        assert!(!facts.hostname.is_empty());
        // this test runs on linux or macOS hosts; both must detect
        if facts.os == "linux" || facts.os == "macos" {
            let libc = facts.libc.as_deref().expect("libc detected");
            assert!(
                libc.starts_with("glibc-") || libc == "musl" || libc == "darwin",
                "libc {libc:?} has a known shape"
            );
        }
    }

    #[test]
    fn glibc_version_parses_both_ldd_wordings() {
        for (line, want) in [
            ("ldd (GNU libc) 2.36", Some("2.36")),
            ("ldd (Ubuntu GLIBC 2.39-0ubuntu8.4) 2.39", Some("2.39")),
            ("ldd (musl compat) nonsense", None),
        ] {
            assert_eq!(parse_glibc_line(line).as_deref(), want, "line: {line}");
        }
    }
}
