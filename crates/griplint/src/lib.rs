//! The config-linting engine: first-party linters are versioned data
//! packs checked by one engine linked into grip.

mod admission;
pub mod checks;
mod difflib;
pub mod document;
#[cfg(test)]
mod golden;
mod types;
pub mod value;

use std::path::Path;
pub use types::{FileTable, Format, Meta, Pack, Rule, RuleValue, SectionRules, ValueType};

/// Pack loading failures — io or admission (including TOML syntax).
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("cannot read {0}: {1}")]
    Io(std::path::PathBuf, std::io::Error),
    #[error("invalid pack: {0}")]
    Parse(#[from] toml::de::Error),
}

pub fn load_pack(path: &Path) -> Result<Pack, PackError> {
    let text = std::fs::read_to_string(path).map_err(|e| PackError::Io(path.to_path_buf(), e))?;
    load_pack_str(&text)
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

/// The pack for a tool name, parsed through the same admission boundary.
pub fn pack_for(tool: &str) -> Option<Pack> {
    let (_, text) = PACKS.iter().find(|(name, _)| *name == tool)?;
    load_pack_str(text).ok()
}

/// Parse and admit a pack. Direct `toml::from_str::<Pack>` is equally strict.
pub fn load_pack_str(text: &str) -> Result<Pack, PackError> {
    Ok(toml::from_str(text)?)
}
