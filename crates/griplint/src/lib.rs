//! The config-linting engine (plan/0012 §move-3): first-party linters
//! are data packs — versioned key tables as TOML data — checked by one
//! engine linked into grip. No binaries, no provisioning, no lifecycle.
//!
//! This crate currently ships the pack model and loader; the checker
//! (format parsers with span tracking + the rule walk) lands next.

pub mod checks;
mod difflib;
pub mod document;
#[cfg(test)]
mod golden;
pub mod value;

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// A pack's `[meta]` — which tool, which files, which config format,
/// and the tool-version prefixes these tables are current against
/// (a pinned version outside them earns the W10 coverage warning).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Meta {
    pub tool: String,
    pub handles: Vec<String>,
    pub format: String,
    pub supported: Vec<String>,
    #[serde(default)]
    pub lenient: Vec<String>,
    pub series: String,
    /// The exact W10 coverage-warning text (captured from the reference
    /// resolver); `{version}` is the pinned version placeholder.
    #[serde(default)]
    pub coverage_warning: Option<String>,
}

/// One key rule: expected value types, an optional closed choice set,
/// and a deprecation pointer when the key was renamed upstream.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Rule {
    #[serde(rename = "types")]
    pub types: Vec<String>,
    #[serde(default)]
    pub choices: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub deprecated: Option<String>,
}

/// A section's rules, or a marker: `_free` (FREE_FORM — anything goes,
/// descent included), `_rule` (the section type-checked as a whole).
/// A rule value of the bare string "free" is key-level FREE_FORM.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum SectionRules {
    /// Every key maps to a rule (or "free").
    Keys(BTreeMap<String, RuleValue>),
    /// Markers only (_free / _rule / no_table sit alongside Keys).
    Markers(BTreeMap<String, serde_json::Value>),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RuleValue {
    Free(String),
    Rule(Rule),
}

/// One tool's pack: meta plus per-file rule sections (a resolver may
/// key tables by basename — ruff.toml vs a lenient pyproject.toml).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Pack {
    pub meta: Meta,
    #[serde(default)]
    pub files: BTreeMap<String, FileTable>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FileTable {
    #[serde(default)]
    pub no_table: bool,
    #[serde(default)]
    pub rules: BTreeMap<String, SectionRules>,
}

/// Pack loading failures — io or parse.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("cannot read {0}: {1}")]
    Io(std::path::PathBuf, std::io::Error),
    #[error("invalid pack: {0}")]
    Parse(#[from] toml::de::Error),
}

pub fn load_pack(path: &Path) -> Result<Pack, PackError> {
    let text = std::fs::read_to_string(path).map_err(|e| PackError::Io(path.to_path_buf(), e))?;
    Ok(toml::from_str(&text)?)
}

/// Every first-party pack, embedded — a linter needs nothing but grip.
pub static PACKS: &[(&str, &str)] = &[
    ("alacritty", include_str!("../packs/alacritty.toml")),
    ("atuin", include_str!("../packs/atuin.toml")),
    ("bacon", include_str!("../packs/bacon.toml")),
    ("bottom", include_str!("../packs/bottom.toml")),
    ("broot", include_str!("../packs/broot.toml")),
    ("claude-code", include_str!("../packs/claude-code.toml")),
    ("gh-dash", include_str!("../packs/gh-dash.toml")),
    ("git-cliff", include_str!("../packs/git-cliff.toml")),
    ("glow", include_str!("../packs/glow.toml")),
    ("harlequin", include_str!("../packs/harlequin.toml")),
    ("helix", include_str!("../packs/helix.toml")),
    ("jj", include_str!("../packs/jj.toml")),
    ("mise", include_str!("../packs/mise.toml")),
    ("procs", include_str!("../packs/procs.toml")),
    ("rio", include_str!("../packs/rio.toml")),
    ("ruff", include_str!("../packs/ruff.toml")),
    ("starship", include_str!("../packs/starship.toml")),
    ("superfile", include_str!("../packs/superfile.toml")),
    ("television", include_str!("../packs/television.toml")),
    ("tuicr", include_str!("../packs/tuicr.toml")),
    ("yazi", include_str!("../packs/yazi.toml")),
    ("zed", include_str!("../packs/zed.toml")),
    ("zola", include_str!("../packs/zola.toml")),
];

/// The pack for a tool name, parsed.
pub fn pack_for(tool: &str) -> Option<Pack> {
    let (_, text) = PACKS.iter().find(|(name, _)| *name == tool)?;
    load_pack_str(text).ok()
}

/// Parse a pack from text (the loader's file twin).
pub fn load_pack_str(text: &str) -> Result<Pack, PackError> {
    Ok(toml::from_str(text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pack in-tree loads into the model — the data stays honest.
    #[test]
    fn all_packs_load() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
        let mut count = 0;
        for entry in std::fs::read_dir(&dir).expect("packs dir") {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let pack = load_pack(&path)
                .unwrap_or_else(|e| panic!("pack {} fails to load: {e}", path.display()));
            assert!(!pack.meta.tool.is_empty(), "{}", path.display());
            assert!(!pack.meta.handles.is_empty(), "{}", path.display());
            assert!(!pack.meta.supported.is_empty(), "{}", path.display());
            assert!(!pack.files.is_empty(), "{}", path.display());
            count += 1;
        }
        assert!(count >= 22, "expected the full linter set, found {count}");
    }

    /// The helix pack spot-check: sections, choice sets, FREE_FORM.
    #[test]
    fn helix_pack_shape() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs");
        let pack = load_pack(&dir.join("helix.toml")).expect("helix pack");
        assert_eq!(pack.meta.format, "toml");
        let rules = &pack.files["config.toml"].rules;
        let editor = match &rules["editor"] {
            SectionRules::Keys(keys) => keys,
            other => panic!("editor should be keys, got {other:?}"),
        };
        match &editor["line-number"] {
            RuleValue::Rule(rule) => {
                assert_eq!(rule.types, ["string"]);
                assert_eq!(rule.choices.as_ref().map(|c| c.len()), Some(2),);
            }
            other => panic!("line-number should be a rule, got {other:?}"),
        }
        assert!(rules.contains_key("keys"), "FREE_FORM section survives");
    }
}
