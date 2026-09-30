use super::frontend::Frontend;
use super::prepared::{PreparedEvaluation, operational};
use super::probe::InputsFile;
use crate::render::DiagnosticSink;
use gripsack_ir::{HostName, Ir, Severity};
use gripsack_store::trust::evaluation::{EvaluationId, EvaluationOutcome, EvaluationSession};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

/// The eval wire protocol (0011 §5, 0013 D6): the frontend emits the
/// IR plus any diagnostics (lint results, frontend-side validation)
/// and its symbolic probe requests — the sandbox cannot run probes,
/// so `ctx.probe.*` can only ever record a request here.
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

/// What a successful eval hands back: the IR JSON, the env config,
/// and the host inputs it ran with.
pub struct EvalOutcome {
    pub ir_json: String,
    pub env: gripsack_config::EnvConfig,
    /// Post-injection artifact clients; provisioning uses a separate context.
    pub fetch: std::sync::Arc<gripsack_fetch::FetchContext>,
    pub host_inputs: HostInputs,
    /// The host entrypoint actually evaluated — the same resolution
    /// (--host > env.toml default_host > detected hostname) every
    /// post-eval command must key its lockfile and generations by.
    /// Commands used to re-derive it with drifting rules (`update`
    /// read $HOSTNAME, a bash-ism POSIX sh does not export) and pick
    /// a different lockfile than the eval that preceded them.
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
    let approved = match prepared.authorize(!sink.is_json()) {
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
        provision_plugins(
            &prepared.config.fetchers,
            &prepared.env.linters,
            &prepared.provisioning,
        )?;
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
        env: prepared.env,
        fetch,
        host_inputs,
        host: prepared.host,
        sources,
        receipt: receipt_id,
    })
}

/// Linter configuration and source reads belong to the evaluated snapshot.
pub fn run_lints(
    ir: &Ir,
    outcome: &EvalOutcome,
    sink: &mut DiagnosticSink,
) -> Result<(), ExitCode> {
    let mut diagnostics = gripsack_lint::run(
        ir,
        &outcome.env.linters,
        outcome.sources.repository(),
        &outcome.host,
    );
    super::prepared::map_diagnostics(&outcome.sources, &mut diagnostics);
    if diagnostics.is_empty() {
        return Ok(());
    }
    let failed = diagnostics.iter().any(|d| d.severity == Severity::Error);
    sink.report(&diagnostics);
    if failed {
        return Err(ExitCode::FAILURE);
    }
    Ok(())
}

/// The post-eval validation pipeline every content command runs:
/// IR sema → module source validation → linters (0011 §9). One
/// implementation — check and apply used to carry identical
/// and_then chains that could drift.
pub fn validated_ir(outcome: &EvalOutcome, sink: &mut DiagnosticSink) -> Result<Ir, ExitCode> {
    let ir = check_ir(&outcome.ir_json, sink)?;
    crate::commands::validate_sources(&ir, outcome.sources.repository(), sink)?;
    run_lints(&ir, outcome, sink)?;
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

/// Native file profiles execute; unsupported catalog capabilities must never
/// fall through to an empty legacy apply or an alternate fallback executor.
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

/// E110: entries sourced only from the repo must exist at check time.
/// Fetches and producer recipes supply a staged payload instead; a fetch-less
/// shell/run build may legitimately create files that do not exist yet.
pub fn validate_sources(ir: &Ir, repo: &Path, sink: &mut DiagnosticSink) -> Result<(), ExitCode> {
    let mut diagnostics = Vec::new();
    for (name, module) in &ir.modules {
        let prepared = match gripsack_ir::prepared::PreparedModule::new(module) {
            Ok(prepared) => prepared,
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                continue;
            }
        };
        if prepared.fetch().is_some() || prepared.has_recipe() {
            continue;
        }
        for entry in prepared.entries() {
            if !repo.join(&entry.from).exists() {
                diagnostics.push(
                    gripsack_ir::Diagnostic::error(
                        gripsack_ir::codes::MISSING_SOURCE,
                        format!("module {name:?}: no payload or repo file at {}", entry.from),
                    )
                    .with_label(entry.span.clone().or_else(|| module.span.clone()), "source declared here")
                    .with_help("fix the repository path, or declare the producer that supplies this payload"),
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

/// Stage: declared plugins (0012 §move-2). `package = "owner/repo@tag"`
/// on a [fetchers.x] or [linters.x] entry provisions the binary into
/// the plugin store — declarative, sha256-verified, receipted.
/// Fetchers and linters both resolve from the store downstream.
fn provision_plugins(
    fetchers: &BTreeMap<String, gripsack_config::FetcherSectionView>,
    linters: &BTreeMap<String, gripsack_config::LinterSection>,
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
    for (name, section) in linters {
        if let Some(package) = &section.package
            && gripsack_fetch::plugins::parse_ref(package).is_some()
        {
            provision(&store, fetch, name, package, "griplint")?;
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
