//! The ownership question, asked — never guessed (0015 §7 S1).
//!
//! Interactive: an arrow-key select with the semantics laid out.
//! Non-interactive: the SAFE default with a loud note, `--mode` as the
//! preseed. The generated module file is the persisted answer.

use crate::render::Palette;
use dialoguer::Select;
use dialoguer::theme::ColorfulTheme;
use std::io::IsTerminal;

pub const DEFAULT_MODE: &str = super::generate::MODE_TRACKED_COPY;

/// The three answers, in menu order. (label, semantics)
const CHOICES: &[(&str, &str)] = &[
    (
        "owned",
        "a read-only symlink into the store — the repo is the only editor. \
         For tools that never write their own config.",
    ),
    (
        "tracked_copy",
        "a real file, hash-recorded — the app may rewrite it, and your \
         edits are detected, never clobbered. The safe choice.",
    ),
    (
        "merge",
        "one managed block inside a file other tools also write \
         (.bashrc, .profile).",
    ),
];

/// Index of the safe default in CHOICES.
const DEFAULT_INDEX: usize = 1; // tracked_copy

/// Ask the ownership question. `preseed` is --mode; non-TTY takes the
/// safe default with a note. Returns the mode string.
pub fn ask_mode(preseed: Option<&str>, palette: Palette) -> Result<String, ()> {
    if let Some(mode) = preseed {
        if CHOICES.iter().any(|(m, _)| *m == mode) {
            return Ok(mode.to_string());
        }
        eprintln!("grip: unknown mode {mode:?} — owned | tracked_copy | merge");
        return Err(());
    }
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        eprintln!(
            "{}",
            palette.warn(&format!(
                "ownership: {DEFAULT_MODE} — the safe default (no TTY to ask; \
                 --mode owned|merge overrides)"
            ))
        );
        return Ok(DEFAULT_MODE.to_string());
    }
    let styled: Vec<String> = CHOICES
        .iter()
        .map(|(mode, semantics)| {
            if palette.enabled {
                format!(
                    "{}{}",
                    palette.good(&format!("{mode:<12}")),
                    palette.dim(semantics)
                )
            } else {
                format!("{mode:<12} {semantics}")
            }
        })
        .collect();
    let theme = ColorfulTheme {
        prompt_prefix: dialoguer::console::style("?".to_string()).cyan(),
        success_prefix: dialoguer::console::style("✓".to_string()).green(),
        active_item_prefix: dialoguer::console::style("❯".to_string()).green().bold(),
        picked_item_prefix: dialoguer::console::style("❯".to_string()).green().bold(),
        inactive_item_prefix: dialoguer::console::style(" ".to_string()),
        ..ColorfulTheme::default()
    };
    let choice = Select::with_theme(&theme)
        .with_prompt("how should gripsack own these files?")
        .items(&styled)
        .default(DEFAULT_INDEX)
        .interact()
        .map_err(|e| {
            eprintln!("grip: prompt failed: {e}");
        })?;
    let (mode, _) = CHOICES[choice];
    Ok(mode.to_string())
}

/// A one-line confirmation of the chosen mode, for the flow output.
pub fn mode_line(mode: &str) -> String {
    let (_, semantics) = CHOICES
        .iter()
        .find(|(m, _)| *m == mode)
        .expect("mode validated by ask_mode");
    format!("ownership: {mode} — {semantics}")
}

/// The final go/no-go before apply. Returns false to abort.
pub fn confirm_apply(palette: Palette) -> bool {
    use std::io::Write;
    eprint!("{}", palette.cyan("apply? [y/N] "));
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok() && matches!(answer.trim(), "y" | "Y")
}
