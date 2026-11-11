//! grip doctor — the eval runtime check (0003 §8, plan/0013 D2), and the
//! pending journal when an interrupted run left one (0058 item 1).

use crate::render::Palette;
use std::process::ExitCode;

/// The eval contract (plan/0013 D2): a runnable deno — the exact
/// precedence eval uses (`GRIPSACK_DENO`, the pinned runtime, a PATH
/// deno only as a loud last resort).
pub fn doctor(palette: Palette) -> ExitCode {
    let mut ok = true;
    let mark = |good: bool| {
        if good {
            palette.good("ok  ")
        } else {
            palette.error("MISS")
        }
    };

    let home = gripsack_host::store::gripsack_home();

    // deno is the eval runtime — required, no fallback exists.
    match gripsack_exec::ensure_deno(&home, &gripsack_fetch::FetchContext::default()) {
        Ok(deno) => {
            let version = std::process::Command::new(&deno)
                .arg("--version")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .next()
                        .unwrap_or("deno (unknown version)")
                        .to_string()
                })
                .unwrap_or_else(|| "deno (unknown version)".into());
            let source = if std::env::var_os("GRIPSACK_DENO").is_some() {
                "GRIPSACK_DENO"
            } else if deno.as_os_str() == "deno" {
                // last-resort fallback — the pinned runtime was
                // unavailable (musl host or failed download)
                "on PATH — pinned unavailable"
            } else {
                "provisioned (pinned)"
            };
            println!("{}  deno: {version} ({source})", mark(true));
        }
        Err(e) => {
            println!("{}  deno: {e}", mark(false));
            let reason = e.to_string();
            if reason.contains("musl") {
                println!(
                    "      {} the grip binary itself is musl-static and keeps working — \
                     only the eval sandbox needs a Linux/WSL x86_64 glibc host",
                    palette.warn("hint:")
                );
            } else {
                println!(
                    "      {} set GRIPSACK_DENO to a deno binary to bypass provisioning",
                    palette.warn("hint:")
                );
            }
            ok = false;
        }
    }

    // The frontend source embedded in this binary (plan/0013 D3) —
    // materializing here is idempotent; a MISS means a build without
    // the repo's typescript tree (crates.io builds).
    match gripsack_exec::ensure_ts_frontend(&home, env!("CARGO_PKG_VERSION")) {
        Ok(Some(dir)) => {
            println!(
                "{}  frontend: embedded TypeScript {} (materialized at {})",
                mark(true),
                env!("CARGO_PKG_VERSION"),
                dir.display()
            );
            // 0045: the stable link + the src-pointing package.json make
            // the embedded frontend resolvable by any tsc-based editor
            let current = dir.parent().unwrap_or(&dir).join("current");
            println!(
                "      {} symlink node_modules/@gripsack/core → {} — or tsconfig \
                 \"paths\": {{\"@gripsack/core\": [\"{}\"]}}",
                palette.warn("editor:"),
                current.display(),
                current.join("src/index.ts").display(),
            );
        }
        Ok(None) => {
            println!(
                "{}  frontend: this build carries no embedded TypeScript frontend",
                mark(false)
            );
            ok = false;
        }
        Err(e) => {
            println!("{}  frontend: materialization failed: {e}", mark(false));
            ok = false;
        }
    }

    // Eval only considers the repo-root install (typescript/src/pin.ts).
    // A declaration is not evidence that the installed frontend was upgraded.
    let repo = std::env::current_dir().unwrap_or_else(|_| ".".into());
    if repo.join("gripsack.ts").is_file() {
        println!(
            "{}  workspace: gripsack.ts — run `grip check` to list named outputs",
            mark(true)
        );
    } else {
        println!(
            "      workspace: no gripsack.ts in this directory — run `grip init` to create one"
        );
    }
    let editor_only = if repo.join("env.toml").is_file() {
        match gripsack_config::load_env(&repo.join("env.toml")) {
            Ok(config) => {
                match gripsack_exec::source_bundle::SourceCapturePolicy::new(config.capture.exclude)
                {
                    Ok(policy) => {
                        policy.excludes(std::path::Path::new("node_modules/@gripsack/core"))
                    }
                    Err(error) => {
                        println!("{}  capture policy: {error}", mark(false));
                        ok = false;
                        false
                    }
                }
            }
            Err(diagnostics) => {
                eprintln!(
                    "{}",
                    crate::render::render_diagnostics_bounded(&diagnostics, palette, &repo)
                );
                ok = false;
                false
            }
        }
    } else {
        false
    };
    let embedded = env!("CARGO_PKG_VERSION");
    match if editor_only {
        Ok(None)
    } else {
        installed_core(&repo)
    } {
        Ok(Some(version)) if pin_is_behind(&version, embedded) => {
            println!(
                "{}  repo frontend: installed @gripsack/core {version} shadows embedded {embedded}; \
                 update it (npm i -D https://github.com/gripsack-dev/gripsack/releases/download/core-v{embedded}/gripsack-core-{embedded}.tgz), or remove \
                 node_modules/@gripsack/core to use the embedded frontend",
                mark(false)
            );
            ok = false;
        }
        Ok(Some(version)) => println!(
            "{}  repo frontend: installed @gripsack/core {version} (eval uses this copy)",
            mark(true)
        ),
        Err(error) => {
            println!(
                "{}  repo frontend: {error}; fix or remove node_modules/@gripsack/core",
                mark(false)
            );
            ok = false;
        }
        Ok(None) => {
            if editor_only {
                println!(
                    "{}  repo frontend: editor dependencies excluded from capture; eval uses embedded {embedded}",
                    mark(true)
                );
            }
            if let Some(pin) = core_pin(&repo) {
                if pin_is_behind(&pin, embedded) {
                    println!(
                        "{}  repo pin: package.json declares @gripsack/core {pin}, no installed copy; \
                         eval uses embedded {embedded} — update the declaration \
                         (npm i -D https://github.com/gripsack-dev/gripsack/releases/download/core-v{embedded}/gripsack-core-{embedded}.tgz) or remove the pin",
                        palette.warn("warn")
                    );
                } else {
                    println!(
                        "      repo pin: declared {pin}, no installed copy; eval uses embedded {embedded}"
                    );
                }
            }
        }
    }
    println!("      home: {}", home.display());
    ok &= journal(palette, &home);

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The pending journal, read-only (`journal::inspect`): what recovery will
/// do with each entry before the next apply, rollback or gc — and, when it
/// refuses one, which entry, what it expected and found, and the repair.
/// `false` when recovery cannot proceed on its own.
fn journal(palette: Palette, home: &std::path::Path) -> bool {
    use gripsack_host::store::journal::{Decision, EntryOutlook, RunVerdict};
    let capability = match gripsack_host::fs::open(home) {
        Ok(capability) => capability,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(error) => {
            println!(
                "{}  journal: cannot open the home: {error}",
                palette.error("MISS")
            );
            return false;
        }
    };
    let view = match gripsack_host::store::journal::inspect(&capability, home) {
        Ok(None) => {
            println!("{}  journal: nothing pending", palette.good("ok  "));
            return true;
        }
        Ok(Some(view)) => view,
        Err(error) => {
            println!(
                "{}  journal: unreadable ({error}); recovery will refuse — inspect {}",
                palette.error("MISS"),
                home.join("journal").display()
            );
            return false;
        }
    };
    let blocked = view.blocked().is_some()
        || view
            .verdict
            .is_none_or(|verdict| verdict == RunVerdict::Ambiguous)
        || !view.unadmitted.is_empty()
        || view.pending.quarantined > 0;
    println!(
        "{}  journal: an interrupted run left recovery pending ({}); it runs before the next \
         apply, rollback or gc",
        if blocked {
            palette.error("MISS")
        } else {
            palette.warn("warn")
        },
        view.pending
    );
    let run = match view.verdict {
        Some(RunVerdict::Unmarked) => "no run marker: every entry is restored to its prior".into(),
        Some(RunVerdict::Uncommitted) => {
            "uncommitted (the selection never flipped): every entry is restored to its prior".into()
        }
        Some(RunVerdict::Committed) => {
            "committed (the selection flipped): the journal is only cleared, content stands".into()
        }
        Some(RunVerdict::Ambiguous) => palette.error(
            "AMBIGUOUS: `current` matches neither the run's target nor its predecessor — \
             recovery refuses and changes nothing",
        ),
        None => palette.error("the run marker cannot be read — recovery refuses"),
    };
    println!("      run: {run}");
    if view.directories > 0 {
        println!(
            "      {} directory transition record(s), recovered by the directory domain",
            view.directories
        );
    }
    for entry in &view.entries {
        let outlook = match &entry.outlook {
            EntryOutlook::Committed => "committed: the deployed content stands".to_string(),
            EntryOutlook::Decided(Decision::Restore) => "the prior will be restored".to_string(),
            EntryOutlook::Decided(Decision::Unchanged) => {
                "the prior is already in place (it will be sealed)".to_string()
            }
            EntryOutlook::Decided(Decision::Preserve) => {
                "changed since the interrupted run — your edit will be kept".to_string()
            }
            EntryOutlook::Decided(Decision::Refuse) => palette.error(&format!(
                "BLOCKED: found {}, which the journal cannot identify",
                entry.found
            )),
            EntryOutlook::Unreadable(error) => palette.error(&format!(
                "cannot be evaluated ({error}); recovery will stop here"
            )),
        };
        println!("      {}: {outlook}", entry.dest);
        if entry.blocks() {
            println!("        expected: {}", entry.expected);
            println!(
                "        {} move {} aside (it is yours; nothing is deleted), then run `grip apply` \
                 again: recovery finishes first — see docs/operations/storage.md",
                palette.warn("fix:"),
                entry.dest
            );
        }
    }
    for rejected in &view.unadmitted {
        println!(
            "      {} {rejected} (it will be quarantined; recovery refuses until it is inspected)",
            palette.error("unadmissible:")
        );
    }
    if view.pending.quarantined > 0 {
        println!(
            "      {} inspect and remove {} to let recovery run",
            palette.warn("fix:"),
            home.join("journal/quarantine").display()
        );
    }
    !blocked
}

fn installed_core(repo: &std::path::Path) -> Result<Option<String>, String> {
    let dir = repo.join("node_modules/@gripsack/core");
    let text = match std::fs::read_to_string(dir.join("package.json")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read installed package.json: {e}")),
    };
    let pkg: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid installed package.json: {e}"))?;
    let version = pkg
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|v| version_line(v).is_some())
        .ok_or("installed package.json has no valid version")?;
    let dot = pkg.get("exports").and_then(|v| v.get("."));
    let entry = dot
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            dot.and_then(|v| v.get("import").or_else(|| v.get("default")))
                .and_then(serde_json::Value::as_str)
        })
        .or_else(|| pkg.get("main").and_then(serde_json::Value::as_str))
        .unwrap_or("index.js");
    if !dir.join(entry).is_file() {
        return Err(format!(
            "installed @gripsack/core {version} entry {entry} is missing"
        ));
    }
    Ok(Some(version.to_string()))
}

/// The repo's `@gripsack/core` version spec from package.json
/// (dependencies or devDependencies), verbatim — `^0.17.5`, `~0.18`,
/// a URL, anything. None when the repo declares no pin or has no
/// package.json (dotfiles-only setups).
fn core_pin(repo: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(repo.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    for section in ["dependencies", "devDependencies"] {
        if let Some(s) = json
            .get(section)
            .and_then(|d| d.get("@gripsack/core"))
            .and_then(serde_json::Value::as_str)
        {
            return Some(s.to_string());
        }
    }
    None
}

/// Is a version spec meaningfully behind the embedded version? Only
/// major.minor counts — `^0.18.0` against embedded `0.18.1` is the
/// normal npm reality (patch releases don't always publish); a
/// `^0.17.x` pin against `0.18.x` is the drift that matters.
fn pin_is_behind(spec: &str, embedded: &str) -> bool {
    let parse = |v: &str| version_line(v.trim_start_matches(['^', '~', '>', '=', ' ']));
    match (parse(spec), parse(embedded)) {
        (Some(pin), Some(emb)) => pin < emb,
        // unparseable spec (git URL, workspace:) — don't guess
        (Some(_), None) | (None, _) => false,
    }
}

fn version_line(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}
