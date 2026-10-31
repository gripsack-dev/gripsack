//! Explicit container startup versus source-independent owned-state inspection.
use super::{eval_repo, validated_ir};
use crate::render::{DiagnosticSink, Palette};
use gripsack_exec::{Ctx, Repository, services};
use gripsack_process::terminal::tame;
use std::{
    num::NonZeroU64,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug, clap::Args)]
pub struct UpArgs {
    /// Local workspace repository; startup evaluates only approved captured bytes.
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
    /// Explicit local Docker Unix socket; recorded for later status/down.
    #[arg(long)]
    pub socket: Option<PathBuf>,
    #[arg(long, default_value = "180")]
    pub timeout_seconds: NonZeroU64,
    #[arg(long)]
    pub json: bool,
    /// Selected services; dependencies become healthy first. Default: all.
    pub services: Vec<String>,
}
#[derive(Debug, clap::Args)]
pub struct DownArgs {
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
    #[arg(long, default_value = "60")]
    pub timeout_seconds: NonZeroU64,
    #[arg(long)]
    pub json: bool,
    /// Owned containers to stop/remove; named volumes are always retained.
    pub services: Vec<String>,
}
#[derive(Debug, clap::Args)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub command: ServiceCommand,
}
#[derive(Debug, clap::Subcommand)]
pub enum ServiceCommand {
    /// Inspect owned engine state without evaluating or starting anything.
    Status {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Read bounded application logs. Application output may contain secrets.
    Logs {
        service: String,
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long, default_value = "100", value_parser = clap::value_parser!(u16).range(1..=1000))]
        lines: u16,
    },
}
fn deadline(seconds: u64) -> Result<Instant, String> {
    Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| "service deadline is out of range".to_owned())
}
fn recorded_context(repo: &Path, only: Vec<String>) -> Result<Ctx, String> {
    Ok(Ctx {
        home: gripsack_store::gripsack_home(),
        home_dir: Default::default(),
        repository: Repository::direct(
            std::path::absolute(repo).map_err(|error| error.to_string())?,
        ),
        only,
        host: gripsack_ir::HostName::parse(super::default_host())
            .map_err(|error| error.to_string())?,
        take_over: false,
        take_over_entries: None,
        fetch: Arc::new(gripsack_fetch::FetchContext::default()),
        on_progress: None,
    })
}
fn failed(error: impl std::fmt::Display) -> ExitCode {
    eprintln!("grip: {}", tame(error.to_string()));
    ExitCode::FAILURE
}
fn display(
    result: Result<Vec<services::ServiceStatus>, services::ServiceError>,
    json: bool,
) -> ExitCode {
    match result {
        Err(error) => failed(error),
        Ok(records) => {
            if json {
                match serde_json::to_string(&records) {
                    Ok(text) => println!("{text}"),
                    Err(error) => return failed(error),
                }
            } else if records.is_empty() {
                println!("No recorded services.");
            } else {
                for record in records {
                    println!("{}\t{}", tame(record.name), tame(record.state));
                }
            }
            ExitCode::SUCCESS
        }
    }
}
pub fn up(args: UpArgs, palette: Palette) -> ExitCode {
    let deadline = match deadline(args.timeout_seconds.get()) {
        Ok(value) => value,
        Err(error) => return failed(error),
    };
    let mut sink = DiagnosticSink::terminal(palette, &args.repo);
    let outcome = match eval_repo(&args.repo, None, &mut sink) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let ir = match validated_ir(&outcome, &mut sink) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let ctx = Ctx {
        home: gripsack_store::gripsack_home(),
        home_dir: Default::default(),
        repository: Repository::evaluated(Arc::clone(&outcome.sources), outcome.receipt),
        only: args.services,
        host: outcome.host,
        take_over: false,
        take_over_entries: None,
        fetch: outcome.fetch,
        on_progress: None,
    };
    display(
        services::up(&ir, &ctx, args.socket.as_deref(), deadline),
        args.json,
    )
}
pub fn down(args: DownArgs) -> ExitCode {
    let ctx = match recorded_context(&args.repo, args.services) {
        Ok(value) => value,
        Err(error) => return failed(error),
    };
    let deadline = match deadline(args.timeout_seconds.get()) {
        Ok(value) => value,
        Err(error) => return failed(error),
    };
    display(services::down(&ctx, deadline), args.json)
}
pub fn services(args: ServiceArgs) -> ExitCode {
    match args.command {
        ServiceCommand::Status { repo, json } => {
            let ctx = match recorded_context(&repo, Vec::new()) {
                Ok(value) => value,
                Err(error) => return failed(error),
            };
            display(
                services::status(&ctx, Instant::now() + Duration::from_secs(30)),
                json,
            )
        }
        ServiceCommand::Logs {
            service,
            repo,
            lines,
        } => {
            let ctx = match recorded_context(&repo, Vec::new()) {
                Ok(value) => value,
                Err(error) => return failed(error),
            };
            match services::logs(
                &ctx,
                &service,
                lines,
                Instant::now() + Duration::from_secs(30),
            ) {
                Ok(bytes) => {
                    print!("{}", tame(String::from_utf8_lossy(&bytes).into_owned()));
                    ExitCode::SUCCESS
                }
                Err(error) => failed(error),
            }
        }
    }
}
