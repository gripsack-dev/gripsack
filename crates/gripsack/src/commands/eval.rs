use crate::render::{self, Palette};
use gripsack_ir::{Ir, Severity};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The eval wire protocol (0011 §5): the frontend emits the IR plus
/// any diagnostics (lint results, frontend-side validation). A bare IR
/// document is still accepted — GRIPSACK_PYTHON may point at an older
/// frontend, and the traceback passthrough covers the rest.
#[derive(serde::Deserialize)]
struct EvalEnvelope {
    ir: serde_json::Value,
    #[serde(default)]
    diagnostics: Vec<gripsack_ir::Diagnostic>,
}

/// What a successful eval hands back: the IR JSON plus the env config
/// and the frontend python path — lint registrations resolve their
/// `package =` console scripts against the latter (0012 §move-1).
pub struct EvalOutcome {
    pub ir_json: String,
    pub env: gripsack_config::EnvConfig,
    /// The python the frontend ran under — wheel-form linters resolve
    /// their console script next to it. None under the typescript
    /// frontend (linters there are path= or provisioned store refs).
    pub python: Option<PathBuf>,
}

/// Evaluate an env repo's frontend into IR JSON (0005 §4). The core
/// never embeds a runtime — this is a subprocess.
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
    // Declared plugins (0012 §move-2): package = "owner/repo@tag" on a
    // [fetchers.x] or [linters.x] entry provisions the binary into the
    // plugin store — declarative, sha256-verified, receipted. Fetchers
    // and linters both resolve from the store downstream.
    {
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
                    eprintln!(
                        "grip: [fetchers.{name}] declares both plugin and package — pick one"
                    );
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
    }
    // Build-time env (0001 §3.10 build side): injected for the run's
    // duration so every subprocess — fetchers, build steps, plugins —
    // inherits it. A CLI exits after one run, so process-env is the
    // honest carrier; [eval] env in env.toml is the declaration point.
    for (name, value) in &env.eval.env {
        unsafe { std::env::set_var(name, value) };
    }
    let host = host
        .map(str::to_string)
        .or_else(|| env.env.default_host.clone())
        .unwrap_or_else(crate::commands::hostname);
    let mut frontend_python: Option<PathBuf> = None;
    let mut cmd = if env.env.frontend == gripsack_config::Frontend::Typescript {
        // bun runs the driver's TS/JS directly — no transpile chain.
        // NODE_PATH points at the provisioned @gripsack/core so user
        // modules resolve it from any repo (0005 §1).
        let home = gripsack_store::gripsack_home();
        let ts_dir = match gripsack_exec::ensure_ts_frontend(&home, env!("CARGO_PKG_VERSION")) {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("grip: typescript frontend provisioning failed: {e}");
                return Err(ExitCode::FAILURE);
            }
        };
        let bun = match gripsack_exec::ensure_bun(&home) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("grip: bun provisioning failed: {e}");
                eprintln!("hint: set GRIPSACK_BUN to a bun binary to bypass provisioning");
                return Err(ExitCode::FAILURE);
            }
        };
        let driver = ts_dir.join("node_modules/@gripsack/core/dist/src/cli.js");
        let mut cmd = std::process::Command::new(bun);
        cmd.arg(driver)
            .arg(repo)
            .current_dir(repo)
            .env("NODE_PATH", ts_dir.join("node_modules"));
        cmd
    } else {
        let python = match gripsack_exec::ensure_python(
            &gripsack_store::gripsack_home(),
            &env,
            env!("CARGO_PKG_VERSION"),
        ) {
            Ok(python) => python,
            Err(e) => {
                eprintln!("grip: frontend provisioning failed: {e}");
                let detail = e.to_string();
                if detail.contains("certificate") || detail.contains("UnknownIssuer") {
                    eprintln!(
                        "hint: behind a TLS-intercepting proxy, set SSL_CERT_FILE to the corporate CA bundle"
                    );
                } else if detail.contains("no version") || detail.contains("unsatisfiable") {
                    eprintln!(
                        "hint: a corporate default index (uv.toml) may not mirror gripsack — \
                         set UV_DEFAULT_INDEX=https://pypi.org/simple, or GRIPSACK_PYTHON to a \
                         python with `gripsack` installed to bypass provisioning"
                    );
                } else {
                    eprintln!(
                        "hint: set GRIPSACK_PYTHON to a python with `gripsack` installed to bypass provisioning"
                    );
                }
                return Err(ExitCode::FAILURE);
            }
        };
        frontend_python = Some(python.clone());
        let mut cmd = std::process::Command::new(&python);
        cmd.arg("-m").arg("gripsack").arg(repo).current_dir(repo);
        cmd
    };
    cmd.arg("--host").arg(&host);
    let out = cmd.output().map_err(|e| {
        eprintln!("grip: cannot spawn python3: {e} (see `grip doctor`)");
        ExitCode::FAILURE
    })?;
    let stdout = String::from_utf8(out.stdout).map_err(|_| {
        eprintln!("grip: frontend emitted non-utf8 IR — this is a frontend bug");
        ExitCode::FAILURE
    })?;
    // Envelope (0011 §5): diagnostics render through the same path as
    // the core's own; any error-severity diagnostic fails eval.
    if let Ok(envelope) = serde_json::from_str::<EvalEnvelope>(&stdout) {
        if !envelope.diagnostics.is_empty() {
            eprintln!(
                "{}",
                render::render_diagnostics(&envelope.diagnostics, palette)
            );
        }
        let failed = !out.status.success()
            || envelope
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error);
        if failed {
            return Err(ExitCode::FAILURE);
        }
        return Ok(EvalOutcome {
            ir_json: envelope.ir.to_string(),
            env,
            python: frontend_python,
        });
    }
    if !out.status.success() {
        // Frontend tracebacks are the frontend's domain (0005 §4) —
        // pass them through untouched.
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        eprintln!("grip: frontend eval failed ({host})");
        return Err(ExitCode::FAILURE);
    }
    Ok(EvalOutcome {
        ir_json: stdout,
        env,
        python: frontend_python,
    })
}

/// Run registered linters against the IR's modules (0012 §move-1) and
/// render what comes back; error severity fails the command.
pub fn run_lints(
    ir: &Ir,
    outcome: &EvalOutcome,
    repo: &Path,
    host: Option<&str>,
    palette: Palette,
) -> Result<(), ExitCode> {
    let diagnostics = gripsack_lint::run(
        ir,
        &outcome.env.linters,
        repo,
        host,
        outcome.python.as_deref(),
    );
    if diagnostics.is_empty() {
        return Ok(());
    }
    eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(ExitCode::FAILURE);
    }
    Ok(())
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
