use crate::render::{self, Palette};
use gripsack_ir::{Diagnostic, Ir, Severity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// E112 (0013 D6): probe binding must reach a fixpoint within the
/// round cap — a probe whose request depends on another probe's
/// result is an authoring error, not a fixpoint.
const PROBE_UNSTABLE: &str = "E112";
/// E113: the frontend requested a probe this grip cannot answer —
/// the kind enum is closed on purpose (core-side binding is what
/// makes probes inspectable).
const PROBE_UNSUPPORTED: &str = "E113";

/// Two-stage eval's round cap (0013 D6): eval → bind → re-eval, at
/// most this many frontend runs before the set is declared unstable.
const PROBE_ROUNDS: usize = 4;

/// The eval wire protocol (0011 §5, 0013 D6): the frontend emits the
/// IR plus any diagnostics (lint results, frontend-side validation)
/// and its symbolic probe requests — the sandbox cannot run probes,
/// so `ctx.probe.*` can only ever record a request here.
#[derive(serde::Deserialize)]
struct EvalEnvelope {
    ir: serde_json::Value,
    #[serde(default)]
    diagnostics: Vec<gripsack_ir::Diagnostic>,
    #[serde(default)]
    probe_requests: Vec<ProbeRequest>,
}

/// One symbolic probe request: the effect the frontend wants, the
/// core's to answer (executable: PATH lookup; file_exists:
/// absolute-path stat).
#[derive(Debug, Clone, serde::Deserialize)]
struct ProbeRequest {
    kind: String,
    /// `executable`'s argument: a bare name to look up on PATH.
    name: Option<String>,
    /// `file_exists`'s argument: an absolute path to stat.
    path: Option<String>,
    span: Option<gripsack_ir::Span>,
}

impl ProbeRequest {
    fn arg(&self) -> &str {
        self.name.as_deref().or(self.path.as_deref()).unwrap_or("")
    }
    /// The probes-map key: "executable:nvidia-smi".
    fn key(&self) -> String {
        format!("{}:{}", self.kind, self.arg())
    }
}

/// The inputs envelope (0013 D4): everything the frontend may observe
/// about the host, detected here in the core and injected via file —
/// never argv (world-visible in `ps`), never env (leaks to children).
#[derive(serde::Serialize)]
struct InputsEnvelope<'a> {
    version: u32,
    host: &'a str,
    facts: &'a gripsack_exec::facts::HostFacts,
    /// CLI --tags ∪ host-entrypoint tags. Empty for now: no --tags
    /// flag exists yet, and entrypoint tags are produced by the
    /// entrypoint itself — the core has nothing to add at stage 1.
    tags: &'a [String],
    /// "kind:arg" → bound value, from the previous round. Stage 1
    /// runs with it empty; probe calls return false until bound.
    probes: &'a BTreeMap<String, bool>,
    settings: &'a serde_json::Map<String, serde_json::Value>,
}

/// The inputs file: JSON under `$GRIPSACK_HOME/inputs/`, removed when
/// the guard drops — the run log keeps the facts and probe bindings
/// it carried, so a deleted file loses nothing.
struct InputsFile {
    path: PathBuf,
}

impl InputsFile {
    fn create(home: &Path) -> std::io::Result<Self> {
        let dir = home.join("inputs");
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join(format!("{}.json", gripsack_trace::new_run_id())),
        })
    }

    fn write(&self, envelope: &InputsEnvelope<'_>) -> Result<(), ExitCode> {
        let json = serde_json::to_string(envelope).map_err(|e| {
            eprintln!("grip: cannot serialize host inputs: {e}");
            ExitCode::FAILURE
        })?;
        gripsack_store::atomic_write(&self.path, json.as_bytes()).map_err(|e| {
            eprintln!("grip: cannot write {}: {e}", self.path.display());
            ExitCode::FAILURE
        })
    }
}

impl Drop for InputsFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
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
    // (The deno eval subprocess is the exception — it gets no env:
    // denied by the absence of --allow-env.)
    for (name, value) in &env.eval.env {
        unsafe { std::env::set_var(name, value) };
    }
    let host = host
        .map(str::to_string)
        .or_else(|| env.env.default_host.clone())
        .unwrap_or_else(crate::commands::hostname);
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
    let empty_settings = serde_json::Map::new();
    let no_tags: [String; 0] = [];
    let mut bound: BTreeMap<String, bool> = BTreeMap::new();
    let mut envelope: Option<EvalEnvelope> = None;

    for round in 1..=PROBE_ROUNDS {
        inputs.write(&InputsEnvelope {
            version: 1,
            host: &host,
            facts,
            tags: &no_tags,
            probes: &bound,
            settings: &empty_settings,
        })?;
        tracing::info!(round, probes_bound = bound.len(), "frontend eval");
        let out = deno_command(&deno, repo, &driver, &inputs.path, &frontend_dir, &home)
            .output()
            .map_err(|e| {
                eprintln!("grip: cannot spawn deno: {e} (see `grip doctor`)");
                ExitCode::FAILURE
            })?;
        let stdout = String::from_utf8(out.stdout.clone()).map_err(|_| {
            eprintln!("grip: frontend emitted non-utf8 output — this is a frontend bug");
            ExitCode::FAILURE
        })?;
        let parsed = match serde_json::from_str::<EvalEnvelope>(&stdout) {
            Ok(envelope) => envelope,
            Err(_) => {
                // frontend errors are the frontend's domain (0005 §4) —
                // pass the stderr through untouched.
                if !out.status.success() {
                    eprint!("{}", String::from_utf8_lossy(&out.stderr));
                    eprintln!("grip: frontend eval failed ({host})");
                } else {
                    eprintln!(
                        "grip: frontend emitted a malformed envelope — this is a frontend bug"
                    );
                    eprintln!("hint: stdout was {} bytes, not JSON", stdout.len());
                }
                return Err(ExitCode::FAILURE);
            }
        };
        let failed = !out.status.success()
            || parsed
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error);
        if failed {
            if !parsed.diagnostics.is_empty() {
                eprintln!(
                    "{}",
                    render::render_diagnostics(&parsed.diagnostics, palette)
                );
            }
            if !out.status.success() {
                eprint!("{}", String::from_utf8_lossy(&out.stderr));
                eprintln!("grip: frontend eval failed ({host})");
            }
            return Err(ExitCode::FAILURE);
        }
        // requests already bound are answered by the inputs file —
        // only NEW kinds/args force another round (0013 D6)
        let mut seen = std::collections::BTreeSet::new();
        let fresh: Vec<ProbeRequest> = parsed
            .probe_requests
            .iter()
            .filter(|req| !bound.contains_key(&req.key()))
            .filter(|req| seen.insert(req.key()))
            .cloned()
            .collect();
        envelope = Some(parsed);
        if fresh.is_empty() {
            break;
        }
        if round == PROBE_ROUNDS {
            let names = fresh
                .iter()
                .map(ProbeRequest::key)
                .collect::<Vec<_>>()
                .join(", ");
            let diagnostic = Diagnostic::error(
                PROBE_UNSTABLE,
                format!(
                    "probe set unstable: new probe requests still appearing after {PROBE_ROUNDS} eval rounds ({names})"
                ),
            )
            .with_help(
                "a probe depending on a probe is an authoring error — call ctx.probe.* \
                 unconditionally, not behind another probe's result",
            );
            tracing::error!(code = PROBE_UNSTABLE, "{names}");
            eprintln!("{}", render::render_diagnostics(&[diagnostic], palette));
            return Err(ExitCode::FAILURE);
        }
        for req in &fresh {
            match bind_probe(req) {
                Ok(value) => {
                    tracing::info!(probe = %req.key(), result = value, "probe bound");
                    bound.insert(req.key(), value);
                }
                Err(message) => {
                    let diagnostic = Diagnostic::error(PROBE_UNSUPPORTED, message)
                        .with_label(req.span.clone(), "probe requested here");
                    tracing::error!(code = PROBE_UNSUPPORTED, "{}", req.key());
                    eprintln!("{}", render::render_diagnostics(&[diagnostic], palette));
                    return Err(ExitCode::FAILURE);
                }
            }
        }
    }
    let envelope = envelope.expect("the loop runs at least once");
    if !envelope.diagnostics.is_empty() {
        eprintln!(
            "{}",
            render::render_diagnostics(&envelope.diagnostics, palette)
        );
    }
    Ok(EvalOutcome {
        ir_json: envelope.ir.to_string(),
        env,
        host_inputs: HostInputs {
            facts: facts.clone(),
            probes: bound,
        },
    })
}

/// The sandboxed spawn (0013 D2): read is the ONLY grant — no env, no
/// net, no run, no ffi, no sys, denied by the absence of their flags.
/// `--cached-only --no-remote --no-lock`: nothing downloads, nothing
/// writes a lockfile; the frontend is embedded files and relative
/// imports. DENO_DIR points the cache under $GRIPSACK_HOME — never
/// $HOME, so a sandboxed-HOME run can neither poison nor depend on
/// the user's deno cache.
fn deno_command(
    deno: &Path,
    repo: &Path,
    driver: &Path,
    inputs: &Path,
    frontend_dir: &Path,
    home: &Path,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(deno);
    cmd.args(["run", "--no-remote", "--cached-only", "--no-lock"])
        .arg(format!(
            "--allow-read={},{},{}",
            repo.display(),
            inputs.parent().unwrap_or_else(|| Path::new(".")).display(),
            frontend_dir.display(),
        ))
        .arg(driver)
        .arg(repo)
        .arg("--inputs")
        .arg(inputs)
        .current_dir(repo)
        .env("DENO_DIR", home.join("deno-cache"));
    cmd
}

/// Bind one probe request (0013 D6): the closed enum lives in the
/// core on purpose — binding here, on the frontend's side of the
/// boundary with core-supplied data, is what keeps the emitted IR
/// fully concrete.
fn bind_probe(req: &ProbeRequest) -> Result<bool, String> {
    match req.kind.as_str() {
        "executable" => {
            let name = req
                .name
                .as_deref()
                .filter(|n| !n.is_empty() && !n.contains('/'))
                .ok_or_else(|| {
                    format!(
                        "executable probe needs a bare name to look up on PATH, got {:?}",
                        req.arg()
                    )
                })?;
            Ok(executable_on_path(name))
        }
        "file_exists" => {
            let path = req
                .path
                .as_deref()
                .or(req.name.as_deref())
                .filter(|p| !p.is_empty())
                .ok_or_else(|| "file_exists probe needs a path".to_string())?;
            if !Path::new(path).is_absolute() {
                // relative would resolve against the core's CWD — a
                // silent lie about which file was meant
                tracing::warn!(probe = %req.key(), "file_exists probe is not absolute — bound false");
                return Ok(false);
            }
            Ok(Path::new(path).exists())
        }
        other => Err(format!(
            "unsupported probe kind {other:?} (this grip answers: executable, file_exists)"
        )),
    }
}

fn executable_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| is_executable_file(&dir.join(name))))
        .unwrap_or(false)
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
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
