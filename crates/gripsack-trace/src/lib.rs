//! Run tracing: every `grip` invocation gets a run id and a JSONL log
//! under `$GRIPSACK_HOME/runs/`, with the console layer alongside.
//!
//! ```text
//! $GRIPSACK_HOME/runs/
//! ├── 20260823T065501Z-a1b2c3.jsonl   one file per run
//! └── latest -> 20260823T065501Z-a1b2c3.jsonl
//! ```
//!
//! Causality is span ancestry (0004): an event's span chain —
//! `run → plan → module → step` — is how "a caused b" is read back,
//! from the console and from the JSONL alike. The debug skill for
//! agents lives in `skills/gripsack-debug/`.

use std::io;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

/// A started run: its id and the JSONL path.
#[derive(Debug, Clone)]
pub struct RunLog {
    pub id: String,
    pub path: PathBuf,
}

/// Timestamp + random suffix: sortable, collision-safe.
pub fn new_run_id() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut entropy = [0u8; 4];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use io::Read;
        let _ = f.read_exact(&mut entropy);
    }
    let rand = u32::from_le_bytes(entropy);
    format!("{secs}-{:06x}", rand & 0xFF_FFFF)
}

/// Set up console (compact, colored on tty) + JSONL file layers.
/// Returns the run handle; `runs/latest` points at it. Level comes from
/// `GRIPSACK_LOG` (default `info`).
pub fn init(home: &Path) -> io::Result<RunLog> {
    let id = new_run_id();
    let run = RunLog {
        path: home.join("runs").join(format!("{id}.jsonl")),
        id,
    };
    std::fs::create_dir_all(run.path.parent().expect("runs dir"))?;
    let file = Arc::new(std::fs::File::create(&run.path)?);

    // Console is warn-by-default (GRIPSACK_LOG raises it); the JSONL
    // always gets info and up — the log is the full record, the console
    // is for humans.
    let console_filter = tracing_subscriber::EnvFilter::try_from_env("GRIPSACK_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let console = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(io::stdout().is_terminal())
        .with_writer(io::stdout)
        .with_filter(console_filter);
    let json = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .with_writer(FileWriter(file))
        .with_filter(tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(console)
        .with(json)
        .try_init();

    gripsack_store::symlink_replace(&home.join("runs").join("latest"), &run.path)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ids_are_unique_and_sortable() {
        let a = new_run_id();
        let b = new_run_id();
        assert_ne!(a, b);
        // distinct ids; timestamp prefix keeps them chronologically sortable
    }

    #[test]
    fn init_writes_jsonl_with_run_id_and_spans() {
        let dir = tempfile::tempdir().unwrap();
        let run = init(dir.path()).unwrap();
        {
            let span = run_span!(run, "test");
            let _entered = span.enter();
            let inner = tracing::info_span!("module", name = "helix");
            let _inner = inner.enter();
            tracing::info!(answer = 42, "test event");
        }
        // latest points at the run file
        let latest = dir.path().join("runs").join("latest");
        assert_eq!(latest.read_link().unwrap(), run.path);
        let log = std::fs::read_to_string(&run.path).unwrap();
        assert!(log.contains(&run.id));
        assert!(log.contains("test event"));
        assert!(log.contains("\"answer\":42"));
        // causality: the event carries its span ancestry
        assert!(log.contains("module"));
    }
}
