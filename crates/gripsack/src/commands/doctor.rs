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
    match gripsack_exec::ensure_deno(&home) {
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

    // The repo's own @gripsack/core pin (the deliberate-pin rule:
    // node_modules shadows the embedded copy when present). It does
    // not change what RUNS — but it is what the editor and tsc
    // typecheck against, and a stale pin happily accepts authoring
    // styles the current frontend removed (migration report 0.18.1:
    // a ^0.17.5 pin typechecked a call the 0.18.x DSL rejects).
    let repo = std::env::current_dir().unwrap_or_else(|_| ".".into());
    if let Some(pin) = core_pin(&repo) {
        let embedded = env!("CARGO_PKG_VERSION");
        if pin_is_behind(&pin, embedded) {
            println!(
                "{}  repo pin: package.json pins @gripsack/core {pin}; the embedded \
                 frontend is {embedded} — your editor typechecks against the older \
                 types (npm i -D @gripsack/core@^{embedded})",
                mark(true).replace('\u{2713}', "!")
            );
        } else {
            println!("      repo pin: @gripsack/core {pin} (matches the embedded {embedded})");
        }
    }
    println!("      home: {}", home.display());

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
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
    let parse = |v: &str| -> Option<(u64, u64)> {
        let digits: String = v
            .trim_start_matches(['^', '~', '>', '=', ' '])
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let mut it = digits.split('.').map(str::parse::<u64>);
        match (it.next(), it.next()) {
            (Some(Ok(a)), Some(Ok(b))) => Some((a, b)),
            _ => None,
        }
    };
    match (parse(spec), parse(embedded)) {
        (Some(pin), Some(emb)) => pin < emb,
        // unparseable spec (git URL, workspace:) — don't guess
        (Some(_), None) | (None, _) => false,
    }
}
