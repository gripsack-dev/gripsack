//! Tool configuration as data (plan/0005 §2): parsed before any module
//! code runs — config can never depend on eval results.
//!
//! ```text
//! precedence (later wins):
//!
//!   built-in defaults
//!     < ~/.config/gripsack/config.toml   user layer — machine-local, uncommitted
//!     < env.toml                         repo layer — committed, self-describing
//!     < GRIPSACK_* env vars
//!     < CLI flags
//! ```
//!
//! `env.toml` declares the frontend, eval-time deps (resolvers/fetcher
//! libraries), fetcher plugin wiring, throttle domains, and settings.
//! The user layer only fills gaps — a cloned repo behaves identically
//! everywhere.
use serde::Deserialize;
use std::collections::BTreeMap;

use gripsack_ir::{Diagnostic, Span, codes};

/// Repo-level configuration (`env.toml`), committed — the env is
/// self-describing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnvConfig {
    pub env: EnvSection,
    pub eval: EvalSection,
    pub fetchers: BTreeMap<String, FetcherSection>,
    /// Linter registry (0011 §7): name → pinned package (provisioned
    /// into the frontend venv) or an explicit executable path. The
    /// frontend consumes it at eval; the core only parses it and
    /// feeds `package` entries to provisioning.
    pub linters: BTreeMap<String, LinterSection>,
    /// Rate limits per throttle domain, e.g. `"api.github.com" = "2/s"`
    /// (0007 §throttling). The core attaches primitives to domains.
    #[serde(default)]
    pub throttle: BTreeMap<String, String>,
    pub settings: Settings,
}

/// User-level configuration (`~/.config/gripsack/config.toml`) — for
/// machine-local overrides that must not be committed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserConfig {
    pub fetchers: BTreeMap<String, FetcherSection>,
    pub settings: Settings,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnvSection {
    pub name: Option<String>,
    #[serde(default)]
    pub frontend: Frontend,
    /// The host entrypoint when no --host is given and the machine's
    /// hostname matches nothing in hosts/ — for role-named host files
    /// on ephemeral containers with random hostnames (enterprise
    /// review finding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_host: Option<String>,
}

/// One frontend per env repo (0005 §1) — declared, never sniffed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Frontend {
    #[default]
    Python,
    Typescript,
}

/// Frontend-environment provisioning: packages the modules import at
/// eval time (resolvers, fetcher libraries — 0002 §3). Content-cached:
/// same spec, same environment.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EvalSection {
    pub deps: Vec<String>,
    /// Environment injected into the apply process for the run's
    /// duration — build steps, fetchers, and plugins inherit it
    /// (SSL_CERT_FILE is the canonical case).
    pub env: std::collections::BTreeMap<String, String>,
}

/// A named fetcher.s tool-level wiring (0002 §4).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FetcherSection {
    /// Fetcher plugin override; default discovery is
    /// `gripfetch-<name>` on PATH.
    pub plugin: Option<String>,
    /// Provision the fetcher from a GitHub release:
    /// `owner/repo@tag` — downloaded, sha256-verified, receipted into
    /// the plugin store (0012 §move-2). Mutually exclusive with plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Explicit executable path — the offline/air-gapped route, and the
    /// registry-symmetric name for `plugin` (linters use `path`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}
/// A named linter (0010 §3, 0011 §7): provisioned from a pinned
/// package, or an explicit executable path for development.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LinterSection {
    /// Pip requirement with an `==` pin, e.g. `griplint-yazi==1.2.0`.
    pub package: Option<String>,
    /// Explicit executable path — wins over `package`, for development.
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub keep_generations: Option<u32>,
}

/// The merged, effective configuration after layering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub settings: Settings,
    pub fetchers: BTreeMap<String, FetcherSectionView>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FetcherSectionView {
    pub plugin: Option<String>,
    pub package: Option<String>,
    pub path: Option<String>,
}

/// Parse a repo `env.toml`. Errors are span-labeled diagnostics
/// pointing at the exact line (0009 §3).
pub fn parse_env(source: &str) -> Result<EnvConfig, Vec<Diagnostic>> {
    parse_as(source, "<env.toml>")
}

/// Parse a user config (`~/.config/gripsack/config.toml`).
pub fn parse_user(source: &str) -> Result<UserConfig, Vec<Diagnostic>> {
    parse_as(source, "<config.toml>")
}

/// Read and parse a repo `env.toml` from disk — spans name the file.
pub fn load_env(path: &std::path::Path) -> Result<EnvConfig, Vec<Diagnostic>> {
    let source = std::fs::read_to_string(path).map_err(|e| {
        vec![Diagnostic::error(
            codes::CONFIG,
            format!("cannot read {}: {e}", path.display()),
        )]
    })?;
    parse_as(&source, &path.display().to_string())
}

fn parse_as<T: serde::de::DeserializeOwned>(
    source: &str,
    file: &str,
) -> Result<T, Vec<Diagnostic>> {
    toml::from_str(source).map_err(|e| vec![toml_diagnostic(source, file, &e)])
}

/// Map a toml error's byte span to a line:col span and render it as our
/// diagnostic — the same shape modules get (0009 §3).
fn toml_diagnostic(source: &str, file: &str, e: &toml::de::Error) -> Diagnostic {
    let span = e.span().map(|range| {
        let (line, col) = line_col(source, range.start);
        Span {
            file: file.to_string(),
            line,
            col: Some(col),
        }
    });
    Diagnostic::error(codes::CONFIG, format!("invalid config: {}", e.message()))
        .with_label(span, "here")
        .with_help("check the key name and type against doc/settings-reference")
}

fn line_col(source: &str, offset: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut col = 1u32;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Layer user config under repo config: repo wins on conflicts, missing
/// values fall through to the user layer (0005 §2).
pub fn merge(user: Option<&UserConfig>, repo: &EnvConfig) -> Config {
    let settings = Settings {
        keep_generations: repo
            .settings
            .keep_generations
            .or_else(|| user.and_then(|u| u.settings.keep_generations)),
    };
    let mut sources: BTreeMap<String, FetcherSectionView> = BTreeMap::new();
    if let Some(user) = user {
        for (name, section) in &user.fetchers {
            sources.insert(
                name.clone(),
                FetcherSectionView {
                    plugin: section.plugin.clone(),
                    package: section.package.clone(),
                    path: section.path.clone(),
                },
            );
        }
    }
    for (name, section) in &repo.fetchers {
        sources.insert(
            name.clone(),
            FetcherSectionView {
                plugin: section.plugin.clone(),
                package: section.package.clone(),
                path: section.path.clone(),
            },
        );
    }
    Config {
        settings,
        fetchers: sources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV: &str = r#"
[env]
name = "tarek"
frontend = "typescript"

[eval]
deps = ["gripsack-fetcher-artifactory==1.2.0"]

[fetchers.artifactory]
plugin = "gripfetch-artifactory"

[settings]
keep_generations = 20
"#;

    #[test]
    fn parses_env_toml() {
        let env = parse_env(ENV).unwrap();
        assert_eq!(env.env.name.as_deref(), Some("tarek"));
        assert_eq!(env.env.frontend, Frontend::Typescript);
        assert_eq!(env.eval.deps.len(), 1);
        assert_eq!(
            env.fetchers["artifactory"].plugin.as_deref(),
            Some("gripfetch-artifactory")
        );
        assert_eq!(env.settings.keep_generations, Some(20));
    }

    #[test]
    fn typo_key_is_a_span_labeled_diagnostic() {
        let err = parse_env("[settings]\nkeep_generation = 20\n").unwrap_err();
        let d = &err[0];
        assert_eq!(d.code, gripsack_ir::codes::CONFIG);
        let span = d.labels[0].span.as_ref().unwrap();
        assert_eq!(span.line, 2);
        assert!(d.to_string().contains("keep_generation"));
    }

    #[test]
    fn unknown_top_level_key_is_an_error() {
        let err =
            parse_env("[env]\nname = \"x\"\n\n[eval]\ndeps = []\n\n[setttings]\n").unwrap_err();
        assert_eq!(err[0].code, gripsack_ir::codes::CONFIG);
    }

    #[test]
    fn parses_throttle_table() {
        let env = parse_env("[throttle]\n\"api.github.com\" = \"2/s\"\n").unwrap();
        assert_eq!(env.throttle["api.github.com"], "2/s");
    }

    #[test]
    fn defaults_are_sane() {
        let env = parse_env("").unwrap();
        assert_eq!(env.env.frontend, Frontend::Python);
        assert!(env.eval.deps.is_empty());
        assert!(env.fetchers.is_empty());
    }

    #[test]
    fn repo_wins_user_fills_gaps() {
        let user = parse_user(
            r#"
[fetchers.corp]
plugin = "gripfetch-corp"

[settings]
keep_generations = 10
"#,
        )
        .unwrap();
        let repo = parse_env("[settings]\nkeep_generations = 20\n").unwrap();
        let config = merge(Some(&user), &repo);
        // repo wins on conflict
        assert_eq!(config.settings.keep_generations, Some(20));
        // user-only source survives
        assert_eq!(
            config.fetchers["corp"].plugin.as_deref(),
            Some("gripfetch-corp")
        );
    }

    #[test]
    fn user_value_used_when_repo_silent() {
        let user = parse_user("[settings]\nkeep_generations = 10\n").unwrap();
        let repo = parse_env("").unwrap();
        assert_eq!(
            merge(Some(&user), &repo).settings.keep_generations,
            Some(10)
        );
    }
}
