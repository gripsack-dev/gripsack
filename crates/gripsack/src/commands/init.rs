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

/// (repo-relative path, embedded contents). `hosts/myhost.ts` is
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
        "hosts/myhost.ts",
        include_str!("../../template/env-repo/hosts/myhost.ts"),
    ),
    (
        "modules/hello.ts",
        include_str!("../../template/env-repo/modules/hello.ts"),
    ),
    (
        "modules/examples.ts",
        include_str!("../../template/env-repo/modules/examples.ts"),
    ),
    (
        "configs/hello/hello.toml",
        include_str!("../../template/env-repo/configs/hello/hello.toml"),
    ),
    (
        "tsconfig.json",
        include_str!("../../template/env-repo/tsconfig.json"),
    ),
];

/// package.json for the IDE story: the frontend's types as a devDep so
/// editors give autocomplete + inline errors on module code. Also the
/// deliberate pin (0013 D3) — the repo's install shadows the embedded
/// frontend, so it must shadow a COMPATIBLE version: pin to this
/// grip's major.minor, floating the patch.
fn package_json() -> String {
    let version = env!("CARGO_PKG_VERSION")
        .rsplit_once('.')
        .map(|(major_minor, _)| major_minor)
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    format!(
        r#"{{
  "name": "my-env",
  "private": true,
  "type": "module",
  "devDependencies": {{
    "@gripsack/core": "^{version}",
    "typescript": "^7"
  }}
}}
"#
    )
}

fn hostname() -> String {
    super::sanitize_hostname(&super::hostname())
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
        let rel = if *rel == "hosts/myhost.ts" {
            format!("hosts/{host}.ts")
        } else {
            rel.to_string()
        };
        let path: PathBuf = dir.join(&rel);
        if path.exists() {
            eprintln!("  {} {rel} (exists — kept)", palette.warn("skip"));
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
    // package.json is version-dynamic (the pin must be compatible with
    // this grip), so it's generated, not included
    let pkg_rel = "package.json".to_string();
    let pkg_path = dir.join(&pkg_rel);
    if pkg_path.exists() {
        eprintln!("  {} {pkg_rel} (exists — kept)", palette.warn("skip"));
    } else if let Err(e) = std::fs::write(&pkg_path, package_json()) {
        eprintln!("grip: cannot write {}: {e}", pkg_path.display());
        return ExitCode::FAILURE;
    } else {
        created.push(pkg_rel);
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
            "package.json" => "— the IDE story + deliberate pin",
            "tsconfig.json" => "— editor typechecking",
            r if r.starts_with("hosts/") => "— this machine's entrypoint (tags)",
            "modules/hello.ts" => "— a working first module (config-only)",
            "modules/examples.ts" => "— the feature tour, commented",
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
