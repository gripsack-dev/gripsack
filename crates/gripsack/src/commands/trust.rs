//! Non-evaluating source/policy inspection, explicit approval and revocation.
use super::{
    expand_home,
    prepared::{PreparedEvaluation, operational},
};
use crate::render::{DiagnosticSink, Palette};
use clap::Subcommand;
use gripsack_store::{
    source_bundle::{SourceBundleDigest, SourceInventory},
    trust::{
        self, ApprovalInspection, EvaluationPolicy, EvaluationPolicyDigest, GitProvenance,
        evaluation::EvaluationId,
    },
};
use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Debug, Subcommand)]
pub enum TrustCommand {
    /// List recorded source approvals and legacy entries requiring renewal.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Capture source and show its exact digest, policy and inventory changes.
    Inspect {
        path: Option<String>,
        #[arg(long)]
        json: bool,
        /// Inspect one existing private evaluation receipt without recapturing.
        #[arg(long, conflicts_with = "path")]
        receipt: Option<String>,
    },
    /// Approve captured source and policy. Both expected digests are required
    /// without a terminal; edits or grant/runtime changes require new approval.
    Add {
        path: Option<String>,
        #[arg(long, requires = "policy")]
        bundle: Option<String>,
        #[arg(long, requires = "bundle")]
        policy: Option<String>,
    },
    /// Revoke recorded approvals, including old path-only entries.
    Remove { path: String },
}

pub fn trust(command: TrustCommand, palette: Palette) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    match command {
        TrustCommand::List { json } => list(&home, json),
        TrustCommand::Inspect {
            path,
            json,
            receipt,
        } => {
            if let Some(receipt) = receipt {
                return inspect_receipt(&home, &receipt, json);
            }
            let path = match repo_path(path) {
                Ok(path) => path,
                Err(code) => return code,
            };
            let mut sink = DiagnosticSink::terminal(palette, &path);
            let prepared = match PreparedEvaluation::capture(&path, None, &mut sink) {
                Ok(prepared) => prepared,
                Err(code) => return code,
            };
            if json {
                inspect_json(&prepared)
            } else {
                match prepared.show_approval() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(code) => code,
                }
            }
        }
        TrustCommand::Add {
            path,
            bundle,
            policy,
        } => {
            if bundle.is_none() && !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
                eprintln!(
                    "grip: non-interactive approval requires --bundle and --policy from `grip trust inspect --json`; path-only approval is no longer supported"
                );
                return ExitCode::FAILURE;
            }
            let expected = match (bundle, policy) {
                (Some(bundle), Some(policy)) => match (
                    SourceBundleDigest::parse(&bundle),
                    EvaluationPolicyDigest::parse(&policy),
                ) {
                    (Ok(bundle), Ok(policy)) => Some((bundle, policy)),
                    (Err(error), _) | (_, Err(error)) => return operational(error),
                },
                (None, None) => None,
                _ => return operational("both --bundle and --policy are required"),
            };
            let path = match repo_path(path) {
                Ok(path) => path,
                Err(code) => return code,
            };
            let mut sink = DiagnosticSink::terminal(palette, &path);
            let prepared = match PreparedEvaluation::capture(&path, None, &mut sink) {
                Ok(prepared) => prepared,
                Err(code) => return code,
            };
            if let Some((bundle, policy)) = expected {
                if let Err(error) = trust::approve(
                    prepared.home(),
                    prepared.sources(),
                    prepared.policy(),
                    bundle,
                    policy,
                    prepared.provenance(),
                ) {
                    return operational(error);
                }
            } else if let Err(code) = prepared.authorize(true) {
                return code;
            }
            println!(
                "approved source {} policy {} for {}",
                prepared.sources().digest(),
                match prepared.policy().digest() {
                    Ok(policy) => policy,
                    Err(error) => return operational(error),
                },
                gripsack_process::terminal::tame(
                    prepared
                        .sources()
                        .repository_identity()
                        .display()
                        .to_string()
                )
            );
            ExitCode::SUCCESS
        }
        TrustCommand::Remove { path } => match trust::remove(&home, &expand_home(&path)) {
            Ok(true) => {
                println!(
                    "revoked approval for {}",
                    gripsack_process::terminal::tame(path)
                );
                ExitCode::SUCCESS
            }
            Ok(false) => operational("no recorded approval for that repository"),
            Err(error) => operational(error),
        },
    }
}

fn repo_path(value: Option<String>) -> Result<PathBuf, ExitCode> {
    let value = value
        .map(|value| {
            if value.starts_with('~') {
                expand_home(&value)
                    .into_os_string()
                    .into_string()
                    .map_err(|_| operational("repository path is not UTF-8"))
            } else {
                Ok(value)
            }
        })
        .transpose()?;
    super::resolve_repo(value.as_deref())
}

fn list(home: &Path, json: bool) -> ExitCode {
    let listing = match trust::list(home) {
        Ok(listing) => listing,
        Err(error) => return operational(error),
    };
    if json {
        return print_json(&listing);
    }
    if listing.approved.is_empty() && listing.legacy.is_empty() {
        println!("no recorded source approvals");
    }
    for entry in &listing.approved {
        println!(
            "{}",
            gripsack_process::terminal::tame(entry.repository.clone())
        );
        println!(
            "  bundle: {}  policy: {}",
            entry.bundle, entry.policy_digest
        );
        println!(
            "  {:?}  roots: {:?}  native configuration: {}",
            entry.policy.profile, entry.policy.read_roots, entry.policy.native_configuration_sha256
        );
    }
    for entry in &listing.legacy {
        println!(
            "{} — legacy path-only approval; renewal required",
            gripsack_process::terminal::tame(entry.path.clone())
        );
    }
    ExitCode::SUCCESS
}

#[derive(serde::Serialize)]
struct Inspection<'a> {
    version: u32,
    repository: &'a Path,
    bundle_digest: SourceBundleDigest,
    policy_digest: EvaluationPolicyDigest,
    policy: &'a EvaluationPolicy,
    provenance: &'a GitProvenance,
    inventory: &'a SourceInventory,
    approval: ApprovalInspection,
}

fn inspect_json(prepared: &PreparedEvaluation) -> ExitCode {
    let approval = match trust::inspect(prepared.home(), prepared.sources(), prepared.policy()) {
        Ok(approval) => approval,
        Err(error) => return operational(error),
    };
    let policy_digest = match prepared.policy().digest() {
        Ok(digest) => digest,
        Err(error) => return operational(error),
    };
    print_json(&Inspection {
        version: 1,
        repository: prepared.sources().repository_identity(),
        bundle_digest: prepared.sources().digest(),
        policy_digest,
        policy: prepared.policy(),
        provenance: prepared.provenance(),
        inventory: prepared.sources().inventory(),
        approval,
    })
}

fn inspect_receipt(home: &Path, id: &str, json: bool) -> ExitCode {
    let receipt = EvaluationId::parse(id).and_then(|id| trust::evaluation::read(home, id));
    match receipt {
        Err(error) => operational(error),
        Ok(receipt) if json => print_json(&receipt),
        Ok(receipt) => {
            println!("evaluation {}: {:?}", receipt.id, receipt.outcome);
            println!(
                "  repository: {}",
                gripsack_process::terminal::tame(receipt.repository)
            );
            println!(
                "  bundle: {}  policy: {}",
                receipt.source, receipt.policy_digest
            );
            for round in &receipt.rounds {
                println!(
                    "  round {}: input {} process {:?}",
                    round.number,
                    round.input_sha256,
                    round.process.as_ref().map(|process| &process.disposition)
                );
            }
            ExitCode::SUCCESS
        }
    }
}

fn print_json(value: &impl serde::Serialize) -> ExitCode {
    let mut output = io::stdout().lock();
    if let Err(error) = serde_json::to_writer_pretty(&mut output, value) {
        return operational(error);
    }
    if let Err(error) = writeln!(output) {
        return operational(error);
    }
    ExitCode::SUCCESS
}
