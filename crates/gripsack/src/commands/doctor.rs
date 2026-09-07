//! grip doctor — the eval runtime check (0003 §8, plan/0013 D2).

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

    let home = gripsack_store::gripsack_home();

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
                     only the eval sandbox needs a glibc/macOS host",
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
        Ok(Some(dir)) => println!(
            "{}  frontend: embedded TypeScript {} (materialized at {})",
            mark(true),
            env!("CARGO_PKG_VERSION"),
            dir.display()
        ),
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
    let embedded = env!("CARGO_PKG_VERSION");
    let minor_line = embedded.rsplit_once('.').map_or(embedded, |(line, _)| line);
    match installed_core(&repo) {
        Ok(Some(version)) if pin_is_behind(&version, embedded) => {
            println!(
                "{}  repo frontend: installed @gripsack/core {version} shadows embedded {embedded}; \
                 update it (npm i -D @gripsack/core@^{minor_line}.0), or remove \
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
            if let Some(pin) = core_pin(&repo) {
                if pin_is_behind(&pin, embedded) {
                    println!(
                        "{}  repo pin: package.json declares @gripsack/core {pin}, no installed copy; \
                         eval uses embedded {embedded} — update the declaration \
                         (npm i -D @gripsack/core@^{minor_line}.0) or remove the pin",
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

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
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
