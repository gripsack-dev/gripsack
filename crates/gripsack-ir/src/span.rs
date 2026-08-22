use serde::{Deserialize, Serialize};
use std::fmt;

/// Source location of an IR node in the user's frontend code (0004 §2).
/// Payload: threaded through passes, never recomputed, never part of
/// store-path identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub file: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.col {
            Some(col) => write!(f, "{}:{}:{}", self.file, self.line, col),
            None => write!(f, "{}:{}", self.file, self.line),
        }
    }
}
