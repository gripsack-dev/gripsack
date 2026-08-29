//! `grip adopt` — reversible adoption as a first-class flow (0015).
//!
//! Inspect a live config path, recommend an ownership mode with a
//! stated reason, generate the repo payload + module + host entry,
//! show the plan, and touch nothing until confirmed. The apply runs
//! with scoped take-over and prior-state capture, so adoption is fully
//! reversible: rollback restores the original files.

use crate::commands::{eval_repo, expand_home, hostname, trust_gate};
use crate::render::{self, Palette};
use gripsack_store as store;
use owo_colors::OwoColorize;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Apps that rewrite their own config — tracked_copy or the user's
/// edits get clobbered. Dirname match, lowercase (0015 §2).
const SELF_REWRITING: &[&str] = &[
    "zed",
    "code",
    "vscode",
    "discord",
    "slack",
    "spotify",
    "obsidian",
    "brave",
    "chromium",
    "google-chrome",
];

/// Files other tools also write — gripsack may own one managed block,
/// never the file (0015 §2).
const SHARED_SHELL_FILES: &[&str] = &[".bashrc", ".zshrc", ".profile", ".bash_profile"];

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Owned,
    TrackedCopy,
    Merge,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Owned => "owned",
            Self::TrackedCopy => "tracked_copy",
            Self::Merge => "merge",
        }
    }
}

pub fn adopt(
    target: &str,
    name: Option<&str>,
    mode: Option<&str>,
    host: Option<&str>,
    yes: bool,
    palette: Palette,
) -> ExitCode {
    let repo = std::env::current_dir().unwrap_or_else(|_| ".".into());
    if !repo.join("env.toml").is_file() {
        eprintln!(
            "grip: {} is not an env repo (no env.toml) — run `grip init` first",
            repo.display()
        );
        return ExitCode::FAILURE;
    }
    let dest = expand_home(target);
    if !dest.symlink_metadata().is_ok() {
        eprintln!("grip: nothing at {target} — adopt adopts what exists");
        return ExitCode::FAILURE;
    }
    let home_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if dest == home_dir || dest.parent().is_none() {
        eprintln!("grip: refusing to adopt {target} — too broad");
        return ExitCode::FAILURE;
    }
    let is_dir = dest.is_dir()
        && !dest
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink());
    let name = match name
        .map(str::to_string)
        .or_else(|| default_name(&dest, is_dir))
    {
        Some(n) => n,
        None => {
            eprintln!("grip: cannot derive a module name from {target} — pass --name");
            return ExitCode::FAILURE;
        }
    };

    // already managed? every generation's manifest is evidence
    if let Some(owner) = managed_by(&dest) {
        eprintln!("grip: {target} is already managed by module \"{owner}\" — nothing to adopt");
        return ExitCode::FAILURE;
    }

    // ── inspect ────────────────────────────────────────────────────
    let files = collect_files(&dest);
    if is_dir && files.is_empty() {
        eprintln!("grip: {target} is an empty directory — nothing to adopt");
        return ExitCode::FAILURE;
    }
    let total: u64 = files.iter().map(|(_, size)| size).sum();
    let links = files
        .iter()
        .filter(|(rel, _)| {
            dest.join(rel)
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
        })
        .count();
    let mode = match mode {
        Some(m) => match m {
            "owned" => Mode::Owned,
            "tracked_copy" => Mode::TrackedCopy,
            "merge" => Mode::Merge,
            other => {
                eprintln!("grip: unknown mode {other:?} — owned | tracked_copy | merge");
                return ExitCode::from(2);
            }
        },
        None => recommend(&dest, is_dir),
    };
    if mode == Mode::Merge && is_dir {
        eprintln!("grip: merge owns one block inside a single file — it cannot adopt a directory");
        return ExitCode::FAILURE;
    }
    println!(
        "{} {target} — {} file{}, {:.1} kB{}",
        "adopting".green().bold(),
        files.len(),
        if files.len() == 1 { "" } else { "s" },
        total as f64 / 1024.0,
        if links > 0 {
            format!(
                ", {links} symlink{} (dereferenced)",
                if links == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        }
    );
    println!(
        "  ownership: {} — {}",
        mode.as_str().bold(),
        reason(&dest, mode)
    );

    // ── generate ───────────────────────────────────────────────────
    let rel_files: Vec<String> = files.iter().map(|(rel, _)| rel.clone()).collect();
    if let Err(e) = write_payload(&repo, &dest, &name, &rel_files, is_dir) {
        eprintln!("grip: cannot write payload: {e}");
        return ExitCode::FAILURE;
    }
    let module_ts = module_source(&name, &dest, &rel_files, mode, is_dir);
    if let Err(e) = std::fs::write(repo.join("modules").join(format!("{name}.ts")), &module_ts) {
        eprintln!("grip: cannot write modules/{name}.ts: {e}");
        return ExitCode::FAILURE;
    }
    let host_name = host.map(str::to_string).unwrap_or_else(hostname);
    let host_rel = format!("hosts/{host_name}.ts");
    let host_path = repo.join(&host_rel);
    if !host_path.is_file() {
        eprintln!(
            "grip: no {host_rel} — create it (or run `grip init`) and add:\n\n{}\n",
            host_snippet(&name)
        );
        return ExitCode::FAILURE;
    }
    let host_src = std::fs::read_to_string(&host_path).unwrap_or_default();
    match update_host(&host_src, &name) {
        Some(updated) => {
            if let Err(e) = std::fs::write(&host_path, updated) {
                eprintln!("grip: cannot update {host_rel}: {e}");
                return ExitCode::FAILURE;
            }
        }
        None => {
            eprintln!(
                "grip: {host_rel} doesn't match the expected defineEnv shape — add this yourself:\n\n{}\n",
                host_snippet(&name)
            );
            return ExitCode::FAILURE;
        }
    }
    println!(
        "  wrote configs/{}/ · modules/{name}.ts · {host_rel}",
        name.dimmed()
    );

    // ── plan ───────────────────────────────────────────────────────
    if let Some(code) = trust_gate(&repo) {
        return code;
    }
    let outcome = match eval_repo(&repo, Some(&host_name), palette) {
        Ok(o) => o,
        Err(code) => {
            eprintln!(
                "grip: generated files don't eval — the repo is untouched otherwise; inspect modules/{name}.ts"
            );
            return code;
        }
    };
    let ir = match crate::commands::check_ir(&outcome.ir_json, palette) {
        Ok(ir) => ir,
        Err(code) => return code,
    };
    println!("{}", render::diff_section(&ir, &repo, palette));
    println!(
        "  {}",
        "prior state will be recorded — rollback restores your original files".dimmed()
    );

    // ── confirm & apply ────────────────────────────────────────────
    if !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("grip: non-interactive — pass --yes to apply, or run the shown plan first");
            return ExitCode::FAILURE;
        }
        eprint!("apply? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() || !matches!(answer.trim(), "y" | "Y") {
            eprintln!(
                "not applied — repo files stay; delete configs/{name}, modules/{name}.ts and the {host_rel} entry to abandon"
            );
            return ExitCode::FAILURE;
        }
    }
    let destinations: std::collections::BTreeSet<String> =
        rel_files.iter().map(|rel| tilde(&dest.join(rel))).collect();
    // generation 0 (0015 §4): on a fresh machine, adopt records the
    // empty baseline first — otherwise the adopt apply IS generation 1
    // and there is nothing to roll back to. With it, `grip rollback`
    // reaches the pre-adopt state and restores the original files.
    let home = store::gripsack_home();
    if store::current_generation(&home).is_none()
        && store::list_generations(&home).is_empty()
        && let Err(e) = store::write_manifest(
            &home,
            &store::Generation {
                number: 0,
                modules: Default::default(),
            },
        )
    {
        eprintln!("grip: cannot record the baseline generation: {e}");
        return ExitCode::FAILURE;
    }
    crate::commands::apply_scoped(
        &repo,
        Some(&host_name),
        Vec::new(),
        Some(destinations),
        None,
        palette,
    )
}

fn default_name(dest: &Path, is_dir: bool) -> Option<String> {
    let raw = if is_dir {
        dest.file_name()?.to_string_lossy().into_owned()
    } else {
        dest.file_name()?
            .to_string_lossy()
            .trim_start_matches('.')
            .to_string()
    };
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
    if clean.is_empty() { None } else { Some(clean) }
}

/// Which module already manages a destination under this path.
fn managed_by(dest: &Path) -> Option<String> {
    let home = store::gripsack_home();
    let n = store::current_generation(&home)?;
    let manifest = store::read_manifest(&home, n).ok()?;
    for (name, state) in &manifest.modules {
        for entry in &state.entries {
            let deployed = expand_home(&entry.to);
            if deployed == *dest || deployed.starts_with(dest) {
                return Some(name.clone());
            }
        }
    }
    None
}

/// Relative paths + sizes of every real file under dest (or the file
/// itself). Foreign symlinks are listed but dereferenced at copy.
fn collect_files(dest: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    if dest.is_file()
        || dest
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
    {
        let rel = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        out.push((rel, size));
        return out;
    }
    for entry in walkdir(dest) {
        let rel = entry
            .strip_prefix(dest)
            .expect("walked path is under root")
            .to_string_lossy()
            .replace('\\', "/");
        let size = std::fs::metadata(&entry).map(|m| m.len()).unwrap_or(0);
        out.push((rel, size));
    }
    out.sort();
    out
}

fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // is_dir follows links: a symlinked subdir's CONTENT is adopted
        if path.is_dir() {
            out.extend(walkdir(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn recommend(dest: &Path, is_dir: bool) -> Mode {
    let base = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !is_dir && SHARED_SHELL_FILES.contains(&base.as_str()) {
        Mode::Merge
    } else if SELF_REWRITING.contains(&base.as_str()) {
        Mode::TrackedCopy
    } else {
        Mode::Owned
    }
}

fn reason(dest: &Path, mode: Mode) -> String {
    let base = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match mode {
        Mode::TrackedCopy => {
            format!("{base} rewrites its own config — drift is kept, never clobbered")
        }
        Mode::Merge => format!("other tools write {base} too — gripsack owns one managed block"),
        Mode::Owned => format!("{base} doesn't rewrite its config — a read-only link fits"),
    }
}

/// Copy the live payload into configs/<name>/ (verbatim, symlinks
/// dereferenced — adopt takes the content, not the indirection).
fn write_payload(
    repo: &Path,
    dest: &Path,
    name: &str,
    rel_files: &[String],
    is_dir: bool,
) -> std::io::Result<()> {
    let payload = repo.join("configs").join(name);
    for rel in rel_files {
        let source = if is_dir {
            dest.join(rel)
        } else {
            dest.to_path_buf()
        };
        let out = payload.join(rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &out)?;
    }
    Ok(())
}

fn tilde(dest: &Path) -> String {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    match dest.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => dest.to_string_lossy().into_owned(),
    }
}

fn module_source(
    name: &str,
    dest: &Path,
    rel_files: &[String],
    mode: Mode,
    is_dir: bool,
) -> String {
    let to = tilde(dest);
    match (mode, is_dir) {
        (Mode::Merge, _) => format!(
            "import {{ merge, module }} from \"@gripsack/core\";\n\n\
             export default module(\"{name}\", {{\n  config: {{\n    \"configs/{name}/{}\": merge(\"{to}\", \"#\"),\n  }},\n}});\n",
            rel_files.first().map(String::as_str).unwrap_or("config")
        ),
        (_, true) => format!(
            "import {{ module, tree }} from \"@gripsack/core\";\n\n\
             export default module(\"{name}\", {{\n  config: tree(\"configs/{name}\", \"{to}\", \"{}\"),\n}});\n",
            mode.as_str()
        ),
        (m, false) => {
            let ctor = match m {
                Mode::Owned => "symlink",
                Mode::TrackedCopy => "trackedCopy",
                Mode::Merge => unreachable!("merge handled above"),
            };
            format!(
                "import {{ {ctor}, module }} from \"@gripsack/core\";\n\n\
                 export default module(\"{name}\", {{\n  config: {{\n    \"configs/{name}/{}\": {ctor}(\"{to}\"),\n  }},\n}});\n",
                rel_files.first().map(String::as_str).unwrap_or("config")
            )
        }
    }
}

/// Insert the import and the modules-array entry. Conservative: the
/// host file must have a recognizable defineEnv shape; otherwise the
/// user pastes the snippet (never a mangled host file).
fn update_host(src: &str, name: &str) -> Option<String> {
    let ident: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let import = format!("import {ident} from \"../modules/{name}.ts\";");
    if src.contains(&import) {
        return Some(src.to_string());
    }
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
    let last_import = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with("import ") && l.contains(" from "))?;
    lines.insert(last_import + 1, import);
    let modules_pos = lines.iter().position(|l| l.contains("modules: ["))?;
    let modules_line = &lines[modules_pos];
    let indent = modules_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect::<String>();
    if modules_line.trim_end().ends_with("],") || modules_line.contains("]") {
        // single-line array: modules: [hello],
        let pos = lines[modules_pos].find(']')?;
        lines[modules_pos].insert_str(pos, &format!("{ident}, "));
    } else {
        lines.insert(modules_pos + 1, format!("{indent}  {ident},"));
    }
    Some(lines.join("\n") + "\n")
}

fn host_snippet(name: &str) -> String {
    let ident: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "import {ident} from \"../modules/{name}.ts\";\n// and add `{ident}` to the modules array"
    )
}
