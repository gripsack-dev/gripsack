//! `grip` — the gripsack CLI.
//!
//! ```text
//! grip init            scaffold an env repo from the embedded template
//! grip check           validate captured workspace outputs and sources
//! grip apply|plan      fetch, build, deploy — one atomic generation per run
//! grip rollback|generations|gc|why-owns
//!                      generations, instant rollback, store hygiene
//! grip update          re-resolve pins into the lockfile
//! grip trust           repo trust list — the gate before any eval (0013 D7)
//! ```
//!
//! Colors and source snippets live in [`render`]; they follow the
//! terminal — piped output is plain.

mod commands;
mod render;

use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use render::Palette;

use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "grip",
    version,
    about = "gripsack — your whole environment in one bag"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Adopt an existing config path into a workspace profile.
    /// Review generated sources, then resume with prior-state capture.
    Adopt {
        /// Path to adopt (e.g. ~/.config/helix or ~/.gitconfig)
        path: String,
        /// Profile name (default: derived from the path)
        #[arg(long)]
        name: Option<String>,
        /// Ownership mode: owned | tracked_copy | merge (default:
        /// recommended from what the path is)
        #[arg(long)]
        mode: Option<String>,
        /// Workspace lock identity (default: configured host or hostname)
        #[arg(long)]
        host: Option<String>,
        /// Apply without the confirmation prompt
        #[arg(long)]
        yes: bool,
        /// Apply an already-generated, newly approved profile without writing repo files
        #[arg(long, conflicts_with = "mode")]
        resume: bool,
    },
    /// Deploy workspace profiles as one atomic generation
    Apply {
        /// Workspace lock identity (default: configured host or hostname)
        #[arg(long)]
        host: Option<String>,
        /// Env repo path or git URL (default: current directory)
        #[arg(long)]
        repo: Option<String>,
        /// Restrict to these profiles (default: all profiles)
        profiles: Vec<String>,
        /// Overwrite foreign/drifted tracked_copy destinations
        #[arg(long)]
        take_over: bool,
    },
    /// Realize named workspace recipes/packages into the native store.
    /// Linux production is submitted as one checked BuildKit subgraph.
    Build(commands::BuildArgs),
    /// Inspect, stop or clean the recorded owned Linux builder.
    Builder {
        #[command(subcommand)]
        command: commands::BuilderCommand,
    },
    /// Validate a workspace catalog without host activation.
    /// The provisioned frontend may be prepared; no builder starts.
    Check {
        /// Workspace lock identity (default: configured host or hostname)
        #[arg(long)]
        host: Option<String>,
        /// Env repo path or git URL (default: current directory)
        #[arg(long)]
        repo: Option<String>,
        /// Emit the check report as one JSON document on stdout — the
        /// same diagnostics the terminal renders (0052 A1-06)
        #[arg(long)]
        json: bool,
    },
    /// Preview workspace profile operations without deployment
    Plan {
        #[arg(long)]
        host: Option<String>,
        profiles: Vec<String>,
        /// Read IR JSON directly (frontend debugging)
        #[arg(long)]
        ir: Option<PathBuf>,
        /// Env repo path or git URL (default: current directory)
        #[arg(long)]
        repo: Option<String>,
    },
    /// Flip `current` back to a previous generation
    Rollback {
        /// Generation number (default: the previous one)
        generation: Option<gripsack_store::GenerationId>,
    },
    /// Run one command inside a named project environment (no generation)
    Run(commands::RunArgs),
    /// Open an interactive shell inside a named project environment
    Shell(commands::ShellArgs),
    /// Invoke a named development task once (no generation)
    Task(commands::TaskArgs),
    /// Execute an exact retained package for a generated command projection.
    #[command(name = "__package-command", hide = true)]
    RetainedPackage(commands::PackageCommandArgs),
    /// Update grip itself: tarball installs self-update in place;
    /// brew/cargo/mise installs get their manager's command
    SelfUpdate(commands::self_update::SelfUpdateArgs),
    /// Refresh source pins and captured workspace frontend/import pins
    Update {
        #[arg(long)]
        host: Option<String>,
        /// Env repo path or git URL (default: current directory)
        #[arg(long)]
        repo: Option<String>,
        /// Resolve without publishing sources or writing the lock; exit 1 on changes
        #[arg(long)]
        check: bool,
        outputs: Vec<String>,
    },
    /// List generations and their status
    Generations,
    /// Collect store paths no generation references
    Gc {
        /// Preview what would be collected, deleting nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show which profile owns a deployed path
    WhyOwns { path: String },
    /// Re-hash retained deployment and native artifact payloads
    StoreVerify {
        /// Remove corrupt paths (next apply re-fetches)
        #[arg(long)]
        repair: bool,
    },
    /// Create a workspace repo (gripsack.ts, captured config and editor settings)
    Init {
        /// Directory to initialize (default: current directory)
        dir: Option<PathBuf>,
    },
    /// Check the frontend environment (deno + the embedded frontend)
    Doctor,
    /// Inspect durable hook identities, attempts and outcomes
    Hooks {
        #[command(subcommand)]
        command: commands::HooksCommand,
    },
    /// Inspect and approve captured source bytes and their evaluation policy.
    /// Source or grant changes require renewed digest-bound approval.
    Trust {
        #[command(subcommand)]
        command: commands::TrustCommand,
    },
}

fn main() -> ExitCode {
    let palette = Palette::detect();
    let arg1 = std::env::args().nth(1);
    if matches!(arg1.as_deref(), Some("--version") | Some("-V")) {
        let (name, version) = ("grip", env!("CARGO_PKG_VERSION"));
        if palette.enabled {
            println!("{} {}", name.green().bold(), version.cyan());
        } else {
            println!("{name} {version}");
        }
        return ExitCode::SUCCESS;
    }
    let command = match Cli::parse().command {
        // Fixture simulations must not even create a diagnostic run in the
        // user's real home; their workers also start before any tracing thread.
        Command::Hooks { command } if command.fixture_only() => {
            return commands::hooks(command, palette);
        }
        command => command,
    };
    let command_name = match &command {
        Command::Adopt { .. } => "Adopt",
        Command::Apply { .. } => "Apply",
        Command::Build(_) => "Build",
        Command::Builder { .. } => "Builder",
        Command::Check { .. } => "Check",
        Command::Plan { .. } => "Plan",
        Command::Rollback { .. } => "Rollback",
        Command::Run(_) => "Run",
        Command::Shell(_) => "Shell",
        Command::Task(_) => "Task",
        Command::RetainedPackage(_) => "PackageCommand",
        Command::SelfUpdate(_) => "SelfUpdate",
        Command::Update { .. } => "Update",
        Command::Generations => "Generations",
        Command::Gc { .. } => "Gc",
        Command::WhyOwns { .. } => "WhyOwns",
        Command::StoreVerify { .. } => "StoreVerify",
        Command::Init { .. } => "Init",
        Command::Doctor => "Doctor",
        Command::Trust { .. } => "Trust",
        Command::Hooks { .. } => "Hooks",
    };
    // Retained wrappers name their private store explicitly; an absent or
    // unrelated ambient HOME must not redirect their diagnostic state.
    let run = match &command {
        Command::RetainedPackage(args) => gripsack_trace::init(&args.home),
        _ => gripsack_trace::init(&gripsack_store::gripsack_home()),
    }
    .ok();
    let _run_span = run.map(|r| gripsack_trace::run_span!(r, command_name).entered());
    match command {
        Command::Doctor => commands::doctor(palette),
        Command::Adopt {
            path,
            name,
            mode,
            host,
            yes,
            resume,
        } => commands::adopt(
            &path,
            name.as_deref(),
            mode.as_deref(),
            host.as_deref(),
            yes,
            resume,
            palette,
        ),
        Command::Apply {
            host,
            repo,
            profiles,
            take_over,
        } => match commands::resolve_repo(repo.as_deref()) {
            Ok(repo) => commands::apply(
                &repo,
                commands::ApplyOptions {
                    host,
                    profiles,
                    take_over,
                    take_over_entries: None,
                },
                palette,
            ),
            Err(code) => code,
        },
        Command::Build(mut args) => match commands::resolve_repo(args.repo.take().as_deref()) {
            Ok(repo) => commands::build(&repo, args, palette),
            Err(code) => code,
        },
        Command::Builder { command } => commands::builder(command),
        Command::Check { host, repo, json } => match commands::resolve_repo(repo.as_deref()) {
            Ok(repo) => {
                let sink = if json {
                    render::DiagnosticSink::json()
                } else {
                    render::DiagnosticSink::terminal(palette, &repo)
                };
                commands::check(&repo, host, sink)
            }
            Err(code) => code,
        },
        Command::Gc { dry_run } => commands::gc(palette, dry_run),
        Command::WhyOwns { path } => commands::why_owns(&path, palette),
        Command::StoreVerify { repair } => commands::store_verify(repair, palette),
        Command::Init { dir } => {
            commands::init(&dir.unwrap_or_else(|| PathBuf::from(".")), palette)
        }
        Command::Generations => commands::generations(),
        Command::SelfUpdate(args) => commands::self_update::self_update(palette, args),
        Command::Update {
            host,
            repo,
            outputs,
            check,
        } => match commands::resolve_repo(repo.as_deref()) {
            Ok(repo) => commands::update(&repo, host, outputs, palette, check),
            Err(code) => {
                if check {
                    ExitCode::from(2)
                } else {
                    code
                }
            }
        },
        Command::Rollback { generation } => commands::rollback(generation, palette),
        Command::Run(mut args) => match commands::resolve_repo(args.repo.take().as_deref()) {
            Ok(repo) => commands::run(&repo, args, palette),
            Err(code) => code,
        },
        Command::Shell(mut args) => match commands::resolve_repo(args.repo.take().as_deref()) {
            Ok(repo) => commands::shell(&repo, args, palette),
            Err(code) => code,
        },
        Command::Task(mut args) => match args.resolve_repository() {
            Ok(repo) => commands::task(&repo, args, palette),
            Err(code) => code,
        },
        Command::RetainedPackage(args) => commands::package_command(args, palette),
        Command::Trust { command } => commands::trust(command, palette),
        Command::Hooks { command } => commands::hooks(command, palette),
        Command::Plan {
            ir: Some(path),
            profiles,
            ..
        } => commands::plan_ir(&path, &profiles, palette),
        Command::Plan {
            ir: None,
            host,
            profiles,
            repo,
        } => {
            let repo = match commands::resolve_repo(repo.as_deref()) {
                Ok(r) => r,
                Err(code) => return code,
            };
            let mut sink = render::DiagnosticSink::terminal(palette, &repo);
            let outcome = match commands::eval_repo(&repo, host, &mut sink) {
                Ok(o) => o,
                Err(code) => return code,
            };
            let repository = gripsack_exec::Repository::evaluated(
                std::sync::Arc::clone(&outcome.sources),
                outcome.receipt,
            );
            // the same validation pipeline check/apply run (0033 R5):
            // a plan that succeeds where apply would fail is a lie
            let ir = match commands::validated_ir(&outcome, &mut sink) {
                Ok(ir) => ir,
                Err(code) => return code,
            };
            if let Err(code) = commands::reject_workspace_execution(
                &ir,
                gripsack_ir::workspace::WorkspaceOperation::Plan,
                &mut sink,
            ) {
                return code;
            }
            // host-inputs header (0013 D6): the facts that went in and
            // the probes the core bound — what keeps hardware-reactive
            // plans from reading as nondeterminism.
            println!(
                "{}",
                palette.dim(&commands::render_host_inputs(&outcome.host_inputs))
            );
            match render::diff_section(
                &ir,
                &repository,
                &Default::default(),
                palette,
                &profiles,
                outcome.fetch.limits(),
            ) {
                Ok(section) => println!("{section}"),
                Err(gripsack_exec::ExecError::Gate(diagnostic)) => {
                    sink.report(&[diagnostic]);
                    return ExitCode::FAILURE;
                }
                Err(error) => {
                    eprintln!("grip: cannot compute the preview: {error}");
                    return ExitCode::FAILURE;
                }
            }
            let selected = ir.workspace_outputs().filter(|output| {
                profiles.is_empty() || profiles.iter().any(|name| name == output.name)
            });
            for output in selected {
                println!("  {:?} ({})", output.name, output.kind);
            }
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_selects_profiles_and_keeps_host_as_lock_identity() {
        let cli = Cli::try_parse_from(["grip", "apply", "--host", "work", "desktop"]).unwrap();
        match cli.command {
            Command::Apply { host, profiles, .. } => {
                assert_eq!(host.as_deref(), Some("work"));
                assert_eq!(profiles, ["desktop"]);
            }
            _ => panic!("expected apply"),
        }
    }

    #[test]
    fn removed_execution_flags_are_not_accepted() {
        for args in [
            vec!["grip", "apply", "--migrate-from", "1"],
            vec!["grip", "plan", "--migrate-from", "1"],
            vec!["grip", "apply", "--jobs", "2"],
            vec![
                "grip",
                "adopt",
                "~/.gitconfig",
                "--resume",
                "--mode",
                "owned",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}
