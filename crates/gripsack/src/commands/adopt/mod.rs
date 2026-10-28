//! `grip adopt` — reversible adoption as a first-class flow (0015).
//!
//! Five phases, the middle three observable before anything writes to
//! disk outside the repo: inspect → ask → generate → plan → apply.
//! The apply uses scoped take-over and prior-state capture, so
//! adoption is fully reversible. The audit that shaped this module is
//! 0015 §7: ask, don't guess; say exactly what you wrote.

mod generate;
mod inspect;
mod prompt;

use crate::commands::expand_home;
use crate::render::{self, Palette};

use gripsack_store as store;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub fn adopt(
    target: &str,
    name: Option<&str>,
    mode: Option<&str>,
    host: Option<&str>,
    yes: bool,
    resume: bool,
    palette: Palette,
) -> ExitCode {
    let repo = std::env::current_dir().unwrap_or_else(|_| ".".into());
    if !repo.join("gripsack.ts").is_file() || !repo.join("env.toml").is_file() {
        eprintln!(
            "grip: {} is not a workspace repo (requires gripsack.ts and env.toml) — run `grip init` first",
            repo.display()
        );
        return ExitCode::FAILURE;
    }
    // --yes bypasses confirmation, not source-bound approval.
    let mut sink = crate::render::DiagnosticSink::terminal(palette, &repo);
    let prepared = match super::prepared::PreparedEvaluation::capture(
        &repo,
        host.map(str::to_owned),
        &mut sink,
    ) {
        Ok(prepared) => prepared,
        Err(code) => return code,
    };
    if let Err(code) = prepared.authorize(true, None) {
        return code;
    }
    let dest = expand_home(target);
    let home_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if !dest.symlink_metadata().is_ok() {
        eprintln!("grip: nothing at {target} — adopt adopts what exists");
        return ExitCode::FAILURE;
    }
    if dest == home_dir {
        eprintln!("grip: refusing to adopt {target} — too broad");
        return ExitCode::FAILURE;
    }
    // 0015 §7 S3: absolute paths outside $HOME make non-portable repos
    if !dest.starts_with(&home_dir) {
        eprintln!(
            "grip: {target} is outside your home — adopt manages user config, not system paths"
        );
        return ExitCode::FAILURE;
    }
    let is_dir = dest.is_dir()
        && !dest
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink());
    let name = match name
        .map(str::to_string)
        .or_else(|| generate::default_name(&dest, is_dir))
    {
        Some(n) => n,
        None => {
            eprintln!("grip: cannot derive a profile name from {target} — pass --name");
            return ExitCode::FAILURE;
        }
    };
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        eprintln!("grip: profile name must contain only ASCII letters, digits, '-' or '_'");
        return ExitCode::FAILURE;
    }
    if let Some(owner) = managed_by(&dest) {
        eprintln!("grip: {target} is already managed by profile \"{owner}\" — nothing to adopt");
        return ExitCode::FAILURE;
    }
    if resume {
        let outcome = match super::eval::eval_prepared(prepared, &mut sink) {
            Ok(outcome) => outcome,
            Err(code) => return code,
        };
        return finish_adoption(outcome, &dest, is_dir, &name, yes, palette);
    }
    drop(prepared);
    drop(sink);
    // §7 S4: the repo gets the same never-clobber rule as $HOME
    for artifact in [
        repo.join("profiles").join(format!("{name}.ts")),
        repo.join("configs").join(&name),
    ] {
        if artifact.symlink_metadata().is_ok() {
            eprintln!(
                "grip: {} already exists — refusing to overwrite it (pick another --name, or remove it first)",
                artifact.display()
            );
            return ExitCode::FAILURE;
        }
    }

    // ── inspect ────────────────────────────────────────────────────
    let inv = inspect::inspect(&dest, is_dir);
    if inv.files.is_empty() {
        eprintln!("grip: {target} has no adoptable files");
        return ExitCode::FAILURE;
    }
    println!(
        "{} {target} — {} file{}, {}{}",
        palette.good("adopting"),
        inv.files.len(),
        if inv.files.len() == 1 { "" } else { "s" },
        inspect::fmt_kib(inv.total_bytes),
        if inv.skipped.is_empty() {
            String::new()
        } else {
            format!(", {} skipped", inv.skipped.len())
        }
    );
    for skipped in &inv.skipped {
        println!(
            "  {} {}",
            palette.warn("skip"),
            palette.dim(&format!("{} ({})", skipped.rel, skipped.reason))
        );
    }
    if inv.total_bytes > inspect::SIZE_WARN_BYTES {
        let largest = inspect::largest(&inv, 3)
            .iter()
            .map(|(rel, size)| format!("{rel} ({})", inspect::fmt_kib(*size)))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  {} this is large for a config repo — largest: {largest}",
            palette.warn("note")
        );
    }

    // ── ask ────────────────────────────────────────────────────────
    let mode = match prompt::ask_mode(mode, palette) {
        Ok(m) => m,
        Err(()) => return ExitCode::FAILURE,
    };
    if mode == generate::MODE_MERGE && is_dir {
        eprintln!("grip: merge owns one block inside a single file — it cannot adopt a directory");
        return ExitCode::FAILURE;
    }
    println!("  {}", palette.dim(&prompt::mode_line(&mode)));

    // ── generate ───────────────────────────────────────────────────
    let rel_files: Vec<String> = inv.files.iter().map(|f| f.rel.clone()).collect();
    let written = [
        format!("configs/{name}/"),
        format!("profiles/{name}.ts"),
        "gripsack.ts".to_owned(),
    ];
    let workspace_rel = &written[2];
    let revert = |why: &str| {
        eprintln!("grip: {why}");
        eprintln!(
            "  written so far: {} — remove them (and the `{name}` entry in {}) to abandon",
            written.join(", "),
            written[2]
        );
        ExitCode::FAILURE
    };
    if let Err(e) = generate::write_payload(&repo, &dest, &name, &rel_files, is_dir) {
        return revert(&format!("cannot write payload: {e}"));
    }
    let to = generate::tilde(&dest);
    let profile_ts = generate::profile_source(&name, &to, &rel_files, &mode, is_dir);
    if let Err(error) = std::fs::create_dir_all(repo.join("profiles")).and_then(|()| {
        std::fs::write(
            repo.join("profiles").join(format!("{name}.ts")),
            &profile_ts,
        )
    }) {
        return revert(&format!("cannot write profiles/{name}.ts: {error}"));
    }
    let workspace_path = repo.join(workspace_rel);
    let workspace_src = match std::fs::read_to_string(&workspace_path) {
        Ok(source) => source,
        Err(error) => return revert(&format!("cannot read {workspace_rel}: {error}")),
    };
    match generate::update_workspace(&workspace_src, &name) {
        Some(updated) => {
            if let Err(error) = std::fs::write(&workspace_path, updated) {
                return revert(&format!("cannot update {workspace_rel}: {error}"));
            }
        }
        None => {
            eprintln!("grip: add the generated profile to {workspace_rel} manually:\n");
            eprintln!("{}", generate::workspace_snippet(&name));
        }
    }
    println!(
        "  {} configs/{name}/ · profiles/{name}.ts · {workspace_rel}",
        palette.good("wrote")
    );

    eprintln!(
        "grip: review the generated files, approve the changed snapshot with `grip trust add .`, then resume:"
    );
    eprintln!(
        "  grip adopt {} --name {} --resume{}",
        serde_json::to_string(target).expect("string"),
        name,
        host.map(|host| format!(" --host {host}"))
            .unwrap_or_default(),
    );
    // No newly generated source is evaluated or deployed before explicit review.
    ExitCode::FAILURE
}

fn finish_adoption(
    outcome: super::eval::EvalOutcome,
    dest: &Path,
    is_dir: bool,
    name: &str,
    yes: bool,
    palette: Palette,
) -> ExitCode {
    let repo = outcome.sources.repository_identity();
    let mut sink = crate::render::DiagnosticSink::terminal(palette, repo);
    sink.bind_captured_source(std::sync::Arc::clone(&outcome.sources));
    let ir = match crate::commands::check_ir(&outcome.ir_json, &mut sink) {
        Ok(ir) => ir,
        Err(code) => return code,
    };
    let Some(profile) = ir.workspace.outputs.iter().find_map(|output| match output {
        gripsack_ir::workspace_model::WorkspaceOutput::Profile(profile) if profile.name == name => {
            Some(profile)
        }
        _ => None,
    }) else {
        eprintln!("grip: no profile {name:?} in the approved workspace");
        return ExitCode::FAILURE;
    };
    let target = match store::canonical_dest(&generate::tilde(dest)) {
        Ok(target) => target,
        Err(error) => {
            eprintln!("grip: cannot resolve adoption target: {error}");
            return ExitCode::FAILURE;
        }
    };
    // the scoped take-over set names the DESTINATIONS (0029: a file
    // adopt must not re-join its own file name — `dest.join(rel)`
    // doubled it, so the set never matched and the origin was never
    // captured when the content already matched)
    let mut adopting = std::collections::BTreeSet::new();
    for file in &profile.files {
        use gripsack_ir::workspace::WorkspaceDestination;
        let path = match &file.destination {
            WorkspaceDestination::Symlink { path }
            | WorkspaceDestination::TrackedCopy { path }
            | WorkspaceDestination::ManagedBlock { path, .. } => path,
        };
        let destination = match store::canonical_dest(path) {
            Ok(destination) => destination,
            Err(error) => {
                eprintln!("grip: cannot resolve adoption destination: {error}");
                return ExitCode::FAILURE;
            }
        };
        let inside = if is_dir {
            destination != target && destination.starts_with(&target)
        } else {
            destination == target
        };
        if !inside {
            eprintln!(
                "grip: profile {name:?} declares a destination outside the requested adoption target"
            );
            return ExitCode::FAILURE;
        }
        adopting.insert(path.clone());
    }
    if adopting.is_empty() {
        eprintln!("grip: profile {name:?} has no destinations to adopt");
        return ExitCode::FAILURE;
    }
    let repository = gripsack_exec::Repository::evaluated(
        std::sync::Arc::clone(&outcome.sources),
        outcome.receipt,
    );
    match render::diff_section(
        &ir,
        &repository,
        &adopting,
        palette,
        &[name.to_owned()],
        outcome.fetch.limits(),
    ) {
        Ok(section) => println!("{section}"),
        Err(error) => {
            eprintln!("grip: cannot compute the preview: {error}");
            return ExitCode::FAILURE;
        }
    }
    println!(
        "  {}",
        palette.dim("prior state will be recorded — rollback restores your original files")
    );

    // ── confirm & apply ────────────────────────────────────────────
    if !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("grip: non-interactive — pass --yes to apply, or run the shown plan first");
            return ExitCode::FAILURE;
        }
        if !prompt::confirm_apply(palette) {
            eprintln!(
                "not applied — repo files stay; review them and rerun with --resume to continue"
            );
            return ExitCode::FAILURE;
        }
    }
    // generation 0 (0015 §4): on a fresh machine, adopt records the
    // empty baseline first, or the adopt apply IS generation 1 and
    // there is nothing to roll back to
    let home = store::gripsack_home();
    let baseline = gripsack_fs::open_or_create(&home);
    let current = store::current_generation(&home);
    if matches!(current, Ok(None))
        && store::list_generations(&home).is_ok_and(|g| g.is_empty())
        && let Err(e) = baseline.and_then(|cap| {
            store::write_manifest(
                &cap,
                &store::Generation {
                    number: store::GenerationId::new(0),
                    deployments: Default::default(),
                    directory_transitions: Vec::new(),
                    migration: None,
                },
            )
        })
    {
        eprintln!("grip: cannot record the baseline generation: {e}");
        return ExitCode::FAILURE;
    }
    crate::commands::apply_scoped(outcome, name.to_owned(), adopting, palette)
}

/// Which retained profile already manages a destination under this path.
fn managed_by(dest: &Path) -> Option<String> {
    let home = store::gripsack_home();
    // display path: an unreadable current reads as unmanaged rather
    // than erroring an interactive prompt; mutation paths fail closed
    let n = store::current_generation(&home).ok()??;
    let manifest = store::read_manifest(&home, n).ok()?;
    for (name, state) in &manifest.deployments {
        for entry in &state.entries {
            let deployed = expand_home(&entry.to);
            if deployed == *dest || deployed.starts_with(dest) {
                return Some(name.clone());
            }
        }
    }
    None
}
