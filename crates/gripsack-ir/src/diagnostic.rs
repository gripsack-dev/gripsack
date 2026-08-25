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
