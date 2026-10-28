//! Read-only inspection and bounded manual invocation share the admitted graph.
use super::*;
use gripsack_exec::{
    BusyPolicy, InvocationId, TaskOutputMode, TaskRunPolicy, TaskStatus, TaskStdin,
};
use gripsack_process::terminal::{display, write_output};

#[derive(Debug, clap::Args)]
pub struct TaskArgs {
    #[arg(long, global = true)]
    pub repo: Option<String>,
    #[arg(long, global = true)]
    pub bridge: Option<PathBuf>,
    #[arg(long, global = true, default_value = "2")]
    pub builder_cpus: NonZeroU16,
    #[arg(long, global = true, default_value = "4096")]
    pub builder_memory_mib: NonZeroU64,
    /// One total deadline, including realization, prerequisites and verification.
    #[arg(long, global = true, default_value = "1800")]
    pub timeout_seconds: NonZeroU64,
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: TaskCommand,
}
#[derive(Debug, clap::Subcommand)]
pub enum TaskCommand {
    /// List inert task declarations; no realization or postconditions.
    List,
    /// Inspect the finite graph and complete mutation-key set without effects.
    Plan { name: String },
    /// Execute one new invocation; previous task success is never cached.
    Run {
        name: String,
        #[arg(long)]
        skip_busy: bool,
        #[arg(long)]
        closed_stdin: bool,
    },
    /// Read an exact durable invocation receipt; interrupted outcomes stay unknown.
    Status { invocation: String },
    /// Read the private bounded stdout/stderr log for an invocation.
    Log { invocation: String },
}

impl TaskArgs {
    /// Inspection must not invoke the generic remote clone/pull resolver.
    pub fn resolve_repository(&mut self) -> Result<PathBuf, ExitCode> {
        let specification = self.repo.take();
        if matches!(self.command, TaskCommand::Run { .. }) {
            return super::super::resolve_repo(specification.as_deref());
        }
        match specification {
            Some(path) if Path::new(&path).is_dir() => Ok(PathBuf::from(path)),
            Some(_) => {
                eprintln!(
                    "grip: task inspection accepts only an existing local --repo; it never clones or updates a checkout"
                );
                Err(ExitCode::from(2))
            }
            None => std::env::current_dir().map_err(|error| {
                eprintln!("grip: {}", tame(error.to_string()));
                ExitCode::FAILURE
            }),
        }
    }
}

pub fn task(repo: &Path, args: TaskArgs, palette: Palette) -> ExitCode {
    match &args.command {
        TaskCommand::Run {
            name,
            skip_busy,
            closed_stdin,
        } => inner(
            repo,
            palette,
            NativeRequest::Task {
                name,
                policy: TaskRunPolicy {
                    on_busy: if *skip_busy {
                        BusyPolicy::Skip
                    } else {
                        BusyPolicy::Fail
                    },
                    stdin: if *closed_stdin {
                        TaskStdin::Closed
                    } else {
                        TaskStdin::Inherit
                    },
                    output: if args.json {
                        TaskOutputMode::Capture
                    } else {
                        TaskOutputMode::Stream
                    },
                },
                json: args.json,
            },
            args.bridge,
            args.builder_cpus,
            args.builder_memory_mib,
            args.timeout_seconds,
        ),
        TaskCommand::Status { invocation } | TaskCommand::Log { invocation } => {
            let home = gripsack_store::gripsack_home();
            let id = match InvocationId::parse(invocation) {
                Ok(id) => id,
                Err(error) => {
                    eprintln!("grip: {}", tame(error.to_string()));
                    return ExitCode::from(2);
                }
            };
            let report = match gripsack_exec::read_task_report(&home, &id) {
                Ok(report) => report,
                Err(error) => {
                    return report_execution_error(error, DiagnosticSink::terminal(palette, repo));
                }
            };
            if matches!(args.command, TaskCommand::Log { .. }) {
                match gripsack_exec::read_task_log(&home, &id) {
                    Ok(bytes) => {
                        if args.json {
                            println!(
                                "{}",
                                serde_json::json!({"invocation":id,"log":String::from_utf8_lossy(&bytes),"truncated":report.logs_truncated})
                            );
                        } else if let Err(error) =
                            write_output(&mut std::io::stdout().lock(), &bytes)
                        {
                            eprintln!("grip: {}", tame(error.to_string()));
                            return ExitCode::FAILURE;
                        }
                        ExitCode::SUCCESS
                    }
                    Err(error) => {
                        report_execution_error(error, DiagnosticSink::terminal(palette, repo))
                    }
                }
            } else {
                print_report(&report, args.json);
                ExitCode::from(report.status.exit_code())
            }
        }
        TaskCommand::List | TaskCommand::Plan { .. } => {
            let mut sink = DiagnosticSink::terminal(palette, repo);
            let evaluated = match eval_repo(repo, None, &mut sink) {
                Ok(value) => value,
                Err(code) => return code,
            };
            let ir = match validated_ir(&evaluated, &mut sink) {
                Ok(value) => value,
                Err(code) => return code,
            };
            if let TaskCommand::Plan { name } = &args.command {
                match gripsack_exec::plan_task(&ir, name) {
                    Ok(plan) => {
                        if args.json {
                            println!(
                                "{}",
                                serde_json::to_string(&plan).expect("serializable plan")
                            );
                        } else {
                            println!("{}", tame(plan.order.join(" -> ")));
                            println!("{}", plan.host_effects);
                            for lock in &plan.mutation_locks {
                                println!("lock {:?}: {}", lock.scope, display(&lock.key));
                            }
                        }
                        ExitCode::SUCCESS
                    }
                    Err(error) => report_execution_error(error, sink),
                }
            } else {
                let mut tasks: Vec<_> = ir
                    .workspace
                    .outputs
                    .iter()
                    .filter_map(|output| match output {
                        gripsack_ir::workspace_model::WorkspaceOutput::Task(task) => {
                            Some(&task.name)
                        }
                        _ => None,
                    })
                    .collect();
                tasks.sort();
                if args.json {
                    println!("{}", serde_json::json!({"tasks":tasks}));
                } else {
                    for task in tasks {
                        println!("{}", display(task));
                    }
                }
                ExitCode::SUCCESS
            }
        }
    }
}

fn print_report(report: &gripsack_exec::TaskReport, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(report).expect("serializable task report")
        );
    } else {
        eprintln!(
            "grip: task {} {:?}; invocation {}",
            display(&report.task),
            report.status,
            report.invocation
        );
        if let Some(error) = &report.error {
            eprintln!("grip: {}", display(error));
        }
        if report.root_retained {
            eprintln!(
                "grip: unconfirmed cleanup retains process roots and mutation exclusion; no automatic retry"
            );
        }
    }
}
pub(super) fn finish(report: gripsack_exec::TaskReport, json: bool) -> ExitCode {
    print_report(&report, json);
    if matches!(report.status, TaskStatus::Failed | TaskStatus::Cancelled)
        && let Some(receipt) = report.last_receipt()
        && receipt.disposition == gripsack_process::ProcessDisposition::Exited
    {
        // A task is a graph invocation, not an exec-style process replacement.
        // Report a child's signal numerically; never crash the CLI itself.
        // Supervisor cancellation/deadline/cleanup retain their own authority,
        // even when bounded cleanup reaped the child with SIGKILL.
        if let Some(signal) = receipt.signal {
            return ExitCode::from(128 + signal as u8);
        }
        if let Some(code) = receipt.exit_code.filter(|code| *code != 0) {
            return ExitCode::from(code as u8);
        }
    }
    ExitCode::from(report.status.exit_code())
}
