//! The per-module lifecycle (0008 §3): a [`ModuleRun`] carries the
//! evolving state and walks small phase methods —
//!
//! ```text
//! new (identity + satisfaction) → produce → publish → deploy → verify
//! ```
//!
//! — collecting user-visible reports and the lockfile entry along the
//! way. The pattern follows cargo's UnitContext: one context struct,
//! no six-argument function threading.

use crate::ctx::{Ctx, ExecError};
use crate::deploy::deploy_entry;
use crate::lockfile;
use crate::report::{ReportKind, StepReport, describe_fetch, describe_verify};
use crate::resolve::{module_input, resolve_spec};
use crate::util::{fresh_staging, progress};
use crate::verify::{run_shell, run_verify};
use gripsack_ir::{Build, Step, StepAction, Verify};
use gripsack_store as store;
use std::path::PathBuf;
use tracing::info;

/// What one module's run produced — the manifest state, the reports
/// for the CLI, and the lockfile entry if a fetch happened.
pub(crate) struct ModuleOutcome {
    pub state: store::ModuleState,
    pub reports: Vec<StepReport>,
    pub lock_entry: Option<lockfile::LockEntry>,
    /// The first phase error, if any — the run stops there but the
    /// outcome keeps the deployments made so far, because apply's
    /// run-rollback (0001 §9, review finding E1) needs them to restore
    /// the previous state exactly.
    pub error: Option<ExecError>,
}

/// One module's execution context. Fields evolve as phases run.
struct ModuleRun<'a> {
    name: &'a str,
    module: &'a gripsack_ir::Module,
    steps: &'a [Step],
    ctx: &'a Ctx,
    prev: Option<&'a store::ModuleState>,
    locked: Option<&'a lockfile::LockEntry>,
    /// `{version}` substitution source — the locked pin or a resolution
    /// from this run (0008 §5).
    version: Option<String>,
    store_path: PathBuf,
    /// Satisfaction: the payload is already in the store.
    present: bool,
    staging: Option<PathBuf>,
    deployed: Vec<store::DeployedEntry>,
    reports: Vec<StepReport>,
    pending_verifies: Vec<&'a Verify>,
    lock_entry: Option<lockfile::LockEntry>,
    error: Option<ExecError>,
    /// Identity is finalized after fetch for kinds whose payload hash
    /// isn't knowable up front (pixi, git, plugin — finding C): the
    /// lock-independent path is provisional, and the first fetch's
    /// sha256 completes it — the same path every later apply computes
    /// from the lockfile.
    identity_pending: bool,
}

/// Identity errors (new) escape before anything deploys; phase errors
/// land in `outcome.error` with partial deployments recorded.
pub(crate) fn run_module(
    name: &str,
    module: &gripsack_ir::Module,
    steps: &[Step],
    ctx: &Ctx,
    prev: Option<&store::ModuleState>,
    locked: Option<&lockfile::LockEntry>,
) -> Result<ModuleOutcome, ExecError> {
    let mut run = ModuleRun::new(name, module, steps, ctx, prev, locked)?;
    for phase in [
        ModuleRun::produce,
        ModuleRun::publish,
        ModuleRun::deploy,
        ModuleRun::verify,
    ] {
        if run.error.is_some() {
            break;
        }
        if let Err(e) = phase(&mut run) {
            run.error = Some(e);
        }
    }
    Ok(run.finish())
}

impl<'a> ModuleRun<'a> {
    /// Identity and satisfaction: the payload hash joins the store-path
    /// input before the existence check, so first and second applies
    /// compute the same path (0008 §5).
    fn new(
        name: &'a str,
        module: &'a gripsack_ir::Module,
        steps: &'a [Step],
        ctx: &'a Ctx,
        prev: Option<&'a store::ModuleState>,
        locked: Option<&'a lockfile::LockEntry>,
    ) -> Result<Self, ExecError> {
        let resolved = locked
            .and_then(|e| e.resolved.as_ref())
            .and_then(|r| r.sha256.clone())
            .or_else(|| {
                // the fetch spec lives in module.fetch (declarative) or
                // in a fetch step (explicit steps) — check both
                let spec = module.fetch.as_ref().or_else(|| {
                    steps.iter().find_map(|s| match &s.action {
                        StepAction::Fetch { fetch } => Some(fetch),
                        _ => None,
                    })
                });
                spec.and_then(|s| gripsack_fetch::payload_hash(s).ok().flatten())
            });
        let input = match &resolved {
            Some(sha) => format!("{}|payload={sha}", module_input(module, &ctx.repo)?),
            None => module_input(module, &ctx.repo)?,
        };
        let store_path = store::store_path(&ctx.home, name, &input);
        // Deferred identity (finding C): no hash from the lock AND none
        // computable offline — the first fetch's sha will finalize the
        // path. Presence is meaningless until then: always fetch.
        let identity_pending = resolved.is_none()
            && (module.fetch.is_some()
                || steps
                    .iter()
                    .any(|s| matches!(s.action, gripsack_ir::StepAction::Fetch { .. })));
        let present = store_path.exists() && !identity_pending;
        let mut reports = Vec::new();
        if present {
            reports.push(StepReport {
                module: name.to_string(),
                summary: "payload already in store".into(),
                kind: ReportKind::Satisfied,
            });
        }
        Ok(ModuleRun {
            name,
            module,
            steps,
            ctx,
            prev,
            locked,
            version: locked
                .and_then(|e| e.resolved.as_ref())
                .and_then(|r| r.version.clone()),
            store_path,
            present,
            identity_pending,
            staging: None,
            deployed: Vec::new(),
            reports,
            pending_verifies: Vec::new(),
            lock_entry: None,
            error: None,
        })
    }

    /// Phase A: fetch and build steps, into staging. Skipped entirely
    /// when satisfied (presence is proof).
    /// Acquire the step's declared resources for exactly its duration
    /// (0007 §4, N4 — never the module's whole lifetime).
    fn acquire_step(&self, step: &Step) -> Result<Vec<crate::util::FlockGuard>, ExecError> {
        let mut guards = Vec::new();
        // sorted by the BTreeSet at the call site — a total order, no AB/BA
        let resources: std::collections::BTreeSet<&str> =
            step.resources.iter().map(String::as_str).collect();
        for resource in resources {
            guards.push(crate::util::FlockGuard::acquire(&self.ctx.home, resource)?);
        }
        Ok(guards)
    }

    fn produce(&mut self) -> Result<(), ExecError> {
        for step in self.steps {
            if let Some(verify) = &step.verify {
                self.pending_verifies.push(verify);
            }
            if self.present {
                continue;
            }
            let _guards = self.acquire_step(step)?;
            match &step.action {
                StepAction::Fetch { fetch: spec } => self.fetch_step(step, spec)?,
                StepAction::Build {
                    spec: Build::CustomShell { script },
                }
                | StepAction::CustomShell { script, .. } => self.build_step(step, script)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn fetch_step(&mut self, step: &Step, spec: &gripsack_ir::FetchSpec) -> Result<(), ExecError> {
        progress(self.ctx, self.name, "fetching");
        let stage = self.staging.get_or_insert_with(|| fresh_staging(self.name));
        // resolve to a concrete spec — the locked pin wins; else
        // resolve now (trust on first use, 0002 §3)
        let (concrete, meta) = resolve_spec(spec, self.locked)?;
        if let Some(m) = &meta {
            self.version = Some(m.version.clone());
        }
        // Plugin fetchers learn the pin — first-fetch (resolve, TOFU)
        // and pinned re-fetch (reproduce) are different code paths for
        // internal registries (0002 §4 `locked`).
        let locked_json = self
            .locked
            .and_then(|e| e.resolved.as_ref())
            .and_then(|r| serde_json::to_value(r).ok());
        let outcome = gripsack_fetch::fetch_with_locked(&concrete, stage, locked_json.as_ref())
            .map_err(ExecError::Fetch)?;
        let sha = outcome.hash;
        // Finalize a deferred identity (finding C): the first fetch's
        // sha joins the store-path input — identical to what the lock
        // gives every later apply. Presence was never checked against
        // the provisional path, so this is the path publish must use.
        if self.identity_pending {
            let input = format!(
                "{}|payload={sha}",
                module_input(self.module, &self.ctx.repo)?
            );
            self.store_path = store::store_path(&self.ctx.home, self.name, &input);
        }
        // pin enforcement for kinds without download-level verification
        if let Some(expected) = self
            .locked
            .and_then(|e| e.resolved.as_ref())
            .and_then(|r| r.sha256.as_ref())
            && sha != *expected
            && !matches!(concrete, gripsack_ir::FetchSpec::Tarball { .. })
        {
            return Err(ExecError::Fetch(gripsack_fetch::FetchError::HashMismatch {
                url: format!("{} payload", self.name),
                expected: expected.clone(),
                actual: sha,
            }));
        }
        self.lock_entry = Some(lockfile::LockEntry {
            fetch: spec.clone(),
            resolved: Some(lockfile::Resolved {
                // a plugin's reported pin (upstream artifact url +
                // version) is recorded so the next apply's `locked`
                // tells it exactly what to reproduce; for resolved
                // kinds, the resolution's own metadata
                url: meta.as_ref().map(|m| m.url.clone()).or(outcome.plugin_url),
                version: meta
                    .as_ref()
                    .map(|m| m.version.clone())
                    .or(outcome.plugin_version),
                sha256: Some(sha),
                api_url: meta.and_then(|m| m.api_url.clone()),
            }),
        });
        info!(step = %step.id, "fetched");
        self.reports.push(StepReport {
            module: self.name.to_string(),
            summary: format!("fetched {}", describe_fetch(spec)),
            kind: ReportKind::Fetched,
        });
        Ok(())
    }

    fn build_step(&mut self, step: &Step, script: &str) -> Result<(), ExecError> {
        progress(self.ctx, self.name, "building");
        let dir = self
            .staging
            .clone()
            .unwrap_or_else(|| fresh_staging(self.name));
        // fresh_staging only *clears* the path — a build with no fetch
        // step before it has no staging yet.
        std::fs::create_dir_all(&dir)?;
        run_shell(script, &dir).map_err(|detail| ExecError::Step {
            module: self.name.to_string(),
            step: step.id.clone(),
            detail,
        })
    }

    /// Stage repo-referenced files and publish into the store — once,
    /// immutably (0001 §9.1).
    fn publish(&mut self) -> Result<(), ExecError> {
        if self.present {
            return Ok(());
        }
        let stage = self
            .staging
            .take()
            .unwrap_or_else(|| fresh_staging(self.name));
        // fresh_staging only *clears* the path — the copy loop below is
        // what creates it, so a zero-file payload (e.g. a tree whose
        // last file was dropped) must create it explicitly or the
        // publish rename fails with ENOENT.
        std::fs::create_dir_all(&stage)?;
        for entry in self.module.install.iter().chain(self.module.config.iter()) {
            let repo_file = self.ctx.repo.join(&entry.from);
            if repo_file.is_file() {
                let dest = stage.join(&entry.from);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&repo_file, &dest)?;
            }
        }
        store::publish_dir(&stage, &self.store_path)?;
        Ok(())
    }

    /// Phase B: deploy entries and intents against the published store
    /// path. Deploy runs even when satisfied — it's idempotent and
    /// repairs drift.
    fn deploy(&mut self) -> Result<(), ExecError> {
        for step in self.steps {
            match &step.action {
                StepAction::Install { entries } | StepAction::ConfigDeploy { entries } => {
                    progress(self.ctx, self.name, "deploying");
                    let _guards = self.acquire_step(step)?;
                    for entry in entries {
                        let (summary, kind) = deploy_entry(
                            &mut self.deployed,
                            self.name,
                            &self.store_path,
                            entry,
                            self.ctx,
                            self.prev,
                            self.version.as_deref(),
                        )?;
                        self.reports.push(StepReport {
                            module: self.name.to_string(),
                            summary,
                            kind,
                        });
                    }
                }
                StepAction::Intent { action } => {
                    // Activation adapters are not yet executed (0001 §3.8); declared
                    // intents are recorded, not yet executed.
                    info!(?action, "intent declared (not yet executed)");
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Verify — only when the module actually built something (0008 §4:
    /// a no-op apply runs zero verifies).
    fn verify(&mut self) -> Result<(), ExecError> {
        if self.present {
            return Ok(());
        }
        for step in self.steps {
            if let StepAction::Verify { verify } = &step.action {
                progress(self.ctx, self.name, "verifying");
                run_verify(self.name, verify, &self.store_path, self.version.as_deref())?;
                self.reports.push(StepReport {
                    module: self.name.to_string(),
                    summary: describe_verify(verify, self.version.as_deref()),
                    kind: ReportKind::Verified,
                });
            }
        }
        for verify in self.pending_verifies.clone() {
            run_verify(self.name, verify, &self.store_path, self.version.as_deref())?;
            self.reports.push(StepReport {
                module: self.name.to_string(),
                summary: describe_verify(verify, self.version.as_deref()),
                kind: ReportKind::Verified,
            });
        }
        Ok(())
    }

    fn finish(self) -> ModuleOutcome {
        ModuleOutcome {
            state: store::ModuleState {
                store_path: self.store_path,
                entries: self.deployed,
                env: self.module.env.clone(),
            },
            reports: self.reports,
            lock_entry: self.lock_entry,
            error: self.error,
        }
    }
}
