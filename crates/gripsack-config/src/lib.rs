//! Tool configuration (plan/0005 §2): `env.toml` at the repo root plus
//! the layered merge. Configuration is pure data, read before any code
//! runs — it can never depend on eval results (0005 §3).
//!
//! Precedence (later wins): built-in defaults < user config
//! (`~/.config/gripsack/config.toml`) < repo `env.toml` < env vars <
//! CLI flags. The last two layers resolve in the CLI, not here.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid config TOML: {0}")]
    Toml(#[from] toml::de::Error),
}

/// Repo-level configuration (`env.toml`), committed — the env is
/// self-describing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EnvConfig {
    pub env: EnvSection,
    pub eval: EvalSection,
    pub sources: BTreeMap<String, SourceSection>,
    pub settings: Settings,
}

/// User-level configuration (`~/.config/gripsack/config.toml`) — for
/// machine-local overrides that must not be committed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserConfig {
    pub sources: BTreeMap<String, SourceSection>,
    pub settings: Settings,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EnvSection {
    pub name: Option<String>,
    #[serde(default)]
    pub frontend: Frontend,
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
/// eval time (resolvers, sourcerer libraries — 0002 §3). Content-cached:
/// same spec, same environment.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EvalSection {
    pub deps: Vec<String>,
}

/// A named external source's tool-level wiring (0002 §4).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SourceSection {
    /// Sourcerer executable override; default discovery is
    /// `gripsource-<name>` on PATH.
    pub plugin: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub keep_generations: Option<u32>,
}

/// The merged, effective configuration after layering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub settings: Settings,
    pub sources: BTreeMap<String, SourceSectionView>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceSectionView {
    pub plugin: Option<String>,
}

pub fn parse_env(toml_str: &str) -> Result<EnvConfig, ConfigError> {
    Ok(toml::from_str(toml_str)?)
}

pub fn parse_user(toml_str: &str) -> Result<UserConfig, ConfigError> {
    Ok(toml::from_str(toml_str)?)
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
    let mut sources: BTreeMap<String, SourceSectionView> = BTreeMap::new();
    if let Some(user) = user {
        for (name, section) in &user.sources {
            sources.insert(
                name.clone(),
                SourceSectionView {
                    plugin: section.plugin.clone(),
                },
            );
        }
    }
    for (name, section) in &repo.sources {
        sources.insert(
            name.clone(),
            SourceSectionView {
                plugin: section.plugin.clone(),
            },
        );
    }
    Config { settings, sources }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV: &str = r#"
[env]
name = "tarek"
frontend = "typescript"

[eval]
deps = ["gripsack-sourcerer-artifactory==1.2.0"]

[sources.artifactory]
plugin = "gripsource-artifactory"

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
            env.sources["artifactory"].plugin.as_deref(),
            Some("gripsource-artifactory")
        );
        assert_eq!(env.settings.keep_generations, Some(20));
    }

    #[test]
    fn defaults_are_sane() {
        let env = parse_env("").unwrap();
        assert_eq!(env.env.frontend, Frontend::Python);
        assert!(env.eval.deps.is_empty());
        assert!(env.sources.is_empty());
    }

    #[test]
    fn repo_wins_user_fills_gaps() {
        let user = parse_user(
            r#"
[sources.corp]
plugin = "gripsource-corp"

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
            config.sources["corp"].plugin.as_deref(),
            Some("gripsource-corp")
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
