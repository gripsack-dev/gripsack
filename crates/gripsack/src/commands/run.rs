//! `grip run` / `grip shell` / `grip task` — native project consumers over a
//! current workspace catalog. One shared path: evaluate, realize the selected closure
//! (cached/provider closures need no bridge), then launch through the common
//! consumer service. No personal generation is created and the live
//! checkout stays the working directory.
use super::{eval_repo, validated_ir};
use crate::render::{DiagnosticSink, Palette};
use gripsack_buildkit::worker::{MemoryBytes, WorkerOptions};
use gripsack_exec::{BuildOptions, ConsumerRequest, Ctx, ExecError, Repository, consume};
use gripsack_ir::workspace_model::identity::PackageDigest;
use gripsack_process::{OperatorEnvironment, Sha256Digest, terminal::tame};
use std::{
    ffi::OsString,
    num::{NonZeroU16, NonZeroU64},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug, clap::Args)]
pub struct RunArgs {
    /// Environment output providing the commands and variables.
    #[arg(long)]
    pub env: String,
    #[arg(long)]
    pub repo: Option<String>,
    /// Operator-selected bridge, needed only when the closure must be built.
    #[arg(long)]
    pub bridge: Option<PathBuf>,
    #[arg(long, default_value = "2")]
    pub builder_cpus: NonZeroU16,
    #[arg(long, default_value = "4096")]
    pub builder_memory_mib: NonZeroU64,
    /// Deadline for realization and launch admission; an interactive session
    /// itself is unbounded.
    #[arg(long, default_value = "1800")]
    pub timeout_seconds: NonZeroU64,
    /// The command and its exact arguments (preserve empty/spaced values).
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub argv: Vec<OsString>,
}

#[derive(Debug, clap::Args)]
pub struct ShellArgs {
    /// Environment output for the interactive shell.
    pub env: String,
    #[arg(long)]
    pub repo: Option<String>,
    /// Operator-selected bridge, needed only when the closure must be built.
    #[arg(long)]
    pub bridge: Option<PathBuf>,
    #[arg(long, default_value = "2")]
    pub builder_cpus: NonZeroU16,
    #[arg(long, default_value = "4096")]
    pub builder_memory_mib: NonZeroU64,
    #[arg(long, default_value = "1800")]
    pub timeout_seconds: NonZeroU64,
}

#[derive(Debug, clap::Args)]
pub struct TaskArgs {
    /// Task output to invoke once.
    pub task: String,
    #[arg(long)]
    pub repo: Option<String>,
    /// Operator-selected bridge, needed only when the closure must be built.
    #[arg(long)]
    pub bridge: Option<PathBuf>,
    #[arg(long, default_value = "2")]
    pub builder_cpus: NonZeroU16,
    #[arg(long, default_value = "4096")]
    pub builder_memory_mib: NonZeroU64,
    #[arg(long, default_value = "1800")]
    pub timeout_seconds: NonZeroU64,
}

/// Internal retained-wrapper protocol; never evaluates a repository.
#[derive(Debug, clap::Args)]
pub struct PackageCommandArgs {
    #[arg(long)]
    pub home: PathBuf,
    #[arg(long, value_parser = PackageDigest::parse)]
    pub package: PackageDigest,
    #[arg(long, value_parser = Sha256Digest::parse)]
    pub receipt: Sha256Digest,
    #[arg(long, allow_hyphen_values = true)]
    pub command: String,
    #[arg(long, value_parser = Sha256Digest::parse)]
    pub environment_sha256: Option<Sha256Digest>,
    #[arg(last = true, allow_hyphen_values = true)]
    pub arguments: Vec<OsString>,
}

pub fn package_command(args: PackageCommandArgs, palette: Palette) -> ExitCode {
    let sink = DiagnosticSink::terminal(palette, &args.home);
    let environment = match OperatorEnvironment::capture() {
        Ok(environment) => environment,
        Err(error) => {
            eprintln!("grip: {}", tame(error.to_string()));
            return ExitCode::FAILURE;
        }
    };
    let arguments: Vec<_> = args.arguments.iter().map(OsString::as_os_str).collect();
    match gripsack_exec::run_package_command(
        &args.home,
        args.package,
        args.receipt,
        &args.command,
        &arguments,
        args.environment_sha256,
        &environment,
    ) {
        Ok(outcome) => propagate(&outcome.receipt, outcome.root_retained),
        Err(error) => report_execution_error(error, sink),
    }
}

pub fn run(repo: &Path, args: RunArgs, palette: Palette) -> ExitCode {
    let request = ConsumerRequest::Run {
        environment: &args.env,
        argv: args.argv,
    };
    inner(
        repo,
        palette,
        request,
        args.bridge,
        args.builder_cpus,
        args.builder_memory_mib,
        args.timeout_seconds,
    )
}

pub fn shell(repo: &Path, args: ShellArgs, palette: Palette) -> ExitCode {
    // Shell startup policy: the operator's SHELL when set, else `sh` from
    // the operator PATH. The shell's own startup files run — host access is
    // not hermetic.
    let shell = std::env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("sh"));
    let request = ConsumerRequest::Shell {
        environment: &args.env,
        shell,
    };
    inner(
        repo,
        palette,
        request,
        args.bridge,
        args.builder_cpus,
        args.builder_memory_mib,
        args.timeout_seconds,
    )
}

pub fn task(repo: &Path, args: TaskArgs, palette: Palette) -> ExitCode {
    let request = ConsumerRequest::Task { task: &args.task };
    inner(
        repo,
        palette,
        request,
        args.bridge,
        args.builder_cpus,
        args.builder_memory_mib,
        args.timeout_seconds,
    )
}

fn inner(
    repo: &Path,
    palette: Palette,
    request: ConsumerRequest<'_>,
    bridge: Option<PathBuf>,
    cpus: NonZeroU16,
    memory_mib: NonZeroU64,
    timeout_seconds: NonZeroU64,
) -> ExitCode {
    let mut sink = DiagnosticSink::terminal(palette, repo);
    let deadline = match Instant::now().checked_add(Duration::from_secs(timeout_seconds.get())) {
        Some(deadline) => deadline,
        None => {
            eprintln!("grip: operation deadline is out of range");
            return ExitCode::from(2);
        }
    };
    let memory = match memory_mib
        .get()
        .checked_mul(1024 * 1024)
        .and_then(|bytes| MemoryBytes::new(bytes).ok())
    {
        Some(memory) => memory,
        None => {
            eprintln!("grip: builder memory is out of range");
            return ExitCode::from(2);
        }
    };
    let environment = match OperatorEnvironment::capture() {
        Ok(environment) => environment,
        Err(error) => {
            eprintln!("grip: {}", tame(error.to_string()));
            return ExitCode::FAILURE;
        }
    };
    let outcome = match eval_repo(repo, None, &mut sink) {
        Ok(outcome) => outcome,
        Err(code) => return code,
    };
    let ir = match validated_ir(&outcome, &mut sink) {
        Ok(ir) => ir,
        Err(code) => return code,
    };
    let only = vec![match &request {
        ConsumerRequest::Run { environment, .. } | ConsumerRequest::Shell { environment, .. } => {
            (*environment).to_owned()
        }
        ConsumerRequest::Task { task } => (*task).to_owned(),
    }];
    let ctx = Ctx {
        home: gripsack_store::gripsack_home(),
        home_dir: Default::default(),
        repository: Repository::evaluated(Arc::clone(&outcome.sources), outcome.receipt),
        only,
        host: outcome.host,
        take_over: false,
        take_over_entries: None,
        jobs: None,
        fetch: outcome.fetch,
        on_progress: None,
    };
    let options = BuildOptions {
        environment: &environment,
        bridge: bridge.as_deref(),
        worker: WorkerOptions {
            cpus,
            memory,
            ..WorkerOptions::default()
        },
        deadline,
    };
    match consume(&ir, &ctx, &options, &request) {
        Ok(outcome) => propagate(&outcome.receipt, outcome.root_retained),
        Err(error) => report_execution_error(error, sink),
    }
}

fn report_execution_error(error: ExecError, mut sink: DiagnosticSink) -> ExitCode {
    match error {
        ExecError::Gate(diagnostic) => sink.report(&[diagnostic]),
        ExecError::Fetch(gripsack_fetch::FetchError::Diagnostics(diagnostics)) => {
            sink.report(&diagnostics)
        }
        other => sink.report(&[gripsack_ir::Diagnostic::error(
            gripsack_ir::codes::EXEC_STEP,
            other.to_string(),
        )]),
    }
    sink.finish_failure(ExitCode::FAILURE)
}

/// Exit with the child's exact status: its exit code, or the same signal
/// re-raised so a waiting shell observes a signal death, not a number.
fn propagate(receipt: &gripsack_process::ProcessReceipt, root_retained: bool) -> ExitCode {
    if root_retained {
        eprintln!("grip: process cleanup was unconfirmed; the process root stays registered");
    }
    if let Some(code) = receipt.exit_code {
        return ExitCode::from(code as u8);
    }
    if let Some(signal) = receipt.signal {
        // SAFETY: restores the default disposition before re-raising, so the
        // process dies of the same signal as its child.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
        return ExitCode::from(128 + signal as u8);
    }
    if receipt.disposition == gripsack_process::ProcessDisposition::SpawnFailure {
        eprintln!(
            "grip: command failed to launch{}",
            receipt
                .error
                .as_ref()
                .map(|error| format!(": {error:?}"))
                .unwrap_or_default()
        );
    } else {
        eprintln!(
            "grip: command did not exit cleanly ({:?})",
            receipt.disposition
        );
    }
    ExitCode::FAILURE
}
