use crate::span::Span;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;

/// Stable diagnostic codes (0004 §3). Match on codes, never on text.
pub mod codes {
    pub const MALFORMED: &str = "E000";
    pub const VERSION: &str = "E100";
    pub const UNKNOWN_DEPENDENCY: &str = "E101";
    pub const BAD_DESTINATION: &str = "E102";
    pub const STEPS_WITH_FIELDS: &str = "E103";
    pub const UNKNOWN_STEP: &str = "E104";
    pub const DUPLICATE_STEP: &str = "E106";
    pub const UNKNOWN_RESOURCE: &str = "E107";
    pub const CONFIG: &str = "E400";
    pub const UNSUPPORTED_MODE: &str = "E108";
    pub const VERIFY_PATH_SHAPE: &str = "E109";
    pub const MISSING_SOURCE: &str = "E110";
    pub const DUPLICATE_DESTINATION: &str = "E111";
    /// Probe binding never reached a fixpoint (0013 D6).
    pub const PROBE_UNSTABLE: &str = "E112";
    /// The frontend requested a probe kind this grip cannot answer.
    pub const PROBE_UNSUPPORTED: &str = "E113";
    /// Fetch/resolution failed at apply (registry, network, hash drift).
    pub const EXEC_FETCH: &str = "E201";
    /// An execution step failed (build, deploy, install).
    pub const EXEC_STEP: &str = "E301";
    /// A module's verify contract failed.
    pub const EXEC_VERIFY: &str = "E302";
    /// Unknown `{placeholder}` in a fetch/install/verify string (0016 §D1).
    pub const UNKNOWN_PLACEHOLDER: &str = "E114";
    /// A source or destination path with an illegal shape (0016 §D4).
    pub const BAD_PATH: &str = "E115";
    /// Module names that would escape their store segment or break refs.
    pub const INVALID_MODULE_NAME: &str = "E116";
    /// Env var name that is not a shell identifier.
    pub const INVALID_ENV_NAME: &str = "E117";
    /// Steps modules with more than one fetch step cannot be pinned.
    pub const UNPINNABLE_STEPS: &str = "E118";
    /// A step's `needs` references a LATER phase — post-deploy effects
    /// belong in activate hooks (0035 F8).
    pub const STEP_PHASE_ORDER: &str = "E121";
    /// A cycle in a module's step `needs` graph (0033 R4).
    pub const STEP_CYCLE: &str = "E120";
    /// Two declarations resolve to one physical destination (aliases:
    /// `~` vs `$HOME` vs absolute, symlinked ancestors) — 0030 §P0-1.
    pub const DESTINATION_ALIAS: &str = "E119";
    /// A dependency edge kind other than `runtime`/`build` (0039).
    pub const UNKNOWN_EDGE: &str = "E122";
    /// Two build dependencies normalize to the same GRIP_DEP_* identifier.
    pub const BUILD_DEP_ENV_COLLISION: &str = "E123";
    /// A declared workspace capability has no available executor on this
    /// build (isolated_linux before B2, schedule registration before E3,
    /// …). Emitted by the CLI plan/apply lanes before any effect — never
    /// a silent host fallback (plan/0052 §3.2).
    pub const WORKSPACE_EXEC_UNAVAILABLE: &str = "E124";
    /// Two workspace outputs declare the same catalog name (0052 §2.1).
    pub const DUPLICATE_WORKSPACE_OUTPUT: &str = "E125";
    /// A workspace reference names no output/command, names the wrong
    /// kind, or binds incompatible target/layout declarations (§2.2).
    pub const UNKNOWN_WORKSPACE_REF: &str = "E126";
    /// A cycle in the workspace production/build/runtime/task graph.
    pub const WORKSPACE_CYCLE: &str = "E127";
    /// A typed command reference in a position that cannot run it:
    /// package_command as an environment value, or a run_bash
    /// interpreter that is not a pinned package_command (0052 §2.2).
    pub const BAD_WORKSPACE_CONTEXT: &str = "E128";
    /// A workspace span with an empty file or a line/column below 1 —
    /// workspace provenance is mandatory and well-formed.
    pub const BAD_WORKSPACE_SPAN: &str = "E129";
    /// A workspace value outside the admitted grammar: empty output
    /// catalog/name, malformed calendar time, file origin/content
    /// mismatch, unsafe artifact selector or Bash body interpolation.
    pub const INVALID_WORKSPACE_VALUE: &str = "E130";
    /// A required workspace producer or validation edge vanished from
    /// the admitted graph projection. Fail closed, never publish from
    /// an incomplete graph (0052 §5.1).
    pub const REQUIRED_WORKSPACE_EDGE_MISSING: &str = "E131";
}

// ---------------------------------------------------------------- diagnostics

/// Compiler-style diagnostics (0004 §3): structured, span-labeled,
/// collected across passes. Rendered by the CLI; matched on `code` by
/// tooling and the future LSP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable code (`E101`); plugin codes are namespaced runtime
    /// strings (0009 §2), hence the Cow.
    pub code: Cow<'static, str>,
    pub severity: Severity,
    pub message: String,
    pub labels: Vec<Label>,
    pub help: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    /// None when the node has no span — the message carries context then.
    pub span: Option<Span>,
    pub note: String,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Diagnostic {
            code: Cow::Borrowed(code),
            severity: Severity::Error,
            message: message.into(),
            labels: Vec::new(),
            help: None,
        }
    }

    pub fn with_label(mut self, span: Option<Span>, note: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            note: note.into(),
        });
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]: {}", self.severity, self.code, self.message)?;
        for label in &self.labels {
            match &label.span {
                Some(span) => write!(f, "\n  --> {span}")?,
                None => write!(f, "\n  = {}", label.note)?,
            }
            if !label.note.is_empty() && label.span.is_some() {
                write!(f, " — {}", label.note)?;
            }
        }
        if let Some(help) = &self.help {
            write!(f, "\n  help: {help}")?;
        }
        Ok(())
    }
}
