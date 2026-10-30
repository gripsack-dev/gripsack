//! Repository-free scheduling inspection and exact captured invocation dispatch.
use gripsack_exec::{
    LifecycleSession,
    schedules::{self, JobId, JobRevision},
};
use gripsack_process::terminal::{display, write_output};
use gripsack_store::{self as store, activation::ActivationAction};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Debug, clap::Args)]
pub struct JobsArgs {
    /// Explicit store home, captured in the owned unit argv.
    #[arg(long, global = true)]
    pub home: Option<PathBuf>,
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: JobsCommand,
}
#[derive(Debug, clap::Subcommand)]
pub enum JobsCommand {
    /// Inspect the user manager without installing, enabling, or fetching anything.
    Capability,
    /// Show selected revisions, actual manager observations and invocation history.
    Status { job: Option<String> },
    /// Read an exact scheduled invocation's bounded private log.
    Log { invocation: String },
    /// Disable an owned job via an ordinary journaled profile generation.
    Disable { job: String },
    /// Invoke the exact active captured revision; used by the user timer.
    #[command(hide = true)]
    Dispatch { job: String, revision: String },
}

pub fn jobs(args: JobsArgs) -> ExitCode {
    match execute(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("grip: {}", display(&error.to_string()));
            ExitCode::FAILURE
        }
    }
}
fn execute(args: JobsArgs) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let home = args.home.unwrap_or_else(store::gripsack_home);
    if !home.is_absolute() {
        return Err("jobs --home must be absolute".into());
    }
    match args.command {
        JobsCommand::Capability => emit(&schedules::systemd::inspect()?, args.json)?,
        JobsCommand::Status { job } => {
            let selected = job.as_deref().map(JobId::parse).transpose()?;
            emit(&status(&home, selected.as_ref())?, args.json)?;
        }
        JobsCommand::Log { invocation } => {
            let id = gripsack_exec::InvocationId::parse(&invocation)?;
            let report = gripsack_exec::read_task_report(&home, &id)?;
            if report.schedule.is_none() {
                return Err("invocation was not a scheduled job".into());
            }
            let bytes = gripsack_exec::read_task_log(&home, &id)?;
            if args.json {
                emit(
                    &serde_json::json!({"invocation":id,"schedule":report.schedule,"log":String::from_utf8_lossy(&bytes),"truncated":report.logs_truncated}),
                    true,
                )?;
            } else {
                write_output(&mut std::io::stdout().lock(), &bytes)?;
            }
        }
        JobsCommand::Disable { job } => {
            let id = JobId::parse(&job)?;
            let session = LifecycleSession::acquire(&home)?;
            let notes = schedules::disable(&session, &id)?;
            emit(
                &serde_json::json!({"job":id,"selection":"disabled","recovery_notes":notes.len(),"status":status(&home,Some(&id))?}),
                args.json,
            )?;
        }
        JobsCommand::Dispatch { job, revision } => {
            let report =
                schedules::dispatch(&home, &JobId::parse(&job)?, &JobRevision::parse(&revision)?)?;
            let code = report.status.exit_code();
            emit(&report, args.json)?;
            return Ok(ExitCode::from(code));
        }
    }
    Ok(ExitCode::SUCCESS)
}
fn emit(value: &impl serde::Serialize, json: bool) -> Result<(), serde_json::Error> {
    if json {
        println!("{}", serde_json::to_string(value)?);
    } else {
        println!("{}", display(&serde_json::to_string_pretty(value)?));
    }
    Ok(())
}
fn status(
    home: &Path,
    selected: Option<&JobId>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let generation = store::current_generation(home)?;
    let mut jobs = Vec::new();
    if let Some(number) = generation {
        let manifest = store::generations::admit_manifest(home, number)?;
        for (owner, state) in &manifest.deployments {
            for intent in &state.intents {
                let ActivationAction::ScheduledJob {
                    job,
                    revision,
                    capture,
                    unit_directory,
                    enabled,
                    ..
                } = &intent.action
                else {
                    continue;
                };
                if selected.is_some_and(|selected| selected != job) {
                    continue;
                }
                jobs.push(serde_json::json!({
                    "job":job,"revision":revision,"profile":owner,"enabled_selection":enabled,
                    "capture":capture,"persistent":false,"missed":"native","scope":"user",
                    "manager":schedules::systemd::status(job,unit_directory)?,
                }));
            }
        }
    }
    let mut activation = Vec::new();
    let mut invocations = Vec::new();
    if home.exists() {
        let directory = gripsack_fs::open(home)?;
        store::activation::inspect(&directory, |entry| {
            let value = serde_json::to_value(entry).map_err(std::io::Error::other)?;
            activation.push(value);
            Ok(())
        })?;
        match gripsack_fs::open_dir_nofollow(&directory, Path::new("task-invocations")) {
            Ok(history) => {
                for (count, entry) in history.entries()?.enumerate() {
                    if count >= 10000 {
                        return Err("task history exceeds inspection entry budget".into());
                    }
                    let entry = entry?;
                    let name = entry.file_name();
                    let Some(name) = name.to_str().and_then(|name| name.strip_suffix(".json"))
                    else {
                        continue;
                    };
                    let id = gripsack_exec::InvocationId::parse(name)?;
                    let report = gripsack_exec::read_task_report(home, &id)?;
                    if let Some(schedule) = &report.schedule
                        && selected.is_none_or(|selected| selected == &schedule.job)
                    {
                        invocations.push(serde_json::json!({"invocation":id,"schedule":schedule,"status":report.status,"root_retained":report.root_retained}));
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let registration: Vec<_> = schedules::inspect(home)?
        .into_iter()
        .filter(|entry| selected.is_none_or(|selected| selected == &entry.job))
        .collect();
    Ok(
        serde_json::json!({"generation":generation.map(gripsack_store::GenerationId::value),"jobs":jobs,"registration":registration,"activation":activation,"invocations":invocations}),
    )
}
