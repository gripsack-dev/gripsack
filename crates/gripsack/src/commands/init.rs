//! `grip init` — create a usable workspace repo.
//!
//! The template is embedded in the binary, never fetched: first run
//! is exactly the moment a corporate proxy bites hardest, and an
//! embedded template can never skew from the binary's feature set.
//! The source lives in crates/gripsack/template/env-repo/ (inside the
//! crate, so `cargo publish` packages it) and is
//! mirrored to github.com/gripsack-dev/template for browsing.

use crate::render::Palette;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::ExitCode;
use tempfile::NamedTempFile;

/// Repository-relative paths and embedded contents.
const TEMPLATE: &[(&str, &str)] = &[
    (
        ".gitignore",
        include_str!("../../template/env-repo/.gitignore"),
    ),
    (
        "README.md",
        include_str!("../../template/env-repo/README.md"),
    ),
    (
        "configs/hello/hello.toml",
        include_str!("../../template/env-repo/configs/hello/hello.toml"),
    ),
    (
        "tsconfig.json",
        include_str!("../../template/env-repo/tsconfig.json"),
    ),
    // The optional editor SDK pin follows the binary, not an independently
    // maintained template version.
    (
        "package.json",
        concat!(
            r#"{
  "name": "my-env",
  "private": true,
  "type": "module",
  "devDependencies": {
    "@gripsack/core": "https://github.com/gripsack-dev/gripsack/releases/download/core-v"#,
            env!("CARGO_PKG_VERSION"),
            "/gripsack-core-",
            env!("CARGO_PKG_VERSION"),
            r#".tgz",
    "@types/node": "^26",
    "typescript": "^7"
  }
}
"#
        ),
    ),
];

const SETTINGS: (&str, &str) = ("env.toml", include_str!("../../template/env-repo/env.toml"));
const ENTRY: (&str, &str) = (
    "gripsack.ts",
    include_str!("../../template/env-repo/gripsack.ts"),
);

fn stage(path: &Path, contents: &str) -> Result<NamedTempFile, String> {
    let write = || -> io::Result<NamedTempFile> {
        // Stage on the same filesystem as the destination, with the ordinary
        // create mode (subject to umask). A failed write never leaves a partial
        // destination for a later init to mistake for a user's existing file.
        let mut file = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o666))
            .tempfile_in(path.parent().expect("template file has a parent"))?;
        file.write_all(contents.as_bytes())?;
        file.as_file().sync_data()?; // Catch deferred write errors before publication.
        Ok(file)
    };
    write().map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn publish(file: NamedTempFile, path: &Path) -> Result<fs::File, String> {
    file.persist_noclobber(path)
        .map_err(|e| format!("cannot install {}: {}", path.display(), e.error))
}

fn scaffold(dir: &Path, palette: Palette) -> Result<Vec<&'static str>, String> {
    for (name, _) in [SETTINGS, ENTRY] {
        let path = dir.join(name);
        match path.symlink_metadata() {
            Ok(_) => {
                return Err(format!(
                    "{} already looks like a workspace repo — nothing written",
                    dir.display()
                ));
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot inspect {}: {e}", path.display())),
        }
    }
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let mut created = Vec::with_capacity(TEMPLATE.len() + 2);
    for &(rel, contents) in TEMPLATE {
        let mut path = dir.to_path_buf();
        let mut parts = Path::new(rel).iter().peekable();
        while let Some(part) = parts.next() {
            path.push(part);
            if parts.peek().is_none() {
                break;
            }
            if let Err(e) = fs::create_dir(&path)
                && !(e.kind() == ErrorKind::AlreadyExists
                    && path.symlink_metadata().is_ok_and(|m| m.is_dir()))
            {
                return Err(format!("cannot create directory {}: {e}", path.display()));
            }
        }
        match path.symlink_metadata() {
            Ok(m) if m.is_file() => {
                eprintln!("  {} {rel} (exists — kept)", palette.warn("skip"));
                continue;
            }
            Ok(_) => return Err(format!("{} is not a regular file — kept", path.display())),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot inspect {}: {e}", path.display())),
        }
        publish(stage(&path, contents)?, &path)?;
        created.push(rel);
    }

    // Publish complete, retryable auxiliary files first, then stage *both*
    // markers before exposing either one. Never remove a user directory.
    let settings_path = dir.join(SETTINGS.0);
    let entry_path = dir.join(ENTRY.0);
    let settings = stage(&settings_path, SETTINGS.1)?;
    let entry = stage(&entry_path, ENTRY.1)?;
    let original = settings
        .as_file()
        .metadata()
        .map_err(|e| format!("cannot inspect staged {}: {e}", SETTINGS.0))?;
    let _settings = publish(settings, &settings_path)?;
    if let Err(e) = publish(entry, &entry_path) {
        // If another writer replaced or edited the marker, it is not ours to
        // unlink. The open handle keeps the original inode from being reused.
        if settings_path.symlink_metadata().is_ok_and(|m| {
            (m.dev(), m.ino()) == (original.dev(), original.ino())
                && m.mtime() == original.mtime()
                && m.mtime_nsec() == original.mtime_nsec()
        }) {
            fs::remove_file(&settings_path).map_err(|cleanup| {
                format!("{e}; cannot remove {}: {cleanup}", settings_path.display())
            })?;
        }
        return Err(e);
    }
    created.extend([SETTINGS.0, ENTRY.0]);
    Ok(created)
}

/// Create a workspace at `dir`. Existing env.toml or gripsack.ts refuses the
/// operation; other existing regular files are skipped and reported.
pub fn init(dir: &Path, palette: Palette) -> ExitCode {
    let created = match scaffold(dir, palette) {
        Ok(created) => created,
        Err(e) => {
            eprintln!("grip: workspace not created: {e}");
            return ExitCode::FAILURE;
        }
    };
    let label = palette.good("created");
    println!("{label} workspace repo in {}", dir.display());
    for rel in created {
        let note = match rel {
            "env.toml" => "— workspace settings and captured source policy",
            "package.json" => "— optional editor dependencies",
            "tsconfig.json" => "— editor typechecking",
            "gripsack.ts" => "— workspace outputs and file ownership",
            _ => "",
        };
        println!("  {rel} {note}");
    }
    report_git(dir);
    if let Err(e) = next_steps(&mut io::stdout().lock(), dir) {
        eprintln!("grip: workspace initialized, but cannot print next steps: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn report_git(dir: &Path) {
    if dir.join(".git").symlink_metadata().is_ok() {
        println!("  existing .git — kept");
        return;
    }
    // Inherit Git's streams: its specific diagnostic belongs to the user.
    match std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir)
        .status()
    {
        Ok(status) if status.success() => println!("  initialized git repository"),
        Ok(status) => eprintln!("grip: workspace initialized, but git init failed ({status})"),
        Err(e) if e.kind() == ErrorKind::NotFound && git_missing(dir) => {
            eprintln!("grip: workspace initialized; git not available — skipped git init ({e})");
        }
        Err(e) => eprintln!("grip: workspace initialized, but could not start git init: {e}"),
    }
}

// ENOENT can also mean a *present* Git has a missing interpreter/loader.
// Only label Git absent when all search locations are known to be missing.
fn git_missing(cwd: &Path) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/bin:/usr/bin".into());
    std::env::split_paths(&path).all(|bin| {
        cwd.join(bin)
            .join("git")
            .symlink_metadata()
            .is_err_and(|e| matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory))
    })
}

fn next_steps(out: &mut impl Write, dir: &Path) -> io::Result<()> {
    out.write_all(b"next: ")?;
    if dir != Path::new(".") {
        out.write_all(b"cd -P '")?;
        if dir.is_relative() {
            // Bypass CDPATH and avoid interpreting a leading '-' as an option.
            out.write_all(b"./")?;
        }
        // Quote the actual argument bytes, not a lossy display path.
        for (i, part) in dir
            .as_os_str()
            .as_encoded_bytes()
            .split(|&b| b == b'\'')
            .enumerate()
        {
            if i != 0 {
                out.write_all(b"'\\''")?;
            }
            out.write_all(part)?;
        }
        out.write_all(b"' && ")?;
    }
    writeln!(out, "grip check && grip plan && grip apply")
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
