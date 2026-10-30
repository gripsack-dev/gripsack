use super::{eval_repo, validated_ir};
use crate::render::{DiagnosticSink, Palette};
use gripsack_exec::{BuildOptions, Ctx, ExecError, Repository};
use gripsack_process::{OperatorEnvironment, terminal::tame};
use std::{
    num::NonZeroU64,
    path::Path,
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug, clap::Args)]
pub struct BuildArgs {
    /// Named direct package outputs; no personal activation.
    #[arg(required = true)]
    pub outputs: Vec<String>,
    #[arg(long)]
    pub repo: Option<String>,
    /// Deadline for native package preparation.
    #[arg(long, default_value = "1800")]
    pub timeout_seconds: NonZeroU64,
    /// Emit retained output paths and commands as JSON.
    #[arg(long)]
    pub json: bool,
}

pub fn build(repo: &Path, args: BuildArgs, palette: Palette) -> ExitCode {
    let mut sink = if args.json {
        DiagnosticSink::json()
    } else {
        DiagnosticSink::terminal(palette, repo)
    };
    let result = build_inner(repo, args, &mut sink);
    gripsack_fetch::throttle::save_global();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => sink.finish_failure(code),
    }
}
fn build_inner(repo: &Path, args: BuildArgs, sink: &mut DiagnosticSink) -> Result<(), ExitCode> {
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(args.timeout_seconds.get()))
        .ok_or_else(|| {
            eprintln!("grip: build deadline is out of range");
            ExitCode::from(2)
        })?;
    let environment = OperatorEnvironment::capture().map_err(operational)?;
    let outcome = eval_repo(repo, None, sink)?;
    let ir = validated_ir(&outcome, sink)?;
    let progress_sources = Arc::clone(&outcome.sources);
    let ctx = Ctx {
        home: gripsack_store::gripsack_home(),
        home_dir: Default::default(),
        repository: Repository::evaluated(Arc::clone(&outcome.sources), outcome.receipt),
        only: args.outputs,
        host: outcome.host,
        take_over: false,
        take_over_entries: None,
        fetch: outcome.fetch,
        on_progress: Some(Box::new(move |name, event| {
            eprintln!(
                "{}: {}",
                tame(progress_sources.logical_text(name).into_owned()),
                tame(progress_sources.logical_text(event).into_owned())
            );
        })),
    };
    let options = BuildOptions {
        environment: &environment,
        deadline,
    };
    let result = gripsack_exec::build_workspace(&ir, &ctx, &options).map_err(|error| {
        match error {
            ExecError::Gate(diagnostic) => sink.report(&[diagnostic]),
            ExecError::Fetch(gripsack_fetch::FetchError::Diagnostics(diagnostics)) => {
                sink.report(&diagnostics)
            }
            ExecError::Fetch(error) => sink.report(&[gripsack_ir::Diagnostic::error(
                gripsack_ir::codes::EXEC_FETCH,
                error.to_string(),
            )]),
            other => sink.report(&[gripsack_ir::Diagnostic::error(
                gripsack_ir::codes::EXEC_STEP,
                other.to_string(),
            )]),
        }
        ExitCode::FAILURE
    })?;
    if args.json {
        println!("{}", serde_json::to_string(&result).map_err(operational)?);
    } else {
        for output in result.outputs {
            println!(
                "{} {}: {}",
                output.kind,
                tame(output.name),
                tame(output.path.display().to_string())
            );
            for (name, path) in output.commands {
                println!("  {}: {}", tame(name), tame(path.display().to_string()));
            }
        }
    }
    Ok(())
}
fn operational(error: impl std::fmt::Display) -> ExitCode {
    eprintln!("grip: {}", tame(error.to_string()));
    ExitCode::FAILURE
}
