//! Homebrew bottle-tag selection (handover A0-01, plan/0049).
//!
//! Pure policy: tag names plus injected host facts in, one tag or a
//! precise refusal out. The previous lexical guess iterated the tag map
//! in reverse order, which selected Linux bottles on Intel Macs
//! (`x86_64_linux` sorts after `sonoma`) and inferred macOS chronology
//! from tag spelling (`ventura` beats `sequoia` lexically but not
//! chronologically). Selection here recognizes tags structurally and
//! validates them against the injected OS/architecture/macOS-version
//! facts; macOS ordering comes only from the codename table, never
//! from tag-name ordering.
//!
//! Tag recognition follows the Homebrew bottle-tag grammar
//! (`arm64_linux`, `x86_64_linux`, `arm64_<codename>`, `<codename>`,
//! `all`). Unrecognized tags never match; a map with no recognizable
//! compatible tag is an error, not a guess.

use std::collections::BTreeMap;

/// macOS codename → minimum host version that may run its bottles.
/// Chronology lives here and nowhere else.
const MACOS_CODENAMES: &[(&str, &[u32])] = &[
    ("high_sierra", &[10, 13]),
    ("mojave", &[10, 14]),
    ("catalina", &[10, 15]),
    ("big_sur", &[11]),
    ("monterey", &[12]),
    ("ventura", &[13]),
    ("sonoma", &[14]),
    ("sequoia", &[15]),
    ("tahoe", &[26]),
];

/// A dotted macOS version (`15`, `15.1`, `26.0.1`), compared
/// componentwise. Not a semver: no pre-release grammar.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MacOsVersion(Vec<u32>);

impl MacOsVersion {
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() {
            return None;
        }
        let parts = raw
            .split('.')
            .map(|part| part.parse::<u32>().ok())
            .collect::<Option<Vec<_>>>()?;
        Some(Self(parts))
    }
}

impl std::fmt::Display for MacOsVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, part) in self.0.iter().enumerate() {
            if index != 0 {
                f.write_str(".")?;
            }
            write!(f, "{part}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Os {
    Linux,
    Macos,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
    Other(String),
}

/// The injected facts bottle selection runs on. Detected once per
/// acquisition context (see [`HostPlatform::detect`]); tests construct
/// synthetic values — the policy never reads the environment itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPlatform {
    pub os: Os,
    pub arch: Arch,
    /// Required to admit any macOS bottle: without it compatibility
    /// cannot be validated, and guessing is exactly the removed bug.
    pub macos_version: Option<MacOsVersion>,
}

impl HostPlatform {
    /// Bounded local detection. `sw_vers` runs only on macOS; a missing
    /// or malformed answer stays `None` and selection refuses rather
    /// than guessing.
    pub fn detect() -> Self {
        let os = match std::env::consts::OS {
            "linux" => Os::Linux,
            "macos" => Os::Macos,
            other => Os::Other(other.to_string()),
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            other => Arch::Other(other.to_string()),
        };
        let macos_version = if os == Os::Macos {
            Self::detect_macos_version()
        } else {
            None
        };
        Self {
            os,
            arch,
            macos_version,
        }
    }

    /// A native fact probe, not an evaluation effect. Use the same
    /// supervised process boundary as fetchers: a hung or noisy system
    /// tool must not hang resolution or smuggle unbounded output.
    fn detect_macos_version() -> Option<MacOsVersion> {
        use gripsack_process::{Control, Limits, StopReason};
        use std::time::Duration;

        let mut command = std::process::Command::new("/usr/bin/sw_vers");
        command.arg("-productVersion");
        let mut version = None;
        let mut lines = 0;
        let outcome = gripsack_process::run(
            &mut command,
            &[],
            Limits {
                timeout: Duration::from_secs(3),
                input_bytes: 0,
                line_bytes: 64,
                stdout_bytes: 128,
                stderr_bytes: 128,
                retained_stderr_bytes: 0,
            },
            |line| {
                lines += 1;
                if lines == 1 {
                    version = std::str::from_utf8(line)
                        .ok()
                        .and_then(|text| MacOsVersion::parse(text.trim_end_matches('\r')));
                }
                Control::Continue
            },
        )
        .ok()?;
        (lines == 1
            && matches!(outcome.reason, StopReason::Exited)
            && outcome.status.is_some_and(|status| status.success()))
        .then_some(version)
        .flatten()
    }

    fn describe(&self) -> String {
        let os = match &self.os {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Other(other) => other,
        };
        let arch = match &self.arch {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
            Arch::Other(other) => other,
        };
        match &self.macos_version {
            Some(version) => format!("{os}/{arch} (macOS {version})"),
            None => format!("{os}/{arch}"),
        }
    }
}

/// One recognized bottle tag.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BottleTag {
    /// Architecture-neutral; Homebrew ships these for
    /// platform-independent payloads. Admitted only when no
    /// platform-specific tag matches.
    All,
    Linux {
        arch: Arch,
    },
    Macos {
        arch: Arch,
        min_version: &'static [u32],
    },
}

fn parse_tag(tag: &str) -> Option<BottleTag> {
    if tag == "all" {
        return Some(BottleTag::All);
    }
    if let Some(codename) = tag.strip_prefix("arm64_") {
        if codename != "linux" {
            let (_, min_version) = MACOS_CODENAMES.iter().find(|(name, _)| *name == codename)?;
            return Some(BottleTag::Macos {
                arch: Arch::Aarch64,
                min_version,
            });
        }
        return Some(BottleTag::Linux {
            arch: Arch::Aarch64,
        });
    }
    if tag == "x86_64_linux" {
        return Some(BottleTag::Linux { arch: Arch::X86_64 });
    }
    let (_, min_version) = MACOS_CODENAMES.iter().find(|(name, _)| *name == tag)?;
    Some(BottleTag::Macos {
        arch: Arch::X86_64,
        min_version,
    })
}

/// Why a tag did not match, for the refusal message.
enum Rejection {
    Unrecognized,
    WrongPlatform(String),
    TooNew(MacOsVersion),
}

impl Rejection {
    fn describe(&self, tag: &str) -> String {
        match self {
            Rejection::Unrecognized => format!("{tag} (unrecognized tag)"),
            Rejection::WrongPlatform(target) => format!("{tag} ({target})"),
            Rejection::TooNew(min) => format!("{tag} (requires macOS ≥ {min})"),
        }
    }
}

/// The refusal returned when no bottle can be admitted: enough context
/// to act on (what the host is, why each available tag was rejected).
#[derive(Debug, thiserror::Error)]
#[error("no bottle compatible with {host}: {reason}")]
pub struct SelectionFailure {
    pub host: String,
    pub reason: String,
}

fn arch_name(arch: &Arch) -> &str {
    match arch {
        Arch::X86_64 => "x86_64",
        Arch::Aarch64 => "aarch64",
        Arch::Other(other) => other,
    }
}

/// Select the bottle tag for `host` from `files`' keys.
///
/// Linux wants the exact architecture tag. macOS wants the greatest
/// codename tag whose minimum version the host satisfies — older
/// bottles run on newer macOS, never the reverse. `all` is a last
/// resort under that validated policy, not a peer candidate.
pub fn select<'a>(
    files: &'a BTreeMap<String, impl Sized>,
    host: &HostPlatform,
) -> Result<&'a str, SelectionFailure> {
    let refuse = |reason: String| {
        Err(SelectionFailure {
            host: host.describe(),
            reason,
        })
    };
    match (&host.os, &host.arch) {
        (Os::Other(other), _) | (_, Arch::Other(other)) => {
            return refuse(format!("unsupported platform {other} for bottle selection"));
        }
        (Os::Linux, arch) => {
            let wanted = match arch {
                Arch::X86_64 => "x86_64_linux",
                Arch::Aarch64 => "arm64_linux",
                Arch::Other(_) => unreachable!(),
            };
            if files.contains_key(wanted) {
                return Ok(wanted);
            }
        }
        (Os::Macos, _) => {
            let host_version = host
                .macos_version
                .as_ref()
                .ok_or_else(|| SelectionFailure {
                    host: host.describe(),
                    reason: "host macOS version unavailable; bottle compatibility cannot be \
                             validated"
                        .into(),
                })?;
            let mut best: Option<(&str, &[u32])> = None;
            for tag in files.keys() {
                if let Some(BottleTag::Macos { arch, min_version }) = parse_tag(tag) {
                    if arch != host.arch {
                        continue;
                    }
                    if min_version <= host_version.0.as_slice()
                        && best
                            .as_ref()
                            .is_none_or(|(_, best_min)| *best_min < min_version)
                    {
                        best = Some((tag.as_str(), min_version));
                    }
                }
            }
            if let Some((tag, _)) = best {
                return Ok(tag);
            }
        }
    }
    // No platform-specific match. `all` is admissible only now.
    if files.contains_key("all") {
        return Ok("all");
    }
    let reason = if files.is_empty() {
        "formula ships no bottles".to_string()
    } else {
        let rejections = files
            .keys()
            .map(|tag| {
                let rejection = match parse_tag(tag) {
                    None => Rejection::Unrecognized,
                    Some(BottleTag::All) => Rejection::WrongPlatform("arch-neutral".into()),
                    Some(BottleTag::Linux { arch }) => {
                        Rejection::WrongPlatform(format!("linux/{}", arch_name(&arch)))
                    }
                    Some(BottleTag::Macos { arch, min_version }) => {
                        let min = MacOsVersion(min_version.to_vec());
                        if arch != host.arch {
                            Rejection::WrongPlatform(format!("macos/{}", arch_name(&arch)))
                        } else if host
                            .macos_version
                            .as_ref()
                            .is_some_and(|version| &min > version)
                        {
                            Rejection::TooNew(min)
                        } else {
                            Rejection::WrongPlatform("macos".into())
                        }
                    }
                };
                rejection.describe(tag)
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("available bottles do not match: {rejections}")
    };
    refuse(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(keys: &[&str]) -> BTreeMap<String, ()> {
        keys.iter().map(|k| ((*k).to_string(), ())).collect()
    }

    fn linux(arch: Arch) -> HostPlatform {
        HostPlatform {
            os: Os::Linux,
            arch,
            macos_version: None,
        }
    }

    fn macos(arch: Arch, version: &str) -> HostPlatform {
        HostPlatform {
            os: Os::Macos,
            arch,
            macos_version: Some(MacOsVersion::parse(version).unwrap()),
        }
    }

    fn select_str(keys: &[&str], host: &HostPlatform) -> Result<String, String> {
        select(&files(keys), host)
            .map(str::to_string)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn linux_matches_exact_architecture() {
        assert_eq!(
            select_str(&["x86_64_linux", "sonoma"], &linux(Arch::X86_64)).unwrap(),
            "x86_64_linux"
        );
        assert_eq!(
            select_str(&["arm64_linux", "sonoma"], &linux(Arch::Aarch64)).unwrap(),
            "arm64_linux"
        );
    }

    #[test]
    fn intel_mac_never_takes_the_linux_bottle() {
        // The removed bug: `x86_64_linux` passed the old "not arm64,
        // not all" filter and sorted after every codename.
        let host = macos(Arch::X86_64, "14.1");
        assert_eq!(
            select_str(&["sonoma", "ventura", "x86_64_linux"], &host).unwrap(),
            "sonoma"
        );
    }

    #[test]
    fn intel_mac_takes_the_newest_bottle_the_host_can_run() {
        let host = macos(Arch::X86_64, "13.5");
        assert_eq!(
            select_str(&["sonoma", "ventura", "x86_64_linux"], &host).unwrap(),
            "ventura"
        );
    }

    #[test]
    fn apple_silicon_orders_by_chronology_not_spelling() {
        // Lexical reverse order would pick `ventura` over `sonoma`.
        let host = macos(Arch::Aarch64, "15.0");
        assert_eq!(
            select_str(&["arm64_ventura", "arm64_sonoma"], &host).unwrap(),
            "arm64_sonoma"
        );
    }

    #[test]
    fn arm64_linux_is_not_a_mac_bottle() {
        let host = macos(Arch::Aarch64, "15.0");
        let err = select_str(&["arm64_linux", "x86_64_linux"], &host).unwrap_err();
        assert!(err.contains("macos/aarch64"), "{err}");
        assert!(err.contains("linux"), "{err}");
    }

    #[test]
    fn older_macos_refuses_newer_bottle_with_the_requirement() {
        let host = macos(Arch::Aarch64, "14.0");
        let err = select_str(&["arm64_sequoia"], &host).unwrap_err();
        assert!(err.contains("requires macOS ≥ 15"), "{err}");
    }

    #[test]
    fn missing_macos_version_fact_refuses_rather_than_guessing() {
        let host = HostPlatform {
            os: Os::Macos,
            arch: Arch::Aarch64,
            macos_version: None,
        };
        let err = select_str(&["arm64_sonoma"], &host).unwrap_err();
        assert!(err.contains("macOS version unavailable"), "{err}");
    }

    #[test]
    fn unknown_tags_are_reported_not_matched() {
        let host = linux(Arch::X86_64);
        let err = select_str(&["arm64_mavericks", "weird-tag"], &host).unwrap_err();
        assert!(err.contains("unrecognized"), "{err}");
    }

    #[test]
    fn all_is_a_last_resort_under_a_validated_policy() {
        assert_eq!(select_str(&["all"], &linux(Arch::Aarch64)).unwrap(), "all");
        // A specific match always wins over `all`.
        let host = macos(Arch::Aarch64, "15.0");
        assert_eq!(
            select_str(&["all", "arm64_sonoma"], &host).unwrap(),
            "arm64_sonoma"
        );
    }

    #[test]
    fn unsupported_architecture_fails_clearly() {
        let host = linux(Arch::Other("riscv64".into()));
        let err = select_str(&["x86_64_linux"], &host).unwrap_err();
        assert!(err.contains("riscv64"), "{err}");
    }

    #[test]
    fn future_macos_major_admits_its_own_codename() {
        let host = macos(Arch::Aarch64, "26.1");
        assert_eq!(
            select_str(&["arm64_sonoma", "arm64_tahoe"], &host).unwrap(),
            "arm64_tahoe"
        );
    }

    #[test]
    fn version_parsing_is_componentwise() {
        assert!(MacOsVersion::parse("15.1.2") > MacOsVersion::parse("15.1"));
        assert!(MacOsVersion::parse("10.15") > MacOsVersion::parse("10.9"));
        assert!(MacOsVersion::parse("").is_none());
        assert!(MacOsVersion::parse("15.x").is_none());
    }
}
