//! Executor context, outcomes, and errors.

use gripsack_fetch::FetchError;
use gripsack_ir::HostName;
use std::io;
use std::path::PathBuf;

/// Thread-safe workspace output progress events.
pub type ProgressCallback = Box<dyn Fn(&str, &str) + Send + Sync>;

/// What an apply run needs beyond the IR.
pub struct Ctx {
    /// $GRIPSACK_HOME.
    pub home: PathBuf,
    /// Repository publication identity and selected content are separate roles.
    pub repository: crate::Repository,
    /// Selected workspace outputs. Empty selects the operation's default outputs.
    pub only: Vec<String>,
    /// Host identity used for workspace lock and profile selection.
    pub host: HostName,
    /// Progress events `(output, verb)` — the CLI renders spinners.
    pub on_progress: Option<ProgressCallback>,
    /// Overwrite foreign/drifted tracked_copy destinations (explicit
    /// user intent — 0009 critique finding 3).
    pub take_over: bool,
    /// Scoped take-over (0015 §3): only these destinations may be
    /// absorbed — `grip adopt` passes exactly what it generated, so an
    /// adopt apply can never clobber unrelated drift.
    pub take_over_entries: Option<std::collections::BTreeSet<String>>,

    /// Command-owned acquisition policy, clients and independent concurrency.
    pub fetch: std::sync::Arc<gripsack_fetch::FetchContext>,

    /// The home capability (plan/0021), opened lazily on first use:
    /// read-only commands (plan, check) never materialize
    /// `$GRIPSACK_HOME`; mutation paths share ONE pinned root inode.
    pub home_dir: std::sync::OnceLock<gripsack_fs::Dir>,
}

impl Ctx {
    /// May this destination be taken over? Global flag or scoped set.
    pub fn takes_over(&self, to: &str) -> bool {
        self.take_over
            || self
                .take_over_entries
                .as_ref()
                .is_some_and(|entries| entries.contains(to))
    }

    /// The home capability, opened (and created) on first mutation.
    /// A concurrent second opener loses to `get_or_init` — both fds
    /// name the same directory, so the race is harmless.
    pub fn home_dir(&self) -> io::Result<&gripsack_fs::Dir> {
        if let Some(dir) = self.home_dir.get() {
            return Ok(dir);
        }
        let dir = gripsack_fs::open_or_create(&self.home)?;
        Ok(self.home_dir.get_or_init(|| dir))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Nothing changed; no generation created.
    Satisfied {
        generation: Option<gripsack_store::GenerationId>,
    },
    /// A new generation was deployed and activated.
    Applied {
        generation: gripsack_store::GenerationId,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("fetch failed: {0}")]
    Fetch(#[from] FetchError),
    #[error("step resolve failed in {output}: {error}")]
    Resolve {
        output: String,
        #[source]
        error: Box<gripsack_fetch::resolve::ResolveError>,
    },
    #[error("step {step} failed in {output}: {detail}")]
    Step {
        output: String,
        step: String,
        detail: String,
    },
    #[error("planning: {0}")]
    Plan(#[from] crate::PlanError),
    /// A pre-mutation validity gate failed (physical destination
    /// uniqueness, 0030 §P0-1): the run aborts before touching
    /// anything, with a span-labeled diagnostic.
    #[error("{0}")]
    Gate(gripsack_ir::diagnostic::Diagnostic),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
