//! The probe fixpoint (0013 D6): eval → bind → re-eval.
//!
//! The sandboxed frontend cannot run probes — `ctx.probe.*` records a
//! request, the core binds it, and the next round sees the answer.
//! New requests each round force another eval, capped at
//! [`PROBE_ROUNDS`]; a set that never settles is an authoring error.

use super::eval::EvalEnvelope;
use crate::render::{self, Palette};
use gripsack_ir::diagnostic::codes;
use gripsack_ir::{Diagnostic, Severity};
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

/// Two-stage eval's round cap (0013 D6): eval → bind → re-eval, at
/// most this many frontend runs before the set is declared unstable.
pub(super) const PROBE_ROUNDS: usize = 4;

/// One symbolic probe request: the effect the frontend wants, the
/// core's to answer (executable: PATH lookup; file_exists:
/// absolute-path stat).
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct ProbeRequest {
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
    pub(super) fn key(&self) -> String {
        format!("{}:{}", self.kind, self.arg())
    }
}

/// The inputs envelope (0013 D4): everything the frontend may observe
/// about the host, detected here in the core and injected via file —
/// never argv (world-visible in `ps`), never env (leaks to children).
#[derive(serde::Serialize)]
pub(super) struct InputsEnvelope<'a> {
    pub version: u32,
    pub host: &'a str,
    pub facts: &'a gripsack_exec::facts::HostFacts,
    /// CLI --tags ∪ host-entrypoint tags. Empty for now: no --tags
    /// flag exists yet, and entrypoint tags are produced by the
    /// entrypoint itself — the core has nothing to add at stage 1.
    pub tags: &'a [String],
    /// "kind:arg" → bound value, from the previous round. Stage 1
    /// runs with it empty; probe calls return false until bound.
    pub probes: &'a BTreeMap<String, bool>,
    pub settings: &'a serde_json::Map<String, serde_json::Value>,
}

/// The inputs file: JSON under `$GRIPSACK_HOME/inputs/`, removed when
/// the guard drops — the run log keeps the facts and probe bindings
/// it carried, so a deleted file loses nothing.
pub(super) struct InputsFile {
    path: PathBuf,
}

impl InputsFile {
    pub(super) fn create(home: &Path) -> std::io::Result<Self> {
        let dir = home.join("inputs");
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join(format!("{}.json", gripsack_trace::new_run_id())),
        })
    }

    pub(super) fn write(&self, envelope: &InputsEnvelope<'_>) -> Result<(), ExitCode> {
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

/// Run eval rounds until the probe set settles: returns the final
/// envelope and the probe bindings it ran with. Rendering and exit
/// semantics stay here (this IS the eval command's error surface);
/// the caller only orchestrates.
pub(super) fn eval_to_fixpoint(
    frontend: &super::frontend::Frontend<'_>,
    host: &str,
    facts: &gripsack_exec::facts::HostFacts,
    inputs: &InputsFile,
    palette: Palette,
) -> Result<(EvalEnvelope, BTreeMap<String, bool>), ExitCode> {
    let empty_settings = serde_json::Map::new();
    let no_tags: [String; 0] = [];
    let mut bound: BTreeMap<String, bool> = BTreeMap::new();
    let mut envelope: Option<EvalEnvelope> = None;

    for round in 1..=PROBE_ROUNDS {
        inputs.write(&InputsEnvelope {
            version: 1,
            host,
            facts,
            tags: &no_tags,
            probes: &bound,
            settings: &empty_settings,
        })?;
        tracing::info!(round, probes_bound = bound.len(), "frontend eval");
        let out = frontend.command(&inputs.path).output().map_err(|e| {
            eprintln!("grip: cannot spawn deno: {e} (see `grip doctor`)");
            ExitCode::FAILURE
        })?;
        let stdout = String::from_utf8(out.stdout).map_err(|_| {
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
                codes::PROBE_UNSTABLE,
                format!(
                    "probe set unstable: new probe requests still appearing after {PROBE_ROUNDS} eval rounds ({names})"
                ),
            )
            .with_help(
                "a probe depending on a probe is an authoring error — call ctx.probe.* \
                 unconditionally, not behind another probe's result",
            );
            tracing::error!(code = codes::PROBE_UNSTABLE, "{names}");
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
                    let diagnostic = Diagnostic::error(codes::PROBE_UNSUPPORTED, message)
                        .with_label(req.span.clone(), "probe requested here");
                    tracing::error!(code = codes::PROBE_UNSUPPORTED, "{}", req.key());
                    eprintln!("{}", render::render_diagnostics(&[diagnostic], palette));
                    return Err(ExitCode::FAILURE);
                }
            }
        }
    }
    envelope.map(|e| (e, bound)).ok_or_else(|| {
        eprintln!("grip: frontend produced no envelope — this is a frontend bug");
        ExitCode::FAILURE
    })
}

/// Bind one probe request against this host (0013 D6): the core's
/// answer, never the sandbox's.
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
