//! grip doctor — the frontend environment check (0003 §8).

use crate::render::Palette;
use owo_colors::OwoColorize;
use std::process::ExitCode;

/// The frontend contract (plan/0003 §8): a Python with the `gripsack`
/// package importable; node for TypeScript repos (0005 §1).
pub fn doctor(palette: Palette) -> ExitCode {
    let mut ok = true;
    let colored = palette.enabled;
    let mark = |good: bool| {
        if !colored {
            return if good { "ok  " } else { "MISS" }.to_string();
        }
        if good {
            "ok  ".green().bold().to_string()
        } else {
            "MISS".red().bold().to_string()
        }
    };

    // The same python eval would use (0005 §3): GRIPSACK_PYTHON wins.
    let python = std::env::var("GRIPSACK_PYTHON").unwrap_or_else(|_| "python3".into());
    match std::process::Command::new(&python)
        .arg("--version")
        .output()
    {
        Ok(out) if out.status.success() => {
            let v = String::from_utf8_lossy(&out.stdout);
            let v = v.trim();
            let v = if v.is_empty() {
                String::from_utf8_lossy(&out.stderr).trim().to_string()
            } else {
                v.to_string()
            };
            println!("{}  python: {v}", mark(true));
        }
        _ => {
            println!("{}  python: `{python}` not runnable", mark(false));
            ok = false;
        }
    }

    let check = std::process::Command::new(&python)
        .args(["-c", "import gripsack; print(gripsack.__version__)"])
        .output();
    match check {
        Ok(out) if out.status.success() => {
            let frontend_v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            println!("{}  frontend: gripsack python {frontend_v}", mark(true));
            let core_v = env!("CARGO_PKG_VERSION");
            if frontend_v != core_v {
                println!(
                    "{}  core/frontend mismatch: grip {core_v} vs python {frontend_v} — {}",
                    "warn".yellow().bold(),
                    "pip install -U gripsack".dimmed()
                );
            }
        }
        _ => {
            // the provisioned venv may carry the frontend even when the
            // system python doesn't — apply works fine in that state,
            // so a bare MISS reads as a broken install when it isn't
            let managed = std::fs::read_dir(gripsack_store::gripsack_home().join("frontend"))
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .map(|e| e.path().join("bin/python3"))
                .find(|p| p.exists())
                .and_then(|p| {
                    std::process::Command::new(&p)
                        .args(["-c", "import gripsack; print(gripsack.__version__)"])
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| (p, String::from_utf8_lossy(&o.stdout).trim().to_string()))
                });
            match managed {
                Some((path, v)) => println!(
                    "{}  frontend: gripsack python {v} (provisioned, {})",
                    mark(true),
                    path.display()
                ),
                None => {
                    println!(
                        "{}  frontend: `import gripsack` failed with {python} — pip install gripsack (or let provisioning handle it on first apply)",
                        mark(false)
                    );
                    ok = false;
                }
            }
        }
    }

    // TypeScript frontend (plan/0005 §1): optional.
    match std::process::Command::new("node").arg("--version").output() {
        Ok(out) if out.status.success() => {
            println!(
                "{}  node: {} (typescript frontend)",
                mark(true),
                String::from_utf8_lossy(&out.stdout).trim()
            );
        }
        _ => println!("info  node: not found — only needed for `frontend = \"typescript\"` repos"),
    }

    println!("      home: {}", gripsack_store::gripsack_home().display());

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
