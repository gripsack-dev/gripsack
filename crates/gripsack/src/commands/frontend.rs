//! The frontend invocation context (0013 D2): every path the sandbox
//! needs, bundled so signatures say what they mean — `&Frontend`
//! instead of six positional `&Path`s a reader must count.

use std::path::Path;

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

impl<'a> Frontend<'a> {
    /// The sandboxed spawn (0013 D2): read is the ONLY grant — no
    /// env, no net, no run, no ffi, no sys, denied by the absence of
    /// their flags. `--cached-only --no-remote --no-lock`: nothing
    /// downloads, nothing writes a lockfile; the frontend is embedded
    /// files and relative imports. DENO_DIR points the cache under
    /// $GRIPSACK_HOME — never $HOME, so a sandboxed-HOME run can
    /// neither poison nor depend on the user's deno cache.
    pub(super) fn command(&self, inputs: &Path) -> std::process::Command {
        // the deliberate pin (the repo's own @gripsack/core) may
        // symlink OUTSIDE the repo — `npm install <path>`,
        // monorepos — and deno checks permissions against the
        // canonical path; grant the real location or the sandbox
        // blocks the very pin it must honor
        let mut reads = vec![
            self.repo.to_path_buf(),
            inputs
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            self.frontend_dir.to_path_buf(),
        ];
        // the deliberate pin may enlarge the read set — so the grant
        // is only as strong as the proof: the RESOLVED target must be
        // a @gripsack/core package (0033 R3). Without the check, a
        // repo-planted symlink would extend its own sandbox reads to
        // wherever it points.
        if let Ok(pin) = self.repo.join("node_modules/@gripsack/core").canonicalize()
            && !reads.contains(&pin)
            && pin_is_gripsack_core(&pin)
        {
            reads.push(pin);
        }
        let reads = reads
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(",");
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
            .arg(format!("--allow-read={reads}"))
            .arg(self.driver)
            .arg(self.repo)
            .arg("--inputs")
            .arg(inputs)
            .current_dir(self.repo)
            .env("DENO_DIR", self.home.join("deno-cache"));
        cmd
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
