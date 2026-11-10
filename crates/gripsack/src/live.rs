//! Live apply rendering: lifecycle events print the moment the executor
//! emits them. Phase and "started" progress go to stderr (a spinner on a
//! terminal, one plain line each otherwise); step lines and verification
//! blocks are the apply report and go to stdout. A verification block is
//! assembled in full and written with one call, so checks never interleave.
//! Captured tool output passes the untrusted-text boundary before it is
//! written; renderer colour follows the palette (TTY, `NO_COLOR`).

use crate::render::Palette;
use gripsack_exec::{Progress, StepReport, Tone, Verification};
use gripsack_process::terminal;
use std::fmt::Write as _;
use std::io::{IsTerminal, Write};

/// Longest output line kept in a collapsed success summary.
const SUMMARY_CHARS: usize = 60;

/// How much of a passing verification's output to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// One summary line on success; failures always show everything.
    Collapsed,
    /// Every line of every verification.
    Full,
}

pub struct LiveRenderer {
    palette: Palette,
    detail: Detail,
    /// Width of the output-name column (the apply report convention).
    width: usize,
    spinner: Option<indicatif::ProgressBar>,
    /// The spinner message of the current phase, restored once a unit ends.
    phase: std::sync::Mutex<String>,
}

impl LiveRenderer {
    /// The spinner exists only on a terminal stderr; piped runs get plain lines.
    /// Before a workspace exists there is nothing to render live; evaluation
    /// announces itself so a slow frontend is never silent.
    pub fn announce_evaluation() {
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all("evaluating workspace…\n".as_bytes());
        let _ = stderr.flush();
    }

    pub fn new(palette: Palette, detail: Detail, ir: &gripsack_ir::Ir) -> Self {
        let spinner = std::io::stderr().is_terminal().then(|| {
            let template = if palette.colors_allowed() {
                "{spinner:.green} {msg}"
            } else {
                "{spinner} {msg}"
            };
            let pb = indicatif::ProgressBar::new_spinner();
            pb.set_style(
                indicatif::ProgressStyle::with_template(template)
                    .expect("static template")
                    // the retro braille snake (rootle's loading eye candy,
                    // adopted): a coil that never stops looking busy
                    .tick_chars("⣾⣽⣻⢿⡿⣟⣯⣷ "),
            );
            pb.enable_steady_tick(std::time::Duration::from_millis(80));
            pb
        });
        // align to the profiles and packages — the outputs that report
        use gripsack_ir::workspace_model::WorkspaceOutput as O;
        let width = ir
            .workspace
            .outputs
            .iter()
            .filter(|output| matches!(output, O::Profile(_) | O::Package(_)))
            .map(|output| output.name().chars().count())
            .max()
            .unwrap_or(0);
        Self {
            palette,
            detail,
            width,
            spinner,
            phase: Default::default(),
        }
    }

    pub fn render(&self, event: Progress<'_>) {
        match event {
            Progress::Phase(phase) => {
                if let Ok(mut current) = self.phase.lock() {
                    *current = format!("{}…", phase.describe());
                }
                self.progress(None, phase.describe())
            }
            Progress::Started { output, activity } => self.progress(Some(output), activity),
            Progress::Step(report) => self.report(&self.step(report)),
            Progress::Verified(verification) => {
                self.report(&self.block(verification));
                if let (Some(pb), Ok(phase)) = (&self.spinner, self.phase.lock()) {
                    pb.set_message(phase.clone());
                }
            }
        }
    }

    /// Stop the spinner before the final summary line.
    pub fn finish(&self) {
        if let Some(pb) = &self.spinner {
            pb.finish_and_clear();
        }
    }

    fn progress(&self, output: Option<&str>, activity: &str) {
        let activity = terminal::display(activity);
        match (&self.spinner, output) {
            (Some(pb), Some(output)) => {
                pb.set_message(format!("{} · {activity}", terminal::display(output)))
            }
            (Some(pb), None) => pb.set_message(format!("{activity}…")),
            (None, output) => {
                let line = match output {
                    Some(output) => format!(
                        "  {:>width$} … {activity}\n",
                        terminal::display(output).to_string(),
                        width = self.width
                    ),
                    None => format!("{activity}…\n"),
                };
                let mut stderr = std::io::stderr().lock();
                let _ = stderr.write_all(line.as_bytes());
                let _ = stderr.flush();
            }
        }
    }

    /// One atomic stdout write, above the spinner when there is one.
    fn report(&self, text: &str) {
        let write = || {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(text.as_bytes());
            let _ = stdout.flush();
        };
        match &self.spinner {
            Some(pb) => pb.suspend(write),
            None => write(),
        }
    }

    fn column(&self, output: &str) -> String {
        let output = format!(
            "{:>width$}",
            terminal::display(output).to_string(),
            width = self.width
        );
        self.palette.cyan(&output)
    }

    fn step(&self, report: &StepReport) -> String {
        let symbol = match report.tone {
            Tone::Warned => self.palette.warn("⚠"),
            Tone::Unchanged => self.palette.dim("·"),
            Tone::Done => self.palette.good("✓"),
        };
        format!(
            "  {} {symbol} {}\n",
            self.column(&report.output),
            terminal::display(&report.summary)
        )
    }

    fn block(&self, verification: &Verification) -> String {
        let label = &verification.label;
        let mut block = format!(
            "  {} ▸ {} {} {}\n",
            self.column(&label.owner),
            label.kind.describe(),
            self.palette
                .bold(&terminal::display(&label.name).to_string()),
            self.palette
                .dim(&format!("$ {}", terminal::display(&label.command))),
        );
        let indent = " ".repeat(self.width + 4);
        let stdout = lines(&verification.stdout);
        let stderr = lines(&verification.stderr);
        let expanded = !verification.passed || self.detail == Detail::Full;
        if expanded {
            let gutter = |block: &mut String, mark: String, line: &str| {
                let space = if line.is_empty() { "" } else { " " };
                let _ = writeln!(block, "{indent}{mark}{space}{line}");
            };
            for line in &stdout {
                gutter(&mut block, self.palette.dim("│"), line);
            }
            for line in &stderr {
                gutter(&mut block, self.palette.warn("┆"), line);
            }
            if verification.truncated {
                let _ = writeln!(block, "{indent}{}", self.palette.dim("(output truncated)"));
            }
        }
        let duration = format!("{:.2}s", verification.duration.as_secs_f32());
        if verification.passed {
            let summary = if expanded {
                String::new()
            } else {
                collapsed(&stdout, &stderr)
            };
            let _ = writeln!(
                block,
                "{indent}{} passed in {duration}{}",
                self.palette.good("✓"),
                self.palette.dim(&summary)
            );
        } else {
            let detail = verification
                .detail
                .as_deref()
                .map(|detail| format!(" ({})", terminal::display(detail)))
                .unwrap_or_default();
            let _ = writeln!(
                block,
                "{indent}{} {} after {duration}{detail}",
                self.palette.error("✗"),
                self.palette.error("FAILED"),
            );
        }
        block
    }
}

/// Escaped display lines of one captured stream; trailing blank lines dropped.
fn lines(bytes: &[u8]) -> Vec<String> {
    let mut lines: Vec<String> = bytes
        .split(|byte| *byte == b'\n')
        .map(|line| {
            let mut escaped = Vec::with_capacity(line.len());
            let _ = terminal::write_output(&mut escaped, line);
            String::from_utf8_lossy(&escaped).into_owned()
        })
        .collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

/// The one-line summary of a passing verification's output.
fn collapsed(stdout: &[String], stderr: &[String]) -> String {
    let all: Vec<&String> = stdout.iter().chain(stderr).collect();
    let Some(first) = all.iter().find(|line| !line.trim().is_empty()) else {
        return " · no output".into();
    };
    let first = first.trim();
    let first = match first.char_indices().nth(SUMMARY_CHARS) {
        Some((cut, _)) => format!("{}…", &first[..cut]),
        None => first.to_owned(),
    };
    let more = all.len() - 1;
    if more == 0 {
        format!(" · {first}")
    } else {
        format!(
            " · {first} (+{more} more line{}, --verbose shows all)",
            if more == 1 { "" } else { "s" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_exec::{VerificationKind, VerificationLabel};
    use std::time::Duration;

    fn verification(passed: bool, stdout: &[u8], stderr: &[u8]) -> Verification {
        Verification {
            label: VerificationLabel {
                kind: VerificationKind::Hook,
                name: "verify-rg".into(),
                owner: "personal".into(),
                command: "rg --version".into(),
            },
            passed,
            duration: Duration::from_millis(40),
            detail: (!passed).then(|| "exit 3".into()),
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
            truncated: false,
        }
    }

    fn renderer(detail: Detail) -> LiveRenderer {
        LiveRenderer {
            palette: Palette::plain(),
            detail,
            width: 8,
            spinner: None,
            phase: Default::default(),
        }
    }

    #[test]
    fn success_collapses_to_one_summary_line() {
        let block = renderer(Detail::Collapsed).block(&verification(
            true,
            b"ripgrep 14.1.1\n\nfeatures:+pcre2\n",
            b"",
        ));
        assert_eq!(
            block,
            "  personal ▸ hook verify-rg $ rg --version\n            ✓ passed in 0.04s · ripgrep 14.1.1 (+2 more lines, --verbose shows all)\n"
        );
    }

    #[test]
    fn verbose_and_failure_expand_every_line() {
        let verbose = renderer(Detail::Full).block(&verification(true, b"one\ntwo\n", b"warn\n"));
        assert_eq!(
            verbose,
            "  personal ▸ hook verify-rg $ rg --version\n            │ one\n            │ two\n            ┆ warn\n            ✓ passed in 0.04s\n"
        );
        let failed = renderer(Detail::Collapsed).block(&verification(false, b"out\n", b"bad\n"));
        assert!(
            failed.contains("│ out\n") && failed.contains("┆ bad\n"),
            "{failed}"
        );
        assert!(
            failed.ends_with("✗ FAILED after 0.04s (exit 3)\n"),
            "{failed}"
        );
    }

    #[test]
    fn captured_controls_never_reach_the_terminal() {
        let block = renderer(Detail::Full).block(&verification(true, b"\x1b]0;x\x07\rbody\n", b""));
        assert!(!block.contains('\x1b') && !block.contains('\r') && !block.contains('\x07'));
        assert!(block.contains("body"));
    }

    #[test]
    fn empty_output_says_so() {
        let block = renderer(Detail::Collapsed).block(&verification(true, b"", b""));
        assert!(
            block.ends_with("✓ passed in 0.04s · no output\n"),
            "{block}"
        );
    }
}
