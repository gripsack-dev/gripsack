use super::frontend::Frontend;
use super::probe::InputsFile;
use crate::render::{self, Palette};
use gripsack_ir::{Ir, Severity};
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
    pub host_inputs: HostInputs,
    /// The host entrypoint actually evaluated — the same resolution
    /// (--host > env.toml default_host > detected hostname) every
    /// post-eval command must key its lockfile and generations by.
    /// Commands used to re-derive it with drifting rules (`update`
    /// read $HOSTNAME, a bash-ism POSIX sh does not export) and pick
    /// a different lockfile than the eval that preceded them.
    pub host: String,
}

/// Evaluate an env repo's frontend into IR JSON (0005 §4). The core
/// never embeds a runtime — this is a deno subprocess, sandboxed
/// deny-by-default (0013 D2): no env, no network, no subprocesses,
/// reads limited to the repo, the inputs dir, and the materialized
/// frontend. Effects the frontend wants arrive as symbolic probe
/// requests; the core binds them and re-runs (two-stage eval, D6).
#[tracing::instrument(name = "eval", skip(palette), fields(host))]
pub fn eval_repo(
    repo: &Path,
    host: Option<&str>,
    palette: Palette,
) -> Result<EvalOutcome, ExitCode> {
    let env_path = repo.join("env.toml");
    if !env_path.exists() {
        eprintln!(
            "grip: no env.toml in {} — is this an env repo?",
            repo.display()
        );
        return Err(ExitCode::FAILURE);
    }
    let env = match gripsack_config::load_env(&env_path) {
        Ok(env) => env,
        Err(diagnostics) => {
            eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
            return Err(ExitCode::FAILURE);
        }
    };
    // Rate budgets (0002 §throttle): [throttle] in env.toml overrides
    // fetcher-declared budgets; buckets persist across runs so
    // back-to-back applies share one budget.
    gripsack_fetch::throttle::install(
        &env.throttle,
        Some(gripsack_store::gripsack_home().join("throttle.json")),
    );
    provision_plugins(&env)?;
    // Build-time env (0001 §3.10 build side): injected for the run's
    // duration so every subprocess — fetchers, build steps, plugins —
    // inherits it. A CLI exits after one run, so process-env is the
    // honest carrier; [eval] env in env.toml is the declaration point.
    // (The deno eval subprocess is the exception — it gets no env:
    // denied by the absence of --allow-env.)
    for (name, value) in &env.eval.env {
        unsafe { std::env::set_var(name, value) };
    }
    let host = host
        .map(str::to_string)
        .or_else(|| env.env.default_host.clone())
        .unwrap_or_else(crate::commands::default_host);
    tracing::Span::current().record("host", &host);

    let home = gripsack_store::gripsack_home();
    let deno = match gripsack_exec::ensure_deno(&home) {
        Ok(deno) => deno,
        Err(e) => {
            eprintln!("grip: deno provisioning failed: {e}");
            eprintln!("hint: set GRIPSACK_DENO to a deno binary to bypass provisioning");
            return Err(ExitCode::FAILURE);
        }
    };
    let frontend_dir = match gripsack_exec::ensure_ts_frontend(&home, env!("CARGO_PKG_VERSION")) {
        Ok(Some(dir)) => dir,
        Ok(None) => {
            eprintln!(
                "grip: this grip binary carries no embedded TypeScript frontend — \
                 install a release build (plan/0013 D3)"
            );
            return Err(ExitCode::FAILURE);
        }
        Err(e) => {
            eprintln!("grip: frontend materialization failed: {e}");
            return Err(ExitCode::FAILURE);
        }
    };
    let driver = frontend_dir.join("src/cli.ts");
    // the allow-read grant and the driver's import base must be the
    // same path the child sees — canonical, not CWD-relative
    let repo = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let repo = &repo;

    let facts = gripsack_exec::facts::detect();
    let inputs = InputsFile::create(&home).map_err(|e| {
        eprintln!(
            "grip: cannot prepare inputs dir under {}: {e}",
            home.display()
        );
        ExitCode::FAILURE
    })?;
    let frontend = Frontend {
        deno: &deno,
        repo,
        driver: &driver,
        frontend_dir: &frontend_dir,
        home: &home,
    };
    let (envelope, bound) =
        super::probe::eval_to_fixpoint(&frontend, &host, facts, &inputs, palette)?;
    if !envelope.diagnostics.is_empty() {
        eprintln!(
            "{}",
            render::render_diagnostics(&envelope.diagnostics, palette)
        );
    }
    Ok(EvalOutcome {
        ir_json: envelope.ir.to_string(),
        env,
        host,
        host_inputs: HostInputs {
            facts: facts.clone(),
            probes: bound,
        },
    })
}

/// Bind one probe request (0013 D6): the closed enum lives in the
/// core on purpose — binding here, on the frontend's side of the
/// boundary with core-supplied data, is what keeps the emitted IR
/// fully concrete.
/// Run registered linters against the IR's modules (0012 §move-1) and
/// render what comes back; error severity fails the command.
pub fn run_lints(
    ir: &Ir,
    outcome: &EvalOutcome,
    repo: &Path,
    host: Option<&str>,
    palette: Palette,
) -> Result<(), ExitCode> {
    let diagnostics = gripsack_lint::run(ir, &outcome.env.linters, repo, host);
    if diagnostics.is_empty() {
        return Ok(());
    }
    eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(ExitCode::FAILURE);
    }
    Ok(())
}

/// The post-eval validation pipeline every content command runs:
/// IR sema → module source validation → linters (0011 §9). One
/// implementation — check and apply used to carry identical
/// and_then chains that could drift.
pub fn validated_ir(
    outcome: &EvalOutcome,
    repo: &Path,
    host: Option<&str>,
    palette: Palette,
) -> Result<Ir, ExitCode> {
    let ir = check_ir(&outcome.ir_json, palette)?;
    crate::commands::validate_sources(&ir, repo, palette)?;
    run_lints(&ir, outcome, repo, host, palette)?;
    Ok(ir)
}

/// Parse + validate IR, rendering diagnostics on failure.
pub fn check_ir(json: &str, palette: Palette) -> Result<Ir, ExitCode> {
    gripsack_ir::check(json).map_err(|diagnostics| {
        for d in &diagnostics {
            tracing::error!(code = d.code.as_ref(), "{}", d.message);
        }
        eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
        ExitCode::FAILURE
    })
}

/// E110: a fetch-less module can only deploy repo files — a missing
/// source is statically knowable and must fail at eval/check time,
/// not mid-deploy (review finding E2). Modules with a payload (fetch
/// or a fetch step) legitimately reference into it.
pub fn validate_sources(ir: &Ir, repo: &Path, palette: Palette) -> Result<(), ExitCode> {
    let mut diagnostics = Vec::new();
    for (name, module) in &ir.modules {
        let has_payload = module.fetch.is_some()
            || module.steps.as_ref().is_some_and(|steps| {
                steps
                    .iter()
                    .any(|s| matches!(s.action, gripsack_ir::StepAction::Fetch { .. }))
            });
        if has_payload {
            continue;
        }
        for entry in module.install.iter().chain(module.config.iter()) {
            if !repo.join(&entry.from).exists() {
                diagnostics.push(
                    gripsack_ir::Diagnostic::error(
                        gripsack_ir::codes::MISSING_SOURCE,
                        format!("module {name:?}: no payload or repo file at {}", entry.from),
                    )
                    .with_label(module.span.clone(), "module declared here")
                    .with_help("fix the path, or add a fetch if the source is a payload"),
                );
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
        Err(ExitCode::FAILURE)
    }
}

/// Provision one plugin; the fresh-install line is the trust notice
/// (a new binary runs with your user rights — name its source).
/// Stage: declared plugins (0012 §move-2). `package = "owner/repo@tag"`
/// on a [fetchers.x] or [linters.x] entry provisions the binary into
/// the plugin store — declarative, sha256-verified, receipted.
/// Fetchers and linters both resolve from the store downstream.
fn provision_plugins(env: &gripsack_config::EnvConfig) -> Result<(), ExitCode> {
    let store = gripsack_fetch::plugins::PluginStore::new(&gripsack_store::gripsack_home());
    for (name, section) in &env.fetchers {
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
            provision(&store, name, package, "gripfetch")?;
        }
    }
    for (name, section) in &env.linters {
        if let Some(package) = &section.package
            && gripsack_fetch::plugins::parse_ref(package).is_some()
        {
            provision(&store, name, package, "griplint")?;
        }
    }
    Ok(())
}

fn provision(
    store: &gripsack_fetch::plugins::PluginStore,
    name: &str,
    package: &str,
    kind: &str,
) -> Result<(), ExitCode> {
    let before = store.receipt(&format!("{kind}-{name}"));
    let bin = store.ensure(name, package, kind).map_err(|e| {
        eprintln!("grip: cannot provision {kind}-{name} from {package}: {e}");
        ExitCode::FAILURE
    })?;
    let after = store.receipt(&format!("{kind}-{name}"));
    if before != after {
        eprintln!("installed {kind}-{name} from {package} → {}", bin.display());
    }
    Ok(())
}
