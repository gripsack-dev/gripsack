//! Diagnostic rendering (0004 §3): colors when the terminal supports
//! them, source snippets when the file is reachable. Spans reference the
//! user's frontend code — a missing file degrades to the header alone,
//! never an error.

use gripsack_ir::{Diagnostic, Ir, Severity, Span};
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default)]
pub struct Palette {
    pub enabled: bool,
}

impl Palette {
    pub fn detect() -> Self {
        Palette {
            enabled: std::io::stdout().is_terminal(),
        }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Palette::default()
    }

    /// Styling helpers: colors follow the terminal (main.rs doc) —
    /// piped output is plain. Every ad-hoc `.green()` at a call site
    /// was a leak of that contract; these make the gate structural.
    pub fn good(&self, text: &str) -> String {
        self.style(text, |t| t.green().bold().to_string())
    }

    pub fn warn(&self, text: &str) -> String {
        self.style(text, |t| t.yellow().bold().to_string())
    }

    pub fn badge(&self, text: &str) -> String {
        self.style(text, |t| t.blue().bold().to_string())
    }

    pub fn cyan(&self, text: &str) -> String {
        self.style(text, |t| t.cyan().to_string())
    }

    pub fn dim(&self, text: &str) -> String {
        self.style(text, |t| t.dimmed().to_string())
    }

    pub fn error(&self, text: &str) -> String {
        self.style(text, |t| t.red().bold().to_string())
    }

    fn style(&self, text: &str, styled: impl FnOnce(&str) -> String) -> String {
        if self.enabled {
            styled(text)
        } else {
            text.to_string()
        }
    }
}

/// Render one diagnostic, with a source snippet for every span whose
/// file can be read.
pub fn render_diagnostic(d: &Diagnostic, palette: Palette) -> String {
    let mut out = String::new();
    let header = format!("{}[{}]: {}", d.severity, d.code, d.message);
    out.push_str(&match (d.severity, palette.enabled) {
        (Severity::Error, true) => header.red().bold().to_string(),
        (Severity::Warning, true) => header.yellow().bold().to_string(),
        _ => header,
    });
    for label in &d.labels {
        match &label.span {
            Some(span) => {
                let arrow = format!("\n  --> {span}");
                out.push_str(&if palette.enabled {
                    arrow.blue().to_string()
                } else {
                    arrow
                });
                out.push_str(&snippet(span, &label.note, palette));
            }
            None if !label.note.is_empty() => {
                out.push_str(&format!("\n  = {}", label.note));
            }
            None => {}
        }
    }
    if let Some(help) = &d.help {
        let line = format!("\n  help: {help}");
        out.push_str(&if palette.enabled {
            line.green().to_string()
        } else {
            line
        });
    }
    out
}

/// The rustc-style snippet: gutter, source line, caret at the column.
fn snippet(span: &Span, note: &str, palette: Palette) -> String {
    let Ok(contents) = std::fs::read_to_string(&span.file) else {
        return String::new();
    };
    let Some(source_line) = contents.lines().nth((span.line - 1) as usize) else {
        return String::new();
    };
    let gutter = format!("{:>3} |", span.line);
    let caret_pad = span.col.unwrap_or(1).saturating_sub(1) as usize;
    let caret = format!("{}^", " ".repeat(caret_pad));
    let mut out = format!("\n   |\n {gutter} {source_line}\n   | {caret}");
    if !note.is_empty() {
        out.push(' ');
        out.push_str(note);
    }
    if palette.enabled {
        format!("\n{}", out.dimmed())
    } else {
        out
    }
}

/// Render one module's plan (0007 §5): what it fetches, deploys, needs,
/// and which wave it lands in.
pub fn render_module(ir: &Ir, name: &str, waves: &[Vec<String>], palette: Palette) -> String {
    let Some(module) = ir.modules.get(name) else {
        return format!("no module {name:?}");
    };
    let wave = waves
        .iter()
        .position(|w| w.iter().any(|m| m == name))
        .map(|i| i.to_string())
        .unwrap_or_else(|| "?".into());
    let title = format!("{name}  (wave {wave})");
    let mut out = if palette.enabled {
        title.green().bold().to_string()
    } else {
        title
    };

    if let Some(fetch) = &module.fetch {
        out.push_str(&format!(
            "\n  fetch    {}",
            gripsack_exec::report::describe_fetch(fetch)
        ));
    }
    for entry in module.install.iter() {
        out.push_str(&format!(
            "\n  install  {} → {} ({:?})",
            entry.from, entry.to, entry.mode
        ));
    }
    for entry in module.config.iter() {
        out.push_str(&format!(
            "\n  config   {} → {} ({:?})",
            entry.from, entry.to, entry.mode
        ));
    }
    for dep in module.depends.iter() {
        out.push_str(&format!("\n  depends  {} ({:?})", dep.module, dep.edge));
    }
    let dependents: Vec<_> = ir
        .modules
        .iter()
        .filter(|(_, m)| m.depends.iter().any(|d| d.module == name))
        .map(|(n, _)| n.as_str())
        .collect();
    if !dependents.is_empty() {
        out.push_str(&format!("\n  blocks   {}", dependents.join(", ")));
    }
    if let Some(steps) = &module.steps {
        out.push_str("\n  steps");
        for step in steps {
            let needs = if step.needs.is_empty() {
                String::new()
            } else {
                format!(" ← {}", step.needs.join(", "))
            };
            out.push_str(&format!("\n    {}{needs}", step.id));
        }
    }
    out
}

/// Render all diagnostics, warnings first-class but non-fatal.
pub fn render_diagnostics(diagnostics: &[Diagnostic], palette: Palette) -> String {
    diagnostics
        .iter()
        .map(|d| render_diagnostic(d, palette))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `grip plan`'s change section: what apply would do, computed against
/// the current generation (0004 pass 5 — the diff that sells the
/// architecture). Config entries hash offline; fetched modules show
/// their store-path satisfaction without a fetch.
pub fn diff_section(
    ir: &Ir,
    repo: &Path,
    adopting: &std::collections::BTreeSet<String>,
    palette: Palette,
) -> String {
    let home = gripsack_store::gripsack_home();
    let current = gripsack_store::current_generation(&home)
        .and_then(|n| gripsack_store::read_manifest(&home, n).ok());
    let c = |s: &str| {
        if palette.enabled {
            s.cyan().to_string()
        } else {
            s.to_string()
        }
    };
    let b = |s: &str| {
        if palette.enabled {
            s.bold().to_string()
        } else {
            s.to_string()
        }
    };
    let mut out = vec![match &current {
        Some(m) => format!("{} generation {}:", b("changes vs"), m.number),
        None => b("changes: no current generation (first apply)"),
    }];
    let mut declared: Vec<&str> = Vec::new();

    for (name, module) in &ir.modules {
        let mut lines = Vec::new();
        let mut non_config_note = None;
        for entry in module.install.iter().chain(module.config.iter()) {
            declared.push(entry.to.as_str());
            let dest = crate::commands::expand_home(&entry.to);
            let repo_file = repo.join(&entry.from);
            if !repo_file.is_file() {
                // a fetched payload: identity check without fetching
                non_config_note = Some(if module.fetch.is_some() {
                    "fetch → deploy (pin-resolved at apply)"
                } else {
                    "steps → deploy"
                });
                continue;
            }
            let new_hash = match gripsack_store::canonical_file_hash(&repo_file) {
                Ok(h) => h,
                Err(_) => {
                    lines.push(format!(
                        "  ! {} → {} (source missing)",
                        entry.from, entry.to
                    ));
                    continue;
                }
            };
            let recorded = current.as_ref().and_then(|m| {
                m.modules
                    .get(name)
                    .and_then(|s| s.entries.iter().find(|e| e.to == entry.to))
            });
            match recorded {
                Some(rec) if rec.hash == new_hash => {
                    lines.push(format!("  = {} (satisfied)", entry.to))
                }
                Some(_) => lines.push(format!("  ~ {} → {} (update)", entry.from, entry.to)),
                None => {
                    let foreign = dest.symlink_metadata().is_ok()
                        && !std::fs::read_link(&dest)
                            .map(|t| t.starts_with(&home))
                            .unwrap_or(false);
                    if adopting.contains(entry.to.as_str()) {
                        // 0015 §7 S6: this take-over is the point of the
                        // command — say so, don't demand a flag
                        lines.push(format!("  ↻ {} will be adopted (prior recorded)", entry.to));
                    } else if foreign {
                        lines.push(format!(
                            "  ! {} exists, not ours — needs --take-over",
                            entry.to
                        ));
                    } else {
                        lines.push(format!("  + {} → {} (new)", entry.from, entry.to));
                    }
                }
            }
        }
        if let Some(note) = non_config_note {
            lines.push(format!("  · {note}"));
        }
        if !lines.is_empty() {
            out.push(format!("  {}", c(name)));
            out.extend(lines);
        }
    }

    // prunes: recorded destinations no longer declared
    if let Some(manifest) = &current {
        for (name, state) in &manifest.modules {
            let mut prunes: Vec<String> = state
                .entries
                .iter()
                .filter(|e| !declared.contains(&e.to.as_str()))
                .map(|e| format!("  - {} (prune)", e.to))
                .collect();
            if !prunes.is_empty() {
                out.push(format!("  {}", c(name)));
                out.append(&mut prunes);
            }
        }
    }
    if out.len() == 1 {
        out.push("  nothing would change".into());
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::codes;

    #[test]
    fn renders_header_and_help_without_color() {
        let d = Diagnostic::error(codes::UNKNOWN_DEPENDENCY, "module \"a\" is unknown")
            .with_help("declare it in modules/");
        let out = render_diagnostic(&d, Palette::plain());
        assert!(out.contains("error[E101]"));
        assert!(out.contains("help: declare it"));
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn renders_snippet_when_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("mod.py");
        std::fs::write(&file, "line one\nline two\nline three\n").unwrap();
        let d = Diagnostic::error(codes::BAD_DESTINATION, "bad dest").with_label(
            Some(Span {
                file: file.to_string_lossy().into_owned(),
                line: 2,
                col: Some(3),
            }),
            "here",
        );
        let out = render_diagnostic(&d, Palette::plain());
        assert!(out.contains("line two"));
        assert!(out.contains("  ^ here"));
    }

    #[test]
    fn missing_file_degrades_gracefully() {
        let d = Diagnostic::error(codes::BAD_DESTINATION, "bad dest").with_label(
            Some(Span {
                file: "/nonexistent/mod.py".into(),
                line: 1,
                col: None,
            }),
            "here",
        );
        let out = render_diagnostic(&d, Palette::plain());
        assert!(out.contains("--> /nonexistent/mod.py:1"));
        assert!(!out.contains('|'));
    }
}
