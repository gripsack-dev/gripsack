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

    match std::process::Command::new("python3")
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
            println!("{}  python: `python3` not found on PATH", mark(false));
            ok = false;
        }
    }

    let check = std::process::Command::new("python3")
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
            println!(
                "{}  frontend: `import gripsack` failed — pip install gripsack",
                mark(false)
            );
            ok = false;
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
