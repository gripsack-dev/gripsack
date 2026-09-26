//! The frontend invocation context (0013 D2): every path the sandbox
//! needs, bundled so signatures say what they mean — `&Frontend`
//! instead of six positional `&Path`s a reader must count.

use gripsack_ir::Diagnostic;
use gripsack_ir::diagnostic::codes;
use gripsack_process::{self, Control, Limits, Outcome};
use std::io;
use std::path::Path;
use std::time::Duration;

/// One frontend eval's fixed coordinates: the deno binary, the env
/// repo being evaluated, the driver script, the materialized
/// frontend, and gripsack's home (deno cache + inputs live under it).
pub(super) struct Frontend<'a> {
    pub deno: &'a Path,
    pub repo: &'a Path,
    pub driver: &'a Path,
    pub frontend_dir: &'a Path,
    pub home: &'a Path,
}

/// Captured JSON bytes and the supervisor's process/limit verdict.
/// The caller must check `reason` before parsing the bounded output.
pub(super) struct FrontendExecution {
    pub stdout: Vec<u8>,
    pub outcome: Outcome,
}
/// A denied read grant is an authoring/configuration error. Process
/// errors remain separate so the CLI can report the existing spawn hint.
pub(super) enum FrontendRunError {
    Grant(Diagnostic),
    Process(io::Error),
}

impl<'a> Frontend<'a> {
    /// The sandboxed spawn (0013 D2): read is the ONLY grant — no
    /// env, no net, no run, no ffi, no sys, denied by the absence of
    /// their flags. `--cached-only --no-remote --no-lock`: nothing
    /// downloads, nothing writes a lockfile; the frontend is embedded
    /// files and relative imports. DENO_DIR points the cache under
    /// $GRIPSACK_HOME — never $HOME, so a sandboxed-HOME run can
    /// neither poison nor depend on the user's deno cache.
    pub(super) fn command(&self, inputs: &Path) -> Result<std::process::Command, Diagnostic> {
        // the deliberate pin (the repo's own @gripsack/core) may
        // symlink OUTSIDE the repo — `npm install <path>`,
        // monorepos — and deno checks permissions against the
        // canonical path; grant the real location or the sandbox
        // blocks the very pin it must honor
        let mut reads = Vec::with_capacity(4);
        for (kind, path) in [
            ("repository", self.repo),
            (
                "host inputs",
                inputs.parent().unwrap_or_else(|| Path::new(".")),
            ),
            ("embedded frontend", self.frontend_dir),
        ] {
            admit_read_grant(kind, path)?;
            reads.push(path);
        }
        // the deliberate pin may enlarge the read set — so the grant
        // is only as strong as the proof: the RESOLVED target must be
        // a @gripsack/core package (0033 R3). Without the check, a
        // repo-planted symlink would extend its own sandbox reads to
        // wherever it points.
        let pin = self
            .repo
            .join("node_modules/@gripsack/core")
            .canonicalize()
            .ok();
        if let Some(pin) = pin.as_deref() {
            admit_read_grant("pinned @gripsack/core package", pin)?;
            if !reads.contains(&pin) && pin_is_gripsack_core(pin) {
                reads.push(pin);
            }
        }
        let mut read_grants = String::new();
        for path in reads {
            if !read_grants.is_empty() {
                read_grants.push(',');
            }
            read_grants.push_str(&path.to_string_lossy());
        }
        // --import-map, NOT deno.json discovery: a discovered deno.json
        // puts deno in project mode where BYONM (the repo's npm-managed
        // node_modules) never engages — third-party bare imports in module
        // code would fail. The flag applies the pin map without creating a
        // project, so env repos get BOTH the deliberate-pin rule and npm
        // dependencies (documented: install them in the repo, they're
        // read-only under the sandbox).
        let mut cmd = std::process::Command::new(self.deno);
        cmd.args(["run", "--no-remote", "--cached-only", "--no-lock"])
            .arg(format!(
                "--import-map={}",
                self.frontend_dir.join("deno.json").display()
            ))
            .arg(format!("--allow-read={read_grants}"))
            .arg(self.driver)
            .arg(self.repo)
            .arg("--inputs")
            .arg(inputs)
            .current_dir(self.repo)
            .env("DENO_DIR", self.home.join("deno-cache"));
        Ok(cmd)
    }

    /// One supervised frontend invocation. The emitted envelope is one
    /// JSON line; use the stdout ceiling as its line ceiling rather than
    /// the supervisor's smaller line-oriented plugin default. Rejoin
    /// framed lines so pretty-printed envelopes remain parseable.
    pub(super) fn run_bounded(
        &self,
        inputs: &Path,
        timeout: Duration,
    ) -> Result<FrontendExecution, FrontendRunError> {
        let mut limits = Limits {
            timeout,
            ..Limits::default()
        };
        limits.line_bytes = limits.stdout_bytes.try_into().map_err(|_| {
            FrontendRunError::Process(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frontend stdout limit exceeds addressable memory",
            ))
        })?;
        let mut stdout = Vec::new();
        let mut first_line = true;
        let mut command = self.command(inputs).map_err(FrontendRunError::Grant)?;
        let outcome = gripsack_process::run(&mut command, &[], limits, |line| {
            if !first_line {
                stdout.push(b'\n');
            }
            first_line = false;
            stdout.extend_from_slice(line);
            Control::Continue
        })
        .map_err(FrontendRunError::Process)?;
        Ok(FrontendExecution { stdout, outcome })
    }
}

/// The resolved pin target must prove it IS @gripsack/core: a
/// package.json with that name. Anything else (a bare directory, a
/// symlink to arbitrary outside content) earns no read grant.
fn pin_is_gripsack_core(pin: &Path) -> bool {
    std::fs::read_to_string(pin.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.get("name")?.as_str().map(str::to_string))
        .is_some_and(|name| name == "@gripsack/core")
}

/// Deno interprets a comma inside --allow-read as another permission
/// entry, even when the path is canonical and belongs to a valid pin.
/// Validate *every* grant path before formatting the single CLI flag.
fn admit_read_grant(kind: &str, path: &Path) -> Result<(), Diagnostic> {
    if !path.as_os_str().as_encoded_bytes().contains(&b',') {
        return Ok(());
    }
    Err(Diagnostic::error(
        codes::UNSAFE_EVAL_READ_GRANT,
        format!(
            "{kind} path {} cannot be a frontend read grant: ',' splits Deno permission lists",
            path.to_string_lossy().escape_debug()
        ),
    )
    .with_help("move the repo, frontend home or pinned package to a path without commas"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_read_grant_rejects_a_comma_before_deno_spawn() {
        let root = tempfile::tempdir().unwrap();
        let safe_repo = root.path().join("repo");
        std::fs::create_dir(&safe_repo).unwrap();
        let safe_inputs = root.path().join("inputs/facts.json");
        let safe_frontend = root.path().join("frontend");
        let comma_path = root.path().join("bad,grant");

        for kind in ["repository", "host inputs", "embedded frontend"] {
            let repo = if kind == "repository" {
                comma_path.as_path()
            } else {
                safe_repo.as_path()
            };
            let inputs = if kind == "host inputs" {
                comma_path.join("facts.json")
            } else {
                safe_inputs.clone()
            };
            let frontend_dir = if kind == "embedded frontend" {
                comma_path.as_path()
            } else {
                safe_frontend.as_path()
            };
            let frontend = Frontend {
                deno: Path::new("deno"),
                repo,
                driver: Path::new("driver.ts"),
                frontend_dir,
                home: root.path(),
            };
            let diagnostic = frontend.command(&inputs).unwrap_err();
            assert_eq!(diagnostic.code, codes::UNSAFE_EVAL_READ_GRANT);
            assert!(diagnostic.message.contains(kind), "{diagnostic:?}");
        }
    }
}
