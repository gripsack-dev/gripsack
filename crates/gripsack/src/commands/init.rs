//! `grip init` — create a usable workspace repo.
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

/// Repository-relative paths and embedded contents.
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
        "gripsack.ts",
        include_str!("../../template/env-repo/gripsack.ts"),
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

/// Optional editor dependencies, pinned to this binary's published SDK archive.
/// Capture excludes node_modules, so editor installs never select evaluated code.
fn package_json() -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        r#"{{
  "name": "my-env",
  "private": true,
  "type": "module",
  "devDependencies": {{
    "@gripsack/core": "https://github.com/gripsack-dev/gripsack/releases/download/core-v{version}/gripsack-core-{version}.tgz",
    "@types/node": "^26",
    "typescript": "^7"
  }}
}}
"#
    )
}

/// Create a workspace at `dir`. Existing env.toml or gripsack.ts refuses the
/// operation; other existing files are skipped and reported.
pub fn init(dir: &Path, palette: Palette) -> ExitCode {
    let c = |s: &str| {
        if palette.enabled {
            s.green().to_string()
        } else {
            s.to_string()
        }
    };
    if dir.join("env.toml").symlink_metadata().is_ok()
        || dir.join("gripsack.ts").symlink_metadata().is_ok()
    {
        eprintln!(
            "grip: {} already looks like a workspace repo — nothing written",
            dir.display()
        );
        return ExitCode::FAILURE;
    }
    let mut created = Vec::new();
    for (rel, contents) in TEMPLATE {
        let rel = rel.to_string();
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
    println!("{} workspace repo in {}", c("created"), dir.display());
    for rel in &created {
        let note = match rel.as_str() {
            "env.toml" => "— workspace settings and captured source policy",
            "package.json" => "— optional editor dependencies",
            "tsconfig.json" => "— editor typechecking",
            "gripsack.ts" => "— workspace outputs and file ownership",
            _ => "",
        };
        println!("  {rel} {note}");
    }
    println!("  {git}");
    println!(
        "next: {} && {}",
        c("grip check"),
        c("grip plan && grip apply")
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_workspace_is_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let entrypoint = directory.path().join("gripsack.ts");
        std::fs::write(&entrypoint, "existing workspace").unwrap();
        assert_eq!(init(directory.path(), Palette::plain()), ExitCode::FAILURE);
        assert_eq!(
            std::fs::read_to_string(entrypoint).unwrap(),
            "existing workspace"
        );
        assert!(!directory.path().join("env.toml").exists());
    }
}
