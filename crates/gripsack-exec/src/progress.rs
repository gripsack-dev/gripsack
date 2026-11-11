//! Live lifecycle progress. Each event is delivered the moment its unit of
//! work starts or finishes — never collected and replayed at the end — so a
//! frontend can stream an apply as it happens. Verification blocks carry the
//! complete captured output of one check or hook; the run log records it in
//! full whatever the terminal chooses to show.
use crate::report::StepReport;
use gripsack_ir::workspace_model::{WorkspaceArg, WorkspaceCommand};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

/// One streamed lifecycle event.
pub enum Progress<'a> {
    /// A lifecycle phase begins.
    Phase(Phase),
    /// A unit begins work that may take a while (acquisition, a check).
    Started { output: &'a str, activity: &'a str },
    /// A step finished; the same report is also returned in the result.
    Step(&'a StepReport),
    /// A check or hook finished; its output is complete and owned by this event.
    Verified(&'a Verification),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Acquire, materialize and stage packages and profile candidates.
    Prepare,
    /// Publish profile content and write destinations.
    Deploy,
    /// Run pre-flip checks against the written candidate.
    Verify,
    /// Run post-commit hooks and adapters for the new generation.
    Activate,
}

impl Phase {
    pub fn describe(self) -> &'static str {
        match self {
            Phase::Prepare => "preparing packages and profiles",
            Phase::Deploy => "deploying profiles",
            Phase::Verify => "verifying candidate",
            Phase::Activate => "activating generation",
        }
    }
}

/// Thread-safe receiver of lifecycle events.
pub type ProgressCallback = Box<dyn Fn(Progress<'_>) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationKind {
    /// A file check declared on a profile file.
    Check,
    /// A workspace hook.
    Hook,
    /// A built-in activation adapter (fonts, services, ...).
    Adapter,
}

impl VerificationKind {
    pub fn describe(self) -> &'static str {
        match self {
            VerificationKind::Check => "check",
            VerificationKind::Hook => "hook",
            VerificationKind::Adapter => "adapter",
        }
    }
}

/// What a verification is: its declared name, owning output and command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationLabel {
    pub kind: VerificationKind,
    pub name: String,
    pub owner: String,
    /// A one-line, display-only rendering of the command. Never logged:
    /// retained diagnostics carry identities, not argv bytes.
    pub command: String,
}

/// Labels of the hooks and deployed checks prepared by this run, keyed by
/// their retained context path. Replayed intents without one still render,
/// under a generic label.
pub type VerificationLabels = BTreeMap<PathBuf, VerificationLabel>;

/// Longest command rendering kept on a header line.
const COMMAND_DISPLAY_CHARS: usize = 72;

impl VerificationLabel {
    pub fn for_command(
        kind: VerificationKind,
        name: &str,
        owner: &str,
        command: &WorkspaceCommand,
    ) -> Self {
        Self {
            kind,
            name: name.to_owned(),
            owner: owner.to_owned(),
            command: display_command(command),
        }
    }
}

/// One-line description of how a supervised process ended.
pub fn process_status(receipt: &gripsack_host::process::ProcessReceipt) -> String {
    use gripsack_host::process::ProcessDisposition as D;
    match (&receipt.disposition, receipt.exit_code, receipt.signal) {
        (D::Exited, Some(code), _) => format!("exit {code}"),
        (D::Exited, None, Some(signal)) => format!("signal {signal}"),
        (disposition, _, _) => format!("{disposition:?}").to_lowercase(),
    }
}

fn display_command(command: &WorkspaceCommand) -> String {
    let words: Vec<String> = match command {
        WorkspaceCommand::Exec { argv, .. } => argv.iter().map(display_argument).collect(),
        WorkspaceCommand::RunBash {
            interpreter, body, ..
        } => vec![
            display_argument(interpreter),
            "-c".into(),
            quote(
                body.lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or(""),
            ),
        ],
    };
    let text = words.join(" ");
    match text.char_indices().nth(COMMAND_DISPLAY_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

fn display_argument(argument: &WorkspaceArg) -> String {
    match argument {
        WorkspaceArg::Literal { value } => quote(value),
        WorkspaceArg::PackageCommand { command, .. } => command.clone(),
        WorkspaceArg::Artifact { output, selector } => format!("<{output}:{selector}>"),
        WorkspaceArg::Input { input } => format!("<input {input}>"),
        WorkspaceArg::Source { selector } if selector.is_empty() || selector == "." => {
            "<subject>".into()
        }
        WorkspaceArg::Source { selector } => format!("<subject/{selector}>"),
        WorkspaceArg::Output { selector } => format!("<output/{selector}>"),
    }
}

fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The completed record of one check or hook run.
#[derive(Debug, Clone)]
pub struct Verification {
    pub label: VerificationLabel,
    pub passed: bool,
    pub duration: Duration,
    /// Exit status or failure reason when the run did not pass.
    pub detail: Option<String>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Bounded capture dropped bytes (stdout head / stderr tail are kept).
    pub truncated: bool,
}

/// The executor's handle on the optional callback. Copyable, so it threads
/// through preparation without borrowing the whole context.
#[derive(Clone, Copy, Default)]
pub struct Reporter<'a> {
    callback: Option<&'a ProgressCallback>,
}

impl<'a> Reporter<'a> {
    pub fn new(callback: Option<&'a ProgressCallback>) -> Self {
        Self { callback }
    }

    /// Nobody renders events; verification output falls back to the raw
    /// stream so legacy callers keep their behavior.
    pub fn silent() -> Self {
        Self::default()
    }

    pub fn is_live(&self) -> bool {
        self.callback.is_some()
    }

    pub fn phase(&self, phase: Phase) {
        tracing::info!(phase = phase.describe(), "phase");
        if let Some(callback) = self.callback {
            callback(Progress::Phase(phase));
        }
    }

    pub fn started(&self, output: &str, activity: &str) {
        if let Some(callback) = self.callback {
            callback(Progress::Started { output, activity });
        }
    }

    pub fn step(&self, report: &StepReport) {
        if let Some(callback) = self.callback {
            callback(Progress::Step(report));
        }
    }

    /// Record the complete output in the run log, then hand the block to the
    /// renderer. Returns whether a renderer took it.
    pub fn verified(&self, verification: &Verification) -> bool {
        tracing::info!(
            kind = verification.label.kind.describe(),
            name = verification.label.name.as_str(),
            owner = verification.label.owner.as_str(),
            passed = verification.passed,
            duration_ms = u64::try_from(verification.duration.as_millis()).unwrap_or(u64::MAX),
            detail = verification.detail.as_deref().unwrap_or(""),
            stdout = %String::from_utf8_lossy(&verification.stdout),
            stderr = %String::from_utf8_lossy(&verification.stderr),
            truncated = verification.truncated,
            "verification output"
        );
        match self.callback {
            Some(callback) => {
                callback(Progress::Verified(verification));
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::Span;

    fn exec(argv: Vec<WorkspaceArg>) -> WorkspaceCommand {
        WorkspaceCommand::Exec {
            span: Span {
                file: "gripsack.ts".into(),
                line: 1,
                col: None,
            },
            argv,
            env: Default::default(),
            cwd: None,
        }
    }

    #[test]
    fn commands_render_on_one_bounded_line() {
        let command = exec(vec![
            WorkspaceArg::PackageCommand {
                package: "ripgrep".into(),
                command: "rg".into(),
                sha256: None,
            },
            WorkspaceArg::Literal {
                value: "--version".into(),
            },
            WorkspaceArg::Literal {
                value: "two words".into(),
            },
            WorkspaceArg::Source {
                selector: String::new(),
            },
        ]);
        let label = VerificationLabel::for_command(VerificationKind::Check, "c", "p", &command);
        assert_eq!(label.command, "rg --version 'two words' <subject>");
        let long = exec(vec![WorkspaceArg::Literal {
            value: "x".repeat(200),
        }]);
        let label = VerificationLabel::for_command(VerificationKind::Hook, "h", "p", &long);
        assert_eq!(label.command.chars().count(), COMMAND_DISPLAY_CHARS + 1);
        assert!(label.command.ends_with('…'));
    }
}
