//! The executor (0001 §4, 0007 §5): run a validated IR graph against
//! the store and deploy — one new generation per apply, or "already
//! satisfied" when nothing changed (0008 §3).
//!
//! v0.1: sequential execution in DAG order. The ready-queue scheduler
//! with resource locks replaces the loop without changing semantics.

use crate::expand;
use gripsack_fetch::{fetch, FetchError};
use gripsack_ir::{Build, Entry, Ir, Ownership, Step, StepAction, Verify};
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use tracing::{info, info_span};

/// What an apply run needs beyond the IR.
pub struct Ctx {
    /// $GRIPSACK_HOME.
    pub home: PathBuf,
    /// The env repo root (config `from` paths are repo-relative).
    pub repo: PathBuf,
    /// Subset apply: only these modules plus their dependencies (0001
    /// §3.6). Empty = the whole graph.
    pub only: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Nothing changed; no generation created.
    Satisfied { generation: Option<u64> },
    /// A new generation was deployed and activated.
    Applied { generation: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("fetch failed: {0}")]
    Fetch(#[from] FetchError),
    #[error("verify failed for {module}: {detail}")]
    Verify { module: String, detail: String },
    #[error("step {step} failed in {module}: {detail}")]
    Step {
        module: String,
        step: String,
        detail: String,
    },
    #[error("scheduling: {0}")]
    Plan(#[from] crate::PlanError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Apply the whole graph (or a subset) and activate a new generation.
pub fn apply(ir: &Ir, ctx: &Ctx) -> Result<Outcome, ExecError> {
    let order = scoped_order(ir, &ctx.only)?;
    let steps_by_module = expand::expand_all(&ir.modules);

    // The manifest starts from the current generation — a subset apply
    // replaces only the modules it touches (0001 §3.6).
    let current_gen = store::current_generation(&ctx.home);
    let mut modules: BTreeMap<String, store::ModuleState> = current_gen
        .and_then(|n| store::read_manifest(&ctx.home, n).ok())
        .map(|g| g.modules)
        .unwrap_or_default();

    for name in &order {
        let span = info_span!("module", name = name.as_str());
        let _entered = span.enter();
        let module = &ir.modules[name.as_str()];
        let steps = &steps_by_module[name.as_str()];
        modules.insert(
            name.clone(),
            run_module(name, module, steps, ctx, modules.get(name.as_str()))?,
        );
    }

    // Satisfied = the module states are identical (the generation
    // number is not part of the comparison — 0008 §3).
    let next = current_gen.unwrap_or(0) + 1;
    if current_gen
        .and_then(|n| store::read_manifest(&ctx.home, n).ok())
        .map(|g| g.modules)
        .as_ref()
        == Some(&modules)
    {
        return Ok(Outcome::Satisfied {
            generation: current_gen,
        });
    }
    let generation = store::Generation {
        number: next,
        modules,
    };
    store::write_manifest(&ctx.home, &generation)?;
    store::flip(&ctx.home, next)?;
    info!(generation = next, "activated");
    Ok(Outcome::Applied { generation: next })
}

/// DAG order restricted to `only` + their transitive dependencies.
fn scoped_order(ir: &Ir, only: &[String]) -> Result<Vec<String>, ExecError> {
    let order = crate::build_order(ir)?;
    if only.is_empty() {
        return Ok(order);
    }
    let mut wanted: BTreeSet<&str> = only.iter().map(String::as_str).collect();
    let mut frontier: Vec<&str> = only.iter().map(String::as_str).collect();
    while let Some(name) = frontier.pop() {
        if let Some(m) = ir.modules.get(name) {
            for dep in &m.depends {
                if wanted.insert(dep.module.as_str()) {
                    frontier.push(dep.module.as_str());
                }
            }
        }
    }
    Ok(order
        .into_iter()
        .filter(|n| wanted.contains(n.as_str()))
        .collect())
}

/// Run one module's steps; returns its state for the manifest.
fn run_module(
    name: &str,
    module: &gripsack_ir::Module,
    steps: &[Step],
    ctx: &Ctx,
    prev: Option<&store::ModuleState>,
) -> Result<store::ModuleState, ExecError> {
    let store_path = store::store_path(&ctx.home, name, &module_input(module, &ctx.repo)?);
    // Satisfaction (0008 §3): presence is proof — skip fetch and build.
    let present = store_path.exists();
    let mut staging: Option<PathBuf> = None;
    let mut pending_verifies: Vec<&Verify> = Vec::new();

    // Phase A: produce the payload (fetch/build/custom steps).
    if !present {
        for step in steps {
            match &step.action {
                StepAction::Fetch { fetch: spec } => {
                    let stage = staging.get_or_insert_with(|| fresh_staging(name));
                    fetch(spec, stage).map_err(ExecError::Fetch)?;
                    info!(step = %step.id, "fetched");
                }
                StepAction::Build {
                    spec: Build::CustomShell { script },
                }
                | StepAction::CustomShell { script, .. } => {
                    let dir = staging.clone().unwrap_or_else(|| fresh_staging(name));
                    run_shell(script, &dir).map_err(|detail| ExecError::Step {
                        module: name.to_string(),
                        step: step.id.clone(),
                        detail,
                    })?;
                }
                _ => {}
            }
            if let Some(verify) = &step.verify {
                pending_verifies.push(verify);
            }
        }
        let stage = staging.take().unwrap_or_else(|| fresh_staging(name));
        // Config/install content can live in the repo (dotfiles travel
        // with the env repo — 0006): copy referenced files into staging.
        for entry in module.install.iter().chain(module.config.iter()) {
            let repo_file = ctx.repo.join(&entry.from);
            if repo_file.is_file() {
                let dest = stage.join(&entry.from);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&repo_file, &dest)?;
            }
        }
        store::publish_dir(&stage, &store_path)?;
    } else {
        for step in steps {
            if let Some(verify) = &step.verify {
                pending_verifies.push(verify);
            }
        }
    }

    // Phase B: deploy + verify against the published store path.
    let mut deployed = Vec::new();
    for step in steps {
        match &step.action {
            StepAction::Install { entries } | StepAction::ConfigDeploy { entries } => {
                for entry in entries {
                    deploy_entry(&mut deployed, &store_path, entry, ctx, prev)?;
                }
            }
            StepAction::Verify { verify } => run_verify(name, verify, &store_path)?,
            StepAction::Intent { action } => {
                // Activation adapters are 0.2 (0001 §3.8); declared
                // intents are recorded, not yet executed.
                info!(?action, "intent declared (not yet executed)");
            }
            _ => {}
        }
    }
    for verify in pending_verifies {
        run_verify(name, verify, &store_path)?;
    }
    Ok(store::ModuleState {
        store_path,
        entries: deployed,
    })
}

fn fresh_staging(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gripsack-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Deploy one entry (owned symlink or tracked copy) and record it.
///
/// tracked_copy drift handling (0001 §3.7): if the destination exists,
/// differs from the store copy, AND differs from what the previous
/// generation deployed, the user owns the change — keep it and warn,
/// never silently overwrite.
fn deploy_entry(
    out: &mut Vec<store::DeployedEntry>,
    store_path: &Path,
    entry: &Entry,
    ctx: &Ctx,
    prev: Option<&store::ModuleState>,
) -> Result<(), ExecError> {
    let source = resolve_source(store_path, &entry.from, &ctx.repo);
    let dest = expand_home(&entry.to);
    let hash = store::canonical_file_hash(&source)?;
    match &entry.mode {
        Ownership::Owned => {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            store::symlink_replace(&dest, &source)?;
        }
        Ownership::TrackedCopy => {
            let prev_hash = prev
                .and_then(|m| m.entries.iter().find(|e| e.to == entry.to))
                .map(|e| e.hash.as_str());
            if dest.exists() {
                let current = store::canonical_file_hash(&dest)?;
                if current == hash {
                    // already deployed, nothing to do
                } else if prev_hash == Some(current.as_str()) || prev_hash.is_none() && false {
                    // destination matches the previous generation: normal update
                    store::atomic_write(&dest, &std::fs::read(&source)?)?;
                } else if prev_hash.is_none() {
                    // first deploy over an existing file the user wrote
                    tracing::warn!(
                        "{}",
                        format!(
                            "{} exists and was not deployed by gripsack — keeping it (adopt into the module to manage)",
                            dest.display()
                        )
                    );
                } else {
                    tracing::warn!(
                        "{}",
                        format!(
                            "{} drifted since the last generation — keeping it",
                            dest.display()
                        )
                    );
                }
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                store::atomic_write(&dest, &std::fs::read(&source)?)?;
            }
        }
        other => {
            return Err(ExecError::Step {
                module: entry.from.clone(),
                step: "deploy".into(),
                detail: format!("ownership mode {other:?} lands in 0.2"),
            });
        }
    }
    out.push(store::DeployedEntry {
        from: entry.from.clone(),
        to: entry.to.clone(),
        mode: entry.mode.clone(),
        hash,
    });
    Ok(())
}

/// Entry content lives in the store payload if present, else in the
/// repo (config files travel with the env repo — 0006).
fn resolve_source(store_path: &Path, from: &str, repo: &Path) -> PathBuf {
    let in_store = store_path.join(from);
    if in_store.exists() {
        in_store
    } else {
        repo.join(from)
    }
}

fn expand_home(to: &str) -> PathBuf {
    if let Some(rest) = to.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(to)
}

/// The store-path input: module IR + canonical hashes of repo config
/// files, so editing a dotfile changes the identity (0008 §2).
fn module_input(module: &gripsack_ir::Module, repo: &Path) -> Result<String, ExecError> {
    let mut input = serde_json::to_string(module)?;
    for entry in module.install.iter().chain(module.config.iter()) {
        let repo_file = repo.join(&entry.from);
        if repo_file.exists() {
            input.push('|');
            input.push_str(&entry.from);
            input.push('=');
            input.push_str(&store::canonical_file_hash(&repo_file)?);
        }
    }
    Ok(input)
}

fn run_verify(name: &str, verify: &Verify, store_path: &Path) -> Result<(), ExecError> {
    let fail = |detail: String| ExecError::Verify {
        module: name.to_string(),
        detail,
    };
    match verify {
        Verify::BinaryRuns { path, args } => {
            let bin = store_path.join(path);
            let status = std::process::Command::new(&bin)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|e| fail(format!("cannot run {}: {e}", bin.display())))?;
            if !status.success() {
                return Err(fail(format!("{} exited {status}", bin.display())));
            }
        }
        Verify::FileExists { path } => {
            if !store_path.join(path).exists() {
                return Err(fail(format!("{} missing in payload", path)));
            }
        }
        Verify::Shell { script } => run_shell(script, store_path).map_err(fail)?,
    }
    Ok(())
}

fn run_shell(script: &str, cwd: &Path) -> Result<(), String> {
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("sh -c exited {status}"))
    }
}
