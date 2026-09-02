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

    println!("      home: {}", home.display());

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
