use crate::model::{Action, Build, Entry, FetchSpec, Verify};
use crate::span::Span;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Step ids every declarative module gets after expansion (0007 §2) —
/// the valid cross-module targets (`rust:install`) when the target
/// module does not declare explicit steps.
pub const SYNTHESIZED_STEP_IDS: [&str; 6] =
    ["fetch", "build", "install", "config", "activate", "done"];

/// The reserved barrier step id (0007 §2); explicit modules may not
/// claim it.
pub const BARRIER_STEP_ID: &str = "done";

/// Resources the core knows how to serialize/throttle (0007 §4).
/// Anything else warns (W201) — an open namespace.
pub const KNOWN_RESOURCES: [&str; 3] = ["network", "pixi-lock", "cargo-lock"];

/// A step: one node in a module's execution DAG (0007 §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub action: StepAction,
    /// Sibling step ids, or cross-module `module:step` refs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// Named resources to acquire before running (0007 §4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<String>,
    /// Reporting tag — never a scheduling barrier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    /// Smoke contract run right after the action; failure = step failed
    /// (0007 §verify). Mandatory in spirit for custom_shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<Verify>,
    /// Retry count override (0007 §retries). Default: engine policy —
    /// retries only for fetch actions, 0 otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepAction {
    Fetch {
        fetch: FetchSpec,
    },
    Build {
        spec: Build,
    },
    Install {
        entries: Vec<Entry>,
    },
    ConfigDeploy {
        entries: Vec<Entry>,
    },
    Intent {
        action: Box<Action>,
    },
    /// A module-level smoke contract as a terminal step (synthesized by
    /// the expansion pass; 0007 §verify).
    Verify {
        verify: Verify,
    },
    /// A structured action (0007 §3): argv/env/cwd as data, no shell
    /// interpretation. Declared `outputs` make it satisfiable (0008 §4).
    Run {
        argv: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        outputs: Vec<String>,
    },
    /// The last rung: declared, flagged. Declared `outputs` restore
    /// caching; without them the step always runs (0008 §4).
    CustomShell {
        script: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        outputs: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Fetch,
    Build,
    Install,
    Config,
    Verify,
    Activate,
    Custom,
}
