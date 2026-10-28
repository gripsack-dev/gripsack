//! Ownership destinations and local calendar values are independent of content.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceDestination {
    Symlink { path: String },
    TrackedCopy { path: String },
    ManagedBlock { path: String, marker: String },
}

/// Daily or weekly local time. Registration is a separate execution capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceCalendar {
    Daily { time: String },
    Weekly { weekday: Weekday, time: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}
