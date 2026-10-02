//! Inspect durable hook identity/outcomes without replay or repository eval.
mod fixtures;
use crate::render::Palette;
use gripsack_store as store;
use serde::{
    Serialize, Serializer,
    ser::{Error, SerializeSeq, SerializeStruct},
};
use std::{
    io::{self, Write},
    process::ExitCode,
};

#[derive(Debug, clap::Subcommand)]
pub enum HooksCommand {
    /// Inspect pending, ambiguous and archived intent outcomes without running hooks
    List {
        #[arg(long)]
        json: bool,
    },
    /// Simulate harmless first-party hooks in throwaway HOME/state only
    Test {
        #[arg(long, conflicts_with = "crash_after_start")]
        duplicate: bool,
        #[arg(long)]
        crash_after_start: bool,
    },
    #[command(hide = true)]
    FixtureWorker {
        #[arg(long)]
        root: std::path::PathBuf,
        #[arg(long, value_enum)]
        case: fixtures::FixtureCase,
        #[arg(long, value_enum)]
        mode: fixtures::SimulationMode,
    },
    #[command(hide = true)]
    FixtureEffect {
        #[arg(long)]
        root: std::path::PathBuf,
        #[arg(long, value_enum)]
        case: fixtures::FixtureCase,
    },
}

impl HooksCommand {
    pub fn fixture_only(&self) -> bool {
        !matches!(self, Self::List { .. })
    }
}

pub fn hooks(command: HooksCommand, palette: Palette) -> ExitCode {
    let result = match command {
        HooksCommand::List { json } => list(json),
        HooksCommand::Test {
            duplicate,
            crash_after_start,
        } => {
            let mode = if duplicate {
                fixtures::SimulationMode::Duplicate
            } else if crash_after_start {
                fixtures::SimulationMode::CrashAfterStart
            } else {
                fixtures::SimulationMode::Clean
            };
            fixtures::simulate(mode).and_then(|report| {
                let mut output = io::stdout().lock();
                serde_json::to_writer_pretty(&mut output, &report).map_err(io::Error::other)?;
                writeln!(output)
            })
        }
        HooksCommand::FixtureWorker { root, case, mode } => fixtures::worker(&root, case, mode),
        HooksCommand::FixtureEffect { root, case } => fixtures::effect(&root, case),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{}",
                palette.error(&gripsack_process::terminal::tame(format!(
                    "hook command failed: {error}"
                )))
            );
            ExitCode::FAILURE
        }
    }
}

fn list(json: bool) -> io::Result<()> {
    let home = match gripsack_fs::open(&store::gripsack_home()) {
        Ok(home) => Some(home),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut output = io::stdout().lock();
    if json {
        serde_json::to_writer(&mut output, &Snapshot(home.as_ref())).map_err(io::Error::other)?;
        writeln!(output)
    } else if let Some(home) = &home {
        store::activation::inspect(home, |row| {
            let identity: &dyn std::fmt::Display = match row.intent.as_ref() {
                Some(intent) => intent,
                None => &"legacy-unidentified",
            };
            write!(
                output,
                "{identity}  generation {}  {}",
                row.generation, row.state
            )?;
            if row.pending {
                write!(output, "  [pending record]")?;
            }
            writeln!(output)
        })
    } else {
        writeln!(output, "no recorded hook outcomes")
    }
}

struct Snapshot<'a>(Option<&'a gripsack_fs::Dir>);
impl Serialize for Snapshot<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut snapshot = serializer.serialize_struct("Hooks", 4)?;
        snapshot.serialize_field("version", &1_u32)?;
        snapshot.serialize_field("delivery", "at_least_once")?;
        snapshot.serialize_field("failure_policy", "warn_no_retry")?;
        snapshot.serialize_field("intents", &Rows(self.0))?;
        snapshot.end()
    }
}
struct Rows<'a>(Option<&'a gripsack_fs::Dir>);
impl Serialize for Rows<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut rows = serializer.serialize_seq(None)?;
        if let Some(home) = self.0 {
            store::activation::inspect(home, |row| {
                rows.serialize_element(&row)
                    .map_err(|error| io::Error::other(error.to_string()))
            })
            .map_err(S::Error::custom)?;
        }
        rows.end()
    }
}
