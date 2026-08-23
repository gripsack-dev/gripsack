//! The executor (0001 §4, 0007 §5): run a validated IR graph against
//! the store and deploy — one new generation per apply, or "already
//! satisfied" when nothing changed (0008 §3).
//!
//! v0.1: sequential execution in DAG order. The ready-queue scheduler
//! with resource locks replaces the loop without changing semantics.

use crate::expand;
use gripsack_fetch::{FetchError, fetch};
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
    /// Host name — selects the lockfile (`locks/<host>.lock`).
    pub host: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Nothing changed; no generation created.
    Satisfied { generation: Option<u64> },
    /// A new generation was deployed and activated.
    Applied { generation: u64 },
}

/// One user-visible line of what a step did — the CLI renders these.
#[derive(Debug, Clone, PartialEq)]
pub struct StepReport {
    pub module: String,
    pub summary: String,
    pub kind: ReportKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Fetched,
    Installed,
    Configured,
    Verified,
    Satisfied,
    Warned,
}

/// The result of an apply: outcome + the reports for the CLI.
#[derive(Debug)]
pub struct ApplyResult {
    pub outcome: Outcome,
    pub reports: Vec<StepReport>,
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
pub fn apply(ir: &Ir, ctx: &Ctx) -> Result<ApplyResult, ExecError> {
    let order = scoped_order(ir, &ctx.only)?;
    let steps_by_module = expand::expand_all(&ir.modules);
    let mut reports = Vec::new();
    let mut lock = crate::lockfile::read(&ctx.repo, &ctx.host).unwrap_or_default();
    let mut lock_dirty = false;

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
        let (state, module_reports, entry) = run_module(
            name,
            module,
            steps,
            ctx,
            modules.get(name.as_str()),
            lock.modules.get(name.as_str()),
        )?;
        reports.extend(module_reports);
        if let Some(entry) = entry
            && lock.modules.get(name.as_str()) != Some(&entry)
        {
            lock.modules.insert(name.clone(), entry);
            lock_dirty = true;
        }
        modules.insert(name.clone(), state);
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
        return Ok(ApplyResult {
            outcome: Outcome::Satisfied {
                generation: current_gen,
            },
            reports,
        });
    }
    let generation = store::Generation {
        number: next,
        modules,
    };
    store::write_manifest(&ctx.home, &generation)?;
    store::flip(&ctx.home, next)?;
    if lock_dirty {
        crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;
    }
    info!(generation = next, "activated");
    Ok(ApplyResult {
        outcome: Outcome::Applied { generation: next },
        reports,
    })
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
    locked: Option<&crate::lockfile::LockEntry>,
) -> Result<
    (
        store::ModuleState,
        Vec<StepReport>,
        Option<crate::lockfile::LockEntry>,
    ),
    ExecError,
> {
    // The payload hash participates in the store-path identity from
    // the very first apply (0008 §5) — resolve it before the existence
    // check or the first and second applies would compute different paths.
    let resolved = locked.and_then(|e| e.sha256.clone()).or_else(|| {
        module
            .fetch
            .as_ref()
            .and_then(|s| gripsack_fetch::payload_hash(s).ok().flatten())
    });
    let input = match &resolved {
        Some(sha) => format!("{}|payload={sha}", module_input(module, &ctx.repo)?),
        None => module_input(module, &ctx.repo)?,
    };
    let store_path = store::store_path(&ctx.home, name, &input);
    let mut lock_entry: Option<crate::lockfile::LockEntry> = None;
    // Satisfaction (0008 §3): presence is proof — skip fetch and build.
    let present = store_path.exists();
    let mut reports = Vec::new();
    if present {
        reports.push(StepReport {
            module: name.to_string(),
            summary: "payload already in store".into(),
            kind: ReportKind::Satisfied,
        });
    }
    let mut staging: Option<PathBuf> = None;
    let mut pending_verifies: Vec<&Verify> = Vec::new();

    // Phase A: produce the payload (fetch/build/custom steps).
    if !present {
        for step in steps {
            match &step.action {
                StepAction::Fetch { fetch: spec } => {
                    let stage = staging.get_or_insert_with(|| fresh_staging(name));
                    // the lockfile's hash wins for verification (0002 §3)
                    let spec = &inject_locked_sha(spec, locked);
                    let sha = fetch(spec, stage).map_err(ExecError::Fetch)?;
                    lock_entry = Some(crate::lockfile::LockEntry {
                        fetch: spec.clone(),
                        sha256: Some(sha),
                    });
                    info!(step = %step.id, "fetched");
                    reports.push(StepReport {
                        module: name.to_string(),
                        summary: format!("fetched {}", describe_fetch(spec)),
                        kind: ReportKind::Fetched,
                    });
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
                    let (summary, kind) =
                        deploy_entry(&mut deployed, &store_path, entry, ctx, prev)?;
                    reports.push(StepReport {
                        module: name.to_string(),
                        summary,
                        kind,
                    });
                }
            }
            StepAction::Verify { verify } if !present => {
                run_verify(name, verify, &store_path)?;
                reports.push(StepReport {
                    module: name.to_string(),
                    summary: describe_verify(verify),
                    kind: ReportKind::Verified,
                });
            }
            StepAction::Intent { action } => {
                // Activation adapters are 0.2 (0001 §3.8); declared
                // intents are recorded, not yet executed.
                info!(?action, "intent declared (not yet executed)");
            }
            _ => {}
        }
    }
    if !present {
        for verify in pending_verifies {
            run_verify(name, verify, &store_path)?;
            reports.push(StepReport {
                module: name.to_string(),
                summary: describe_verify(verify),
                kind: ReportKind::Verified,
            });
        }
    }
    Ok((
        store::ModuleState {
            store_path,
            entries: deployed,
        },
        reports,
        lock_entry,
    ))
}

/// A locked hash overrides the spec's for verification.
fn inject_locked_sha(
    spec: &gripsack_ir::FetchSpec,
    locked: Option<&crate::lockfile::LockEntry>,
) -> gripsack_ir::FetchSpec {
    let Some(entry) = locked else {
        return spec.clone();
    };
    let Some(sha) = &entry.sha256 else {
        return spec.clone();
    };
    match spec.clone() {
        gripsack_ir::FetchSpec::Tarball { url, .. } => gripsack_ir::FetchSpec::Tarball {
            url,
            sha256: Some(sha.clone()),
        },
        other => other,
    }
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
) -> Result<(String, ReportKind), ExecError> {
    let source = resolve_source(store_path, &entry.from, &ctx.repo);
    let dest = expand_home(&entry.to);
    let hash = store::canonical_file_hash(&source)?;
    let (summary, kind) = match &entry.mode {
        Ownership::Owned => {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            store::symlink_replace(&dest, &source)?;
            (
                format!("linked {} → {}", entry.from, entry.to),
                ReportKind::Installed,
            )
        }
        Ownership::TrackedCopy => {
            let prev_hash = prev
                .and_then(|m| m.entries.iter().find(|e| e.to == entry.to))
                .map(|e| e.hash.as_str());
            if dest.exists() {
                let current = store::canonical_file_hash(&dest)?;
                if current == hash {
                    (format!("{} unchanged", entry.to), ReportKind::Satisfied)
                } else if prev_hash == Some(current.as_str()) {
                    store::atomic_write(&dest, &std::fs::read(&source)?)?;
                    (
                        format!("updated {} → {}", entry.from, entry.to),
                        ReportKind::Configured,
                    )
                } else {
                    let note = if prev_hash.is_none() {
                        format!("{} exists (not deployed by gripsack) — kept", entry.to)
                    } else {
                        format!("{} drifted — kept", entry.to)
                    };
                    tracing::warn!("{}", note);
                    (note, ReportKind::Warned)
                }
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                store::atomic_write(&dest, &std::fs::read(&source)?)?;
                (
                    format!("copied {} → {}", entry.from, entry.to),
                    ReportKind::Configured,
                )
            }
        }
        other => {
            return Err(ExecError::Step {
                module: entry.from.clone(),
                step: "deploy".into(),
                detail: format!("ownership mode {other:?} lands in 0.2"),
            });
        }
    };
    out.push(store::DeployedEntry {
        from: entry.from.clone(),
        to: entry.to.clone(),
        mode: entry.mode.clone(),
        hash,
    });
    Ok((summary, kind))
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

fn describe_fetch(spec: &gripsack_ir::FetchSpec) -> String {
    use gripsack_ir::FetchSpec as F;
    match spec {
        F::GithubRelease { repo, asset, .. } => format!("github-release {repo} · {asset}"),
        F::Tarball { url, .. } => format!("tarball {url}"),
        F::Git { url, rev } => format!("git {url} @ {rev}"),
        F::File { path } => format!("file {path}"),
        F::Plugin { name, .. } => format!("plugin gripfetch-{name}"),
    }
}

fn describe_verify(verify: &Verify) -> String {
    match verify {
        Verify::BinaryRuns { path, .. } => format!("verified {path} runs"),
        Verify::FileExists { path } => format!("verified {path} exists"),
        Verify::Shell { .. } => "verified (shell check)".to_string(),
    }
}

fn expand_home(to: &str) -> PathBuf {
    if let Some(rest) = to.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
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

/// One line of an update report.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateReport {
    pub module: String,
    pub status: UpdateStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Unchanged,
    /// New or bumped pin — apply to deploy it.
    Bumped {
        old: Option<String>,
        new: String,
    },
    /// Resolution for this fetch kind lands in 0.2 (github_release, git).
    Skipped,
}

/// Re-resolve and rewrite the lockfile — the only mutator of it
/// (0008 §5). `grip update` never deploys; apply does.
pub fn update(ir: &Ir, ctx: &Ctx) -> Result<Vec<UpdateReport>, ExecError> {
    let order = scoped_order(ir, &ctx.only)?;
    let mut lock = crate::lockfile::read(&ctx.repo, &ctx.host).unwrap_or_default();
    let mut reports = Vec::new();
    for name in &order {
        let module = &ir.modules[name.as_str()];
        let Some(spec) = &module.fetch else {
            continue;
        };
        let old = lock
            .modules
            .get(name.as_str())
            .and_then(|e| e.sha256.clone());
        match spec {
            gripsack_ir::FetchSpec::File { .. } | gripsack_ir::FetchSpec::Tarball { .. } => {
                // resolve the payload hash without deploying
                let sha = gripsack_fetch::payload_hash(spec)
                    .map_err(ExecError::Fetch)?
                    .expect("file/tarball always hash");
                if old.as_deref() == Some(sha.as_str()) {
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Unchanged,
                    });
                } else {
                    lock.modules.insert(
                        name.clone(),
                        crate::lockfile::LockEntry {
                            fetch: spec.clone(),
                            sha256: Some(sha.clone()),
                        },
                    );
                    reports.push(UpdateReport {
                        module: name.clone(),
                        status: UpdateStatus::Bumped { old, new: sha },
                    });
                }
            }
            _ => {
                reports.push(UpdateReport {
                    module: name.clone(),
                    status: UpdateStatus::Skipped,
                });
            }
        }
    }
    crate::lockfile::write(&ctx.repo, &ctx.host, &lock)?;
    Ok(reports)
}
