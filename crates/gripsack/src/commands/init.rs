//! `grip init` — scaffold an env repo (0001 §5).
//!
//! The template is embedded in the binary, never fetched: first run
//! is exactly the moment a corporate proxy bites hardest, and an
//! embedded template can never skew from the binary's feature set.
//! The source lives in crates/gripsack/template/env-repo/ (inside the
//! crate, so `cargo publish` packages it) and is
//! mirrored to github.com/gripsack-dev/template for browsing.

use crate::render::Palette;
use owo_colors::OwoColorize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// (repo-relative path, embedded contents). `hosts/myhost.py` is
/// renamed to the machine's hostname at write time.
const TEMPLATE: &[(&str, &str)] = &[
    ("env.toml", include_str!("../../template/env-repo/env.toml")),
    (
        ".gitignore",
        include_str!("../../template/env-repo/.gitignore"),
    ),
    (
        "README.md",
        include_str!("../../template/env-repo/README.md"),
    ),
    (
        "hosts/myhost.py",
        include_str!("../../template/env-repo/hosts/myhost.py"),
    ),
    (
        "modules/hello.py",
        include_str!("../../template/env-repo/modules/hello.py"),
    ),
    (
        "modules/examples.py",
        include_str!("../../template/env-repo/modules/examples.py"),
    ),
    (
        "configs/hello/hello.toml",
        include_str!("../../template/env-repo/configs/hello/hello.toml"),
    ),
];

fn hostname() -> String {
    let raw = super::hostname();
    // a valid python module name: alnum, dash and underscore
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if clean.is_empty() {
        "myhost".into()
    } else {
        clean
    }
}

/// Scaffold an env repo at `dir`. Never clobbers: an existing env.toml
/// means "this is already an env repo" and is an error; any other
/// existing file is skipped and reported.
pub fn init(dir: &Path, palette: Palette) -> ExitCode {
    let c = |s: &str| {
        if palette.enabled {
            s.green().to_string()
        } else {
            s.to_string()
        }
    };
    if dir.join("env.toml").exists() {
        eprintln!(
            "grip: {} already looks like an env repo (env.toml exists) — nothing written",
            dir.display()
        );
        return ExitCode::FAILURE;
    }
    let host = hostname();
    let mut created = Vec::new();
    for (rel, contents) in TEMPLATE {
        let rel = if *rel == "hosts/myhost.py" {
            format!("hosts/{host}.py")
        } else {
            rel.to_string()
        };
        let path: PathBuf = dir.join(&rel);
        if path.exists() {
            eprintln!("  {} {rel} (exists — kept)", "skip".yellow());
            continue;
        }
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            eprintln!("grip: cannot create {}: {e}", parent.display());
            return ExitCode::FAILURE;
        }
        if let Err(e) = std::fs::write(&path, contents) {
            eprintln!("grip: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        created.push(rel);
    }
    let git = if dir.join(".git").exists() {
        "already a git repository — kept"
    } else {
        match std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
        {
            Ok(s) if s.success() => "initialized git repository",
            _ => "git not available — skipped git init",
        }
    };
    println!("{} env repo in {}", c("created"), dir.display());
    for rel in &created {
        let note = match rel.as_str() {
            "env.toml" => "— the environment declaration",
            r if r.starts_with("hosts/") => "— this machine's entrypoint (tags)",
            "modules/hello.py" => "— a working first module (config-only)",
            "modules/examples.py" => "— the feature tour, commented",
            _ => "",
        };
        println!("  {rel} {note}");
    }
    println!("  {git}");
    println!(
        "next: {} && {}",
        c("grip check"),
        c(&format!("grip apply --host {host}"))
    );
    ExitCode::SUCCESS
}
