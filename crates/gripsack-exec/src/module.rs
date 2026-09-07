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

mod produce;
mod verify;

use crate::ctx::{Ctx, ExecError};
use crate::deploy::deploy_entry;
use crate::lockfile;
use crate::report::{ReportKind, StepReport};
use crate::util::{fresh_staging, progress};
use gripsack_ir::{Step, StepAction, prepared::PreparedModule};
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
    ir: &'a gripsack_ir::Ir,
    plan: &'a PreparedModule,
    ctx: &'a Ctx,
    prev_map: &'a std::collections::BTreeMap<std::path::PathBuf, &'a store::DeployedEntry>,
    /// The previous generation's record for THIS module — the
    /// verification receipt lives there (0035 F2)
    prev_module: Option<&'a store::ModuleState>,
    locked: Option<&'a lockfile::LockEntry>,
    /// The full lockfile — dependency pins ride the consumer's build
    /// key (0035 F4)
    lock: &'a lockfile::Lockfile,
    /// `{version}` substitution source — the locked pin or a resolution
    /// from this run (0008 §5).
    version: Option<String>,
    store_path: PathBuf,
    /// Satisfaction: the payload is already in the store.
    present: bool,
    staging: Option<PathBuf>,
    deployed: Vec<store::DeployedEntry>,
    reports: Vec<StepReport>,
    /// The verification receipt this run earned (0035 F2): Some(fps)
    /// after every declared check passed; None when nothing was
    /// declared or any failed (a failed run never commits anyway).
    verified: Option<Vec<String>>,
    lock_entry: Option<lockfile::LockEntry>,
    error: Option<ExecError>,
    /// Identity is finalized after fetch for kinds whose payload hash
    /// isn't knowable up front (pixi, git, plugin — finding C): the
    /// lock-independent path is provisional, and the first fetch's
    /// sha256 completes it — the same path every later apply computes
    /// from the lockfile. Content-addressed modules (0014) finalize at
    /// publish instead: the tree needs the merged staging.
    identity_pending: bool,
    /// 0014 §3: no build/custom/run step → the store path names the
    /// content itself.
    content_addressed: bool,
    /// The content identity: the expected tree hash from the lock (or
    /// the plan-time overlay for config-only), then the computed tree
    /// at publish. Recorded into the generation manifest for
    /// host-independent store verify.
    tree256: Option<String>,
    /// The build closure this module's produce-phase steps run in
    /// (0039): build-only deps' store paths as PATH prefix +
    /// GRIP_DEP_* vars. Empty for modules without build deps.
    build_env: crate::closure::BuildEnv,
    /// This module deploys nothing (0039): every incoming edge is a
    /// build edge. Produce/publish still run; deploy and destination
    /// verifies don't; the manifest retains only payload state and receipts.
    build_only: bool,
}

/// Identity errors (new) escape before anything deploys; phase errors
/// land in `outcome.error` with partial deployments recorded.
/// Everything one module run needs, named (0035): the IR slice, the
/// lineage records, and the lock. A struct, not nine positional
/// arguments — the signature says what a run consumes.
pub(crate) struct ModuleInputs<'a> {
    pub name: &'a str,
    pub module: &'a gripsack_ir::Module,
    pub ir: &'a gripsack_ir::Ir,
    pub plan: &'a PreparedModule,
    pub prev_map: &'a std::collections::BTreeMap<std::path::PathBuf, &'a store::DeployedEntry>,
    pub prev_module: Option<&'a store::ModuleState>,
    pub locked: Option<&'a lockfile::LockEntry>,
    pub lock: &'a lockfile::Lockfile,
    /// The closure env for produce-phase steps (0039).
    pub build_env: crate::closure::BuildEnv,
    /// This module is a build-only dependency (0039).
    pub build_only: bool,
}

pub(crate) fn run_module<'a>(
    inputs: ModuleInputs<'a>,
    ctx: &'a Ctx,
) -> Result<ModuleOutcome, ExecError> {
    let mut run = ModuleRun::new(inputs, ctx)?;
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
    fn new(inputs: ModuleInputs<'a>, ctx: &'a Ctx) -> Result<Self, ExecError> {
        let ModuleInputs {
            name,
            module,
            ir,
            plan,
            prev_map,
            prev_module,
            locked,
            lock,
            build_env,
            build_only,
        } = inputs;
        // identity is pure resolution (identity.rs) — this type is
        // the phase machine that executes against its answer
        let identity = crate::identity::resolve(crate::identity::IdentityInputs {
            name,
            module,
            ir,
            plan,
            home: &ctx.home,
            repo: &ctx.repo,
            locked,
            lock,
            mode: crate::identity::Resolution::Execute,
        })?;
        let reports = identity
            .satisfied_report(name)
            .into_iter()
            .collect::<Vec<_>>();
        Ok(ModuleRun {
            name,
            module,
            ir,
            plan,
            ctx,
            prev_map,
            prev_module,
            locked,
            lock,
            version: locked
                .and_then(|e| e.resolved.as_ref())
                .and_then(|r| r.version.clone()),
            store_path: identity.store_path,
            content_addressed: identity.content_addressed,
            tree256: identity.tree256,
            present: identity.present,
            identity_pending: identity.identity_pending,
            staging: None,
            deployed: Vec::new(),
            reports,
            verified: None,
            lock_entry: None,
            error: None,
            build_env,
            build_only,
        })
    }

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

    /// Publish staged bytes into the store through the home
    /// capability (plan/0021 phase 4): the destination is named
    /// RELATIVE to the pinned home inode, so a store path can never
    /// be redirected by a swapped path component.
    fn publish_staging(
        &self,
        stage: &std::path::Path,
        dest: &std::path::Path,
    ) -> Result<(), ExecError> {
        let rel = dest
            .strip_prefix(&self.ctx.home)
            .map_err(|_| ExecError::Step {
                module: self.name.to_string(),
                step: "publish".into(),
                detail: format!("store path {} is not under $GRIPSACK_HOME", dest.display()),
            })?;
        gripsack_fs::publish_dir(self.ctx.home_dir()?, stage, rel)?;
        Ok(())
    }

    /// Stage repo-referenced files and publish into the store — once,
    /// immutably (0001 §9.1). Content-addressed modules (0014) name
    /// the path from the merged staging's tree hash: an existing path
    /// IS the content, so publishing dedups by construction.
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
        for entry in self.plan.entries() {
            let repo_file = self.ctx.repo.join(&entry.from);
            let is_real_dir =
                repo_file.is_dir() && !repo_file.symlink_metadata()?.file_type().is_symlink();
            if is_real_dir {
                // a directory `from` stages recursively (symlinks
                // recreated, matching canonical_overlay_hash) — deploy
                // must never link the repo checkout itself
                gripsack_fs::copy_dir(&repo_file, &stage.join(&entry.from))?;
            } else if repo_file.is_file() {
                let dest = stage.join(&entry.from);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&repo_file, &dest)?;
            }
        }
        if !self.content_addressed {
            self.publish_staging(&stage, &self.store_path)?;
            return Ok(());
        }
        let tree = store::canonical_tree_hash(&stage)?.to_string();
        // drift the transport check can't see (plugin fetchers stage
        // trees directly): a locked identity must match what landed
        if let Some(expected) = &self.tree256
            && *expected != tree
        {
            return Err(ExecError::Fetch(gripsack_fetch::FetchError::HashMismatch {
                url: format!("{} store tree", self.name),
                expected: expected.clone(),
                actual: tree,
            }));
        }
        let path = store::content_path(&self.ctx.home, self.name, &tree);
        if path.exists() {
            // the mirror swap: re-fetch proved byte-identity, the path
            // is already there — drop staging, keep the store
            std::fs::remove_dir_all(&stage)?;
        } else {
            self.publish_staging(&stage, &path)?;
        }
        self.store_path = path;
        self.tree256 = Some(tree.clone());
        if let Some(entry) = &mut self.lock_entry
            && let Some(resolved) = &mut entry.resolved
        {
            resolved.tree256 = Some(tree);
        }
        Ok(())
    }

    /// Phase B: deploy entries and intents against the published store
    /// path. Deploy runs even when satisfied — it's idempotent and
    /// repairs drift.
    fn deploy(&mut self) -> Result<(), ExecError> {
        if self.build_only {
            // 0039: a build-only dependency deploys nothing — visible
            // as a report line, never a silent no-op. Its store path
            // rides the consumer's build_closure instead.
            self.reports.push(StepReport {
                module: self.name.to_string(),
                summary: "build-only dependency — staged for the build closure, not deployed"
                    .to_string(),
                kind: ReportKind::Satisfied,
            });
            return Ok(());
        }
        for step in self.plan.steps() {
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
                            self.prev_map,
                            self.version.as_deref(),
                        )?;
                        self.reports.push(StepReport {
                            module: self.name.to_string(),
                            summary,
                            kind,
                        });
                    }
                }
                StepAction::Intent { action, .. } => {
                    // step-form intents run through the activation
                    // adapters after the flip (routed by kind —
                    // activate.rs step_intents)
                    info!(?action, "intent declared (runs via activation adapters)");
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn finish(self) -> ModuleOutcome {
        ModuleOutcome {
            state: store::ModuleState {
                store_path: self.store_path,
                build_only: self.build_only,
                entries: self.deployed,
                intents: self
                    .plan
                    .steps()
                    .iter()
                    .filter(|_| !self.build_only)
                    .filter_map(|s| match &s.action {
                        StepAction::Intent { action, trigger } => Some(store::IntentRecord {
                            action: action.as_ref().clone(),
                            trigger: *trigger,
                        }),
                        _ => None,
                    })
                    .collect(),
                verified: self.verified,
                env: if self.build_only {
                    Vec::new()
                } else {
                    self.module.env.clone()
                },
                tree256: self.tree256,
                build_closure: self.build_env.into_paths(),
            },
            reports: self.reports,
            lock_entry: self.lock_entry,
            error: self.error,
        }
    }
}
