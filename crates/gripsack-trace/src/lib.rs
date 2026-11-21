//! Run tracing: every `grip` invocation gets a run id and a JSONL log
//! under `$GRIPSACK_HOME/runs/`, with the console layer alongside.
//!
//! ```text
//! $GRIPSACK_HOME/runs/
//! ├── <run-id>.jsonl   one file per run
//! └── latest -> <run-id>.jsonl
//! ```
//!
//! Causality is span ancestry (0004): an event's span chain —
//! `run → plan → module → step` — is how "a caused b" is read back,
//! from the console and from the JSONL alike. The debug skill for
//! agents lives in `skills/gripsack-debug/`.

mod retention;

use std::io;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

/// A started run. Keep the handle alive while the command can emit events.
#[derive(Debug)]
pub struct RunLog {
    pub id: String,
    pub path: PathBuf,
    session: retention::Session,
}

/// Set up console (compact, colored on tty) + JSONL file layers.
/// The console level comes from `GRIPSACK_LOG` (default `warn`).
pub fn init(home: &Path) -> io::Result<RunLog> {
    let (id, session) = retention::start(home)?;
    let run = RunLog {
        path: home.join("runs").join(format!("{id}.jsonl")),
        id,
        session,
    };

    // Console is warn-by-default (GRIPSACK_LOG raises it); the JSONL
    // always gets info and up — the log is the full record, the console
    // is for humans. The console writes to STDERR: stdout is data
    // (`grip check --json` emits exactly one document there), and a
    // core-side tracing::error! during eval must never prepend bytes
    // to it.
    let console_filter = tracing_subscriber::EnvFilter::try_from_env("GRIPSACK_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let console = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(io::stderr().is_terminal())
        .with_writer(io::stderr)
        .with_filter(console_filter);
    let json = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .with_writer(FileWriter(Arc::clone(&run.session.file)))
        .with_filter(tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(console)
        .with(json)
        .try_init()
        .map_err(io::Error::other)?;
    tracing::info!(target: "gripsack_trace", run_id = %run.id, "run log opened");
    Ok(run)
}

/// Root span for a command run — every event hangs off this ancestry.
#[macro_export]
macro_rules! run_span {
    ($run:expr, $command:expr) => {
        tracing::info_span!("run", run_id = %$run.id, command = %$command)
    };
}

/// Shares one file across the json layer's per-event writers.
struct FileWriter(Arc<std::fs::File>);

impl<'a> MakeWriter<'a> for FileWriter {
    type Writer = FileGuard;
    fn make_writer(&'a self) -> Self::Writer {
        FileGuard(Arc::clone(&self.0))
    }
}

/// `&File` implements `Write` — the guard just derefs.
struct FileGuard(Arc<std::fs::File>);

impl io::Write for FileGuard {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self.0).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        (&*self.0).flush()
    }
}
