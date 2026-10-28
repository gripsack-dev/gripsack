use super::frontend::Frontend;
use super::prepared::{PreparedEvaluation, operational};
use super::probe::InputsFile;
use crate::render::DiagnosticSink;
use gripsack_ir::{HostName, Ir};
use gripsack_store::trust::evaluation::{EvaluationId, EvaluationOutcome, EvaluationSession};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

/// The frontend emits IR, diagnostics and symbolic probe requests.
/// The sandbox cannot run probes; the core binds each requested result.
#[derive(serde::Deserialize)]
pub(super) struct EvalEnvelope {
    pub(super) ir: serde_json::Value,
    #[serde(default)]
    pub(super) diagnostics: Vec<gripsack_ir::Diagnostic>,
    #[serde(default)]
    pub(super) probe_requests: Vec<super::probe::ProbeRequest>,
}

/// Host inputs an eval ran with (0013 D6) — what `grip plan`'s
/// host-inputs header shows: the facts in, the probe results bound.
/// Probes re-evaluate every run; the header is what keeps that from
/// reading as nondeterminism.
#[derive(Debug, Clone)]
pub struct HostInputs {
    pub facts: gripsack_exec::facts::HostFacts,
    pub probes: BTreeMap<String, bool>,
}

/// The plan host-inputs header, rendered plain — the caller colors.
pub fn render_host_inputs(inputs: &HostInputs) -> String {
    let f = &inputs.facts;
    let libc = f.libc.as_deref().unwrap_or("unknown libc");
    let mut out = format!(
        "host inputs: {}/{} · {} · {}",
        f.os, f.arch, libc, f.hostname
    );
    for (probe, hit) in &inputs.probes {
        out.push_str(&format!(
            "\n  probe {probe}: {}",
            if *hit { "yes" } else { "no" }
        ));
    }
    out
}

/// Evaluated IR, captured sources, acquisition policy and bound host inputs.
pub struct EvalOutcome {
    pub ir_json: String,
    /// Post-injection artifact clients; provisioning uses a separate context.
    pub fetch: std::sync::Arc<gripsack_fetch::FetchContext>,
    pub host_inputs: HostInputs,
    /// Workspace lock identity: --host, configured default_host, then hostname.
    /// This never selects a source entrypoint.
    pub host: HostName,
    /// Keeps every captured source alive through downstream materialization.
    pub sources: std::sync::Arc<gripsack_store::source_bundle::SourceBundle>,
    pub receipt: EvaluationId,
}

/// Evaluate an env repo's frontend into IR JSON (0005 §4). The core
/// never embeds a runtime — this is a deno subprocess, sandboxed
/// deny-by-default (0013 D2): no env, no network, no subprocesses,
/// reads limited to the approved captured roots and one immutable input file.
/// Effects the frontend wants arrive as symbolic probe
/// requests; the core binds them and re-runs (two-stage eval, D6).
#[tracing::instrument(name = "eval", skip(sink), fields(host))]
pub fn eval_repo(
    repo: &Path,
    host: Option<String>,
    sink: &mut DiagnosticSink,
) -> Result<EvalOutcome, ExitCode> {
    let prepared = PreparedEvaluation::capture(repo, host, sink)?;
    eval_prepared(prepared, sink)
}

/// Continue with the exact prepared source selected by a compound command.
pub(super) fn eval_prepared(
    prepared: PreparedEvaluation,
    sink: &mut DiagnosticSink,
) -> Result<EvalOutcome, ExitCode> {
    tracing::Span::current().record("host", prepared.host.as_str());
    let mut receipt = EvaluationSession::begin(
        prepared.home(),
        prepared.sources(),
        prepared.policy(),
        prepared.provenance(),
    )
    .map_err(operational)?;
    let receipt_id = receipt.id();
    let approved = match prepared.authorize(!sink.is_json(), None) {
        Ok(approved) => approved,
        Err(code) => {
            receipt
                .finish(EvaluationOutcome::Rejected)
                .map_err(operational)?;
            return Err(code);
        }
    };
    let result: Result<_, ExitCode> = (|| {
        // Host facts precede the artifact/build overlay and are not part of
        // source approval. Only the core binds subsequent explicit probes.
        let facts = gripsack_exec::facts::detect();
        gripsack_fetch::throttle::install(
            &prepared.env.throttle,
            Some(prepared.home().join("throttle.json")),
        );
        provision_plugins(&prepared.config.fetchers, &prepared.provisioning)?;
        let fetch = std::sync::Arc::new(gripsack_fetch::FetchContext::artifacts(
            acquisition_limits(&prepared.config.settings),
            std::sync::Arc::clone(&prepared.provisioning),
            prepared.env.eval.env.clone(),
        ));
        let inputs = InputsFile::create().map_err(operational)?;
        let frontend = Frontend::new(&approved);
        let (envelope, bound) = super::probe::eval_to_fixpoint(
            &frontend,
            prepared.host.as_str(),
            facts,
            &inputs,
            sink,
            &mut receipt,
        )?;
        sink.report(&envelope.diagnostics);
        Ok((
            envelope.ir.to_string(),
            fetch,
            HostInputs {
                facts: facts.clone(),
                probes: bound,
            },
        ))
    })();
    receipt
        .finish(if result.is_ok() {
            EvaluationOutcome::Completed
        } else {
            EvaluationOutcome::Failed
        })
        .map_err(operational)?;
    let (ir_json, fetch, host_inputs) = result?;
    tracing::info!(evaluation = %receipt_id, source = %prepared.sources().digest(), "evaluation source and inputs recorded");
    let sources = std::sync::Arc::clone(prepared.sources());
    Ok(EvalOutcome {
        ir_json,
        fetch,
        host_inputs,
        host: prepared.host,
        sources,
        receipt: receipt_id,
    })
}

/// Every content command validates IR semantics and captured workspace sources.
pub fn validated_ir(outcome: &EvalOutcome, sink: &mut DiagnosticSink) -> Result<Ir, ExitCode> {
    let ir = check_ir(&outcome.ir_json, sink)?;
    crate::commands::validate_sources(&ir, outcome.sources.repository(), sink)?;
    Ok(ir)
}

/// Parse + validate IR, rendering diagnostics on failure.
pub fn check_ir(json: &str, sink: &mut DiagnosticSink) -> Result<Ir, ExitCode> {
    gripsack_ir::check(json).map_err(|diagnostics| {
        for d in &diagnostics {
            tracing::error!(code = d.code.as_ref(), "{}", d.message);
        }
        sink.report(&diagnostics);
        ExitCode::FAILURE
    })
}

/// Reject workspace capabilities that the selected operation cannot execute.
pub fn reject_workspace_execution(
    ir: &Ir,
    operation: gripsack_ir::workspace::WorkspaceOperation,
    sink: &mut DiagnosticSink,
) -> Result<(), ExitCode> {
    let Some(diagnostic) = ir.workspace_execution_error(operation) else {
        return Ok(());
    };
    sink.report(&[diagnostic]);
    Err(ExitCode::FAILURE)
}

/// Repository file origins must exist in the approved captured source tree.
pub fn validate_sources(ir: &Ir, repo: &Path, sink: &mut DiagnosticSink) -> Result<(), ExitCode> {
    use gripsack_ir::workspace_model::{WorkspaceOutput, WorkspaceSource};
    let mut diagnostics = Vec::new();
    for output in &ir.workspace.outputs {
        let WorkspaceOutput::Profile(profile) = output else {
            continue;
        };
        for file in &profile.files {
            let path = match &file.source {
                Some(
                    WorkspaceSource::RepoFile { path } | WorkspaceSource::RepoTree { path, .. },
                ) => path,
                _ => continue,
            };
            if !repo.join(path).exists() {
                diagnostics.push(
                    gripsack_ir::Diagnostic::error(
                        gripsack_ir::codes::MISSING_SOURCE,
                        format!(
                            "profile {:?}: no captured repository source at {path}",
                            profile.name
                        ),
                    )
                    .with_label(Some(file.span.clone()), "source declared here")
                    .with_help("fix the repository path or the capture exclusions"),
                );
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        sink.report(&diagnostics);
        Err(ExitCode::FAILURE)
    }
}

/// Named nonzero config limits become one command-owned acquisition policy.
pub(super) fn acquisition_limits(
    settings: &gripsack_config::Settings,
) -> gripsack_fetch::FetchLimits {
    let defaults = gripsack_fetch::FetchLimits::default();
    gripsack_fetch::FetchLimits {
        concurrent: settings.acquisition_jobs.unwrap_or(defaults.concurrent),
        download_bytes: settings
            .download_limit_bytes
            .unwrap_or(defaults.download_bytes),
        expanded_bytes: settings
            .expanded_limit_bytes
            .unwrap_or(defaults.expanded_bytes),
        archive_entries: settings
            .archive_entry_limit
            .unwrap_or(defaults.archive_entries),
        decoder_bytes: settings
            .decoder_memory_bytes
            .unwrap_or(defaults.decoder_bytes),
    }
}

/// Provision declared fetcher plugins into the verified plugin store.
fn provision_plugins(
    fetchers: &BTreeMap<String, gripsack_config::FetcherSectionView>,
    fetch: &gripsack_fetch::FetchContext,
) -> Result<(), ExitCode> {
    let store = gripsack_fetch::plugins::PluginStore::new(&gripsack_store::gripsack_home());
    for (name, section) in fetchers {
        // an explicit executable path (path = the registry-symmetric
        // name; plugin = its original alias) registers directly —
        // no provisioning, no network (the offline route)
        let explicit = section.path.as_ref().or(section.plugin.as_ref());
        if let Some(exe) = explicit {
            if section.package.is_some() {
                eprintln!(
                    "grip: [fetchers.{name}] declares an executable path and a package — pick one"
                );
                return Err(ExitCode::FAILURE);
            }
            if exe.contains('/') {
                gripsack_fetch::register_fetcher_path(name, exe.into());
            }
            continue;
        }
        if let Some(package) = &section.package {
            if section.plugin.is_some() {
                eprintln!("grip: [fetchers.{name}] declares both plugin and package — pick one");
                return Err(ExitCode::FAILURE);
            }
            provision(&store, fetch, name, package, "gripfetch")?;
        }
    }
    Ok(())
}

/// Provision one plugin; the fresh-install line is the trust notice
/// (a new binary runs with your user rights — name its source).
fn provision(
    store: &gripsack_fetch::plugins::PluginStore,
    fetch: &gripsack_fetch::FetchContext,
    name: &str,
    package: &str,
    kind: &str,
) -> Result<(), ExitCode> {
    let before = store.receipt(&format!("{kind}-{name}"));
    let bin = store.ensure(fetch, name, package, kind).map_err(|e| {
        eprintln!("grip: cannot provision {kind}-{name} from {package}: {e}");
        ExitCode::FAILURE
    })?;
    let after = store.receipt(&format!("{kind}-{name}"));
    if before != after {
        eprintln!("installed {kind}-{name} from {package} → {}", bin.display());
    }
    Ok(())
}
