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
    ir: &'a gripsack_ir::Ir,
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
}

/// Identity errors (new) escape before anything deploys; phase errors
/// land in `outcome.error` with partial deployments recorded.
pub(crate) fn run_module<'a>(
    name: &str,
    module: &'a gripsack_ir::Module,
    ir: &'a gripsack_ir::Ir,
    steps: &'a [Step],
    ctx: &'a Ctx,
    prev: Option<&'a store::ModuleState>,
    locked: Option<&'a lockfile::LockEntry>,
) -> Result<ModuleOutcome, ExecError> {
    let mut run = ModuleRun::new(name, module, ir, steps, ctx, prev, locked)?;
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
        ir: &'a gripsack_ir::Ir,
        steps: &'a [Step],
        ctx: &'a Ctx,
        prev: Option<&'a store::ModuleState>,
        locked: Option<&'a lockfile::LockEntry>,
    ) -> Result<Self, ExecError> {
        // 0014 §3: content is fully determined before execution unless
        // a build/custom/run step exists. Fetches pin content via the
        // lock's tree256; config-only modules hash their repo sources
        // at plan time. Anything else is input-addressed (recipe-named,
        // plan-time-computable, not content-guaranteed).
        let content_addressed = !steps.iter().any(|s| {
            matches!(
                s.action,
                StepAction::Build { .. } | StepAction::CustomShell { .. } | StepAction::Run { .. }
            )
        });
        // the fetch spec lives in module.fetch (declarative) or in a
        // fetch step (explicit steps) — check both
        let fetch_spec = module.fetch.as_ref().or_else(|| {
            steps.iter().find_map(|s| match &s.action {
                StepAction::Fetch { fetch } => Some(fetch),
                _ => None,
            })
        });
        // a changed spec cannot trust the locked tree for presence:
        // the recipe left the path, so one re-fetch must PROVE byte
        // identity — publish dedups if it matches (the mirror swap)
        let spec_changed = match (locked, fetch_spec) {
            (Some(entry), Some(spec)) => entry.fetch != *spec,
            _ => false,
        };
        // Repo-overlay drift: the locked tree256 names the MERGED
        // staging (fetch payload + repo config files), so a config tree
        // that gains a file moves nothing the transport pin can see.
        // Compare the overlay half or a warm store would deploy stale
        // content. Locks predating repo256 with repo-sourced froms
        // drift once and heal at the next publish.
        let pinned = locked.and_then(|e| e.resolved.as_ref());
        let repo_drift = !spec_changed
            && pinned.is_some_and(|r| r.tree256.is_some())
            && match (
                crate::resolve::repo_overlay(module, &ctx.repo)?,
                pinned.and_then(|r| r.repo256.as_ref()),
            ) {
                (Some(current), Some(lock)) => current != *lock,
                // an old lock can't vouch for the overlay — distrust
                (Some(_), None) => true,
                (None, _) => false,
            };
        let (store_path, identity_pending, tree256) = if content_addressed {
            let locked_tree = if spec_changed || repo_drift {
                None
            } else {
                locked
                    .and_then(|e| e.resolved.as_ref())
                    .and_then(|r| r.tree256.clone())
            };
            match locked_tree {
                Some(tree) => (
                    store::content_path(&ctx.home, name, &tree),
                    false,
                    Some(tree),
                ),
                None if fetch_spec.is_none() => {
                    // config-only: content is the repo's payload sources,
                    // computable without staging (overlay == staged tree)
                    let froms: Vec<String> = module
                        .install
                        .iter()
                        .chain(module.config.iter())
                        .map(|e| e.from.clone())
                        .collect();
                    let tree = store::canonical_overlay_hash(&ctx.repo, &froms)?;
                    (
                        store::content_path(&ctx.home, name, &tree),
                        false,
                        Some(tree),
                    )
                }
                None => {
                    // deferred: the transport hash cannot name an
                    // unextracted tree — the first fetch finalizes the
                    // path at publish (0002 §3 TOFU)
                    let input = module_input(module, &ctx.repo, ir)?;
                    (store::store_path(&ctx.home, name, &input), true, None)
                }
            }
        } else {
            let resolved = locked
                .and_then(|e| e.resolved.as_ref())
                .and_then(|r| r.sha256.clone())
                .or_else(|| {
                    fetch_spec.and_then(|s| gripsack_fetch::payload_hash(s).ok().flatten())
                });
            let input = match &resolved {
                Some(sha) => format!("{}|payload={sha}", module_input(module, &ctx.repo, ir)?),
                None => module_input(module, &ctx.repo, ir)?,
            };
            let path = store::store_path(&ctx.home, name, &input);
            // Deferred identity (finding C): no hash from the lock AND
            // none computable offline — the first fetch's sha finalizes
            // the path. Presence is meaningless until then: always fetch.
            (path, resolved.is_none() && fetch_spec.is_some(), None)
        };
        let present = store_path.exists() && !identity_pending;
        let mut reports = Vec::new();
        if present {
            reports.push(StepReport {
                module: name.to_string(),
                summary: if content_addressed {
                    "content already in store".into()
                } else {
                    "payload already in store".into()
                },
                kind: ReportKind::Satisfied,
            });
        }
        Ok(ModuleRun {
            name,
            module,
            ir,
            steps,
            ctx,
            prev,
            locked,
            version: locked
                .and_then(|e| e.resolved.as_ref())
                .and_then(|r| r.version.clone()),
            store_path,
            content_addressed,
            tree256,
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
                // "not implemented" must be loud, always (the cardinal
                // rule the docs name: silent skip is the enemy) — a
                // schema'd kind the core can't execute is an error, not
                // a no-op (0007 §1, review finding D)
                StepAction::Build { spec } => {
                    return Err(ExecError::Step {
                        module: self.name.to_string(),
                        step: step.id.clone(),
                        detail: format!(
                            "build kind {spec:?} is not executable by this core — \
                             CustomShell is the implemented build kind today"
                        ),
                    });
                }
                StepAction::Run {
                    argv,
                    env,
                    cwd,
                    outputs,
                } => self.run_step(step, argv, env, cwd.as_deref(), outputs)?,
                _ => {} // Install/ConfigDeploy/Intent/Verify belong to other phases
            }
        }
        Ok(())
    }

    fn fetch_step(&mut self, step: &Step, spec: &gripsack_ir::FetchSpec) -> Result<(), ExecError> {
        progress(self.ctx, self.name, "fetching");
        let stage = self.staging.get_or_insert_with(|| fresh_staging(self.name));
        // a changed fetch spec invalidates the lock entry — the args
        // are the declaration, the pin follows them, never the reverse
        // (a spec edit must not fail as "the mirror changed", which
        // costs a confused ten minutes hand-editing the lockfile)
        let spec_changed = self.locked.is_some_and(|e| e.fetch != *spec);
        let locked = if spec_changed {
            tracing::info!(
                module = self.name,
                "fetch spec changed — re-resolving the pin"
            );
            None
        } else {
            self.locked
        };
        // resolve to a concrete spec — the locked pin wins; else
        // resolve now (trust on first use, 0002 §3)
        let (concrete, meta) = resolve_spec(self.name, spec, locked)?;
        if let Some(m) = &meta {
            self.version = Some(m.version.clone());
        }
        // Plugin fetchers learn the pin — first-fetch (resolve, TOFU)
        // and pinned re-fetch (reproduce) are different code paths for
        // internal registries (0002 §4 `locked`).
        let locked_json = locked
            .and_then(|e| e.resolved.as_ref())
            .and_then(|r| serde_json::to_value(r).ok());
        let outcome = gripsack_fetch::fetch_with_locked(&concrete, stage, locked_json.as_ref())
            .map_err(|e| match e {
                // plugin diagnostics keep their envelope (0009 §2 —
                // they render through the one renderer at apply)
                gripsack_fetch::FetchError::Diagnostics(_) => ExecError::Fetch(e),
                // everything else becomes a step error so the apply
                // renderer can point at the module line (0004 §3)
                other => ExecError::Step {
                    module: self.name.to_string(),
                    step: step.id.clone(),
                    detail: other.to_string(),
                },
            })?;
        let sha = outcome.hash.clone();
        // Finalize a deferred identity (finding C): the first fetch's
        // sha joins the store-path input — identical to what the lock
        // gives every later apply. Presence was never checked against
        // the provisional path, so this is the path publish must use.
        // Input-addressed only: content-addressed modules (0014)
        // finalize at publish, where the merged staging's tree exists.
        if self.identity_pending && !self.content_addressed {
            let input = format!(
                "{}|payload={sha}",
                module_input(self.module, &self.ctx.repo, self.ir)?
            );
            self.store_path = store::store_path(&self.ctx.home, self.name, &input);
        }
        // pin enforcement for kinds without download-level verification
        if let Some(expected) = locked
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
        let pin = locked.and_then(|e| e.resolved.as_ref());
        self.lock_entry = Some(lockfile::LockEntry {
            fetch: spec.clone(),
            resolved: Some(fetched_pin(
                meta.as_ref(),
                &outcome,
                &concrete,
                pin,
                sha,
                crate::resolve::repo_overlay(self.module, &self.ctx.repo)?,
            )),
        });
        info!(step = %step.id, "fetched");
        self.reports.push(StepReport {
            module: self.name.to_string(),
            summary: format!("fetched {}", describe_fetch(spec)),
            kind: ReportKind::Fetched,
        });
        Ok(())
    }

    /// A structured action (0007 §3 rung 2): spawn argv in the staging
    /// dir — no shell, no quoting bugs — with declared env overrides;
    /// declared outputs are the contract, checked after the run.
    fn run_step(
        &mut self,
        step: &Step,
        argv: &[String],
        env: &std::collections::BTreeMap<String, String>,
        cwd: Option<&str>,
        outputs: &[String],
    ) -> Result<(), ExecError> {
        progress(self.ctx, self.name, "running");
        let fail = |detail: String| ExecError::Step {
            module: self.name.to_string(),
            step: step.id.clone(),
            detail,
        };
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| fail("run step needs argv (empty array)".into()))?;
        let dir = self
            .staging
            .get_or_insert_with(|| fresh_staging(self.name))
            .clone();
        std::fs::create_dir_all(&dir)?;
        let workdir = cwd.map(|c| dir.join(c)).unwrap_or_else(|| dir.clone());
        let status = std::process::Command::new(program)
            .args(args)
            .current_dir(&workdir)
            .envs(env)
            .status()
            .map_err(|e| fail(format!("cannot spawn {program}: {e}")))?;
        if !status.success() {
            return Err(fail(format!("{program} exited {status}")));
        }
        // declared outputs are the contract (0008 §4): run produced
        // exactly these, no more guessing
        for output in outputs {
            if !dir.join(output).exists() {
                return Err(fail(format!(
                    "declared output {output:?} missing after {program} ran"
                )));
            }
        }
        Ok(())
    }

    fn build_step(&mut self, step: &Step, script: &str) -> Result<(), ExecError> {
        progress(self.ctx, self.name, "building");
        let dir = self
            .staging
            .get_or_insert_with(|| fresh_staging(self.name))
            .clone();
        // get_or_insert persists the dir: publish's fresh_staging must
        // never wipe what a fetchless build/run step just produced
        std::fs::create_dir_all(&dir)?;
        run_shell(script, &dir).map_err(|detail| ExecError::Step {
            module: self.name.to_string(),
            step: step.id.clone(),
            detail,
        })
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
        for entry in self.module.install.iter().chain(self.module.config.iter()) {
            let repo_file = self.ctx.repo.join(&entry.from);
            let is_real_dir =
                repo_file.is_dir() && !repo_file.symlink_metadata()?.file_type().is_symlink();
            if is_real_dir {
                // a directory `from` stages recursively (symlinks
                // recreated, matching canonical_overlay_hash) — deploy
                // must never link the repo checkout itself
                store::copy_dir(&repo_file, &stage.join(&entry.from))?;
            } else if repo_file.is_file() {
                let dest = stage.join(&entry.from);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&repo_file, &dest)?;
            }
        }
        if !self.content_addressed {
            store::publish_dir(&stage, &self.store_path)?;
            return Ok(());
        }
        let tree = store::canonical_tree_hash(&stage)?;
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
            store::publish_dir(&stage, &path)?;
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
                tree256: self.tree256,
            },
            reports: self.reports,
            lock_entry: self.lock_entry,
            error: self.error,
        }
    }
}

/// Build the pin a fetch records. `meta` is a fresh resolution — None
/// on a pinned re-fetch, where the lock's own fields ARE the pin and
/// must survive the rewrite: dropping version breaks {version}
/// substitution on the next warm-store deploy, and dropping
/// url/api_url forces a re-resolve through the registry API on the
/// next cold store.
fn fetched_pin(
    meta: Option<&gripsack_fetch::ResolvedRelease>,
    outcome: &gripsack_fetch::fetch::FetchOutcome,
    concrete: &gripsack_ir::FetchSpec,
    pin: Option<&lockfile::Resolved>,
    sha: String,
    repo256: Option<String>,
) -> lockfile::Resolved {
    lockfile::Resolved {
        // tree256 lands at publish, with the merged staging — never
        // carried over from the old pin
        tree256: None,
        // a plugin's reported pin (upstream artifact url + version) is
        // recorded so the next apply's `locked` tells it exactly what
        // to reproduce; for resolved kinds, the resolution's own
        // metadata; else the surviving lock fields
        url: meta
            .map(|m| m.url.clone())
            .or_else(|| outcome.plugin_url.clone())
            .or_else(|| pin.and_then(|r| r.url.clone())),
        // git floats pin the resolved rev as the lock's version
        // (0016 §D2) — the float re-reads it on every apply
        version: meta
            .map(|m| m.version.clone())
            .or_else(|| outcome.plugin_version.clone())
            .or_else(|| match concrete {
                gripsack_ir::FetchSpec::Git { rev, .. } => rev.clone(),
                _ => None,
            })
            .or_else(|| pin.and_then(|r| r.version.clone())),
        sha256: Some(sha),
        api_url: meta
            .and_then(|m| m.api_url.clone())
            .or_else(|| pin.and_then(|r| r.api_url.clone())),
        // the repo-overlay half of the merged tree — presence checks
        // and `grip update` compare it to catch config trees that
        // change under an unmoved transport pin
        repo256,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locked_pin() -> lockfile::Resolved {
        lockfile::Resolved {
            url: Some("https://ghe.invalid/rel/asset.tar.gz".into()),
            version: Some("15.2.0".into()),
            sha256: Some("ab".repeat(32)),
            tree256: Some("ef".repeat(32)),
            api_url: Some("https://ghe.invalid/api/asset/1".into()),
            repo256: None,
        }
    }

    fn outcome() -> gripsack_fetch::fetch::FetchOutcome {
        gripsack_fetch::fetch::FetchOutcome {
            hash: "ab".repeat(32),
            plugin_url: None,
            plugin_version: None,
        }
    }

    #[test]
    fn pinned_refetch_preserves_the_locks_pin_fields() {
        let locked = locked_pin();
        let concrete = gripsack_ir::FetchSpec::Tarball {
            url: locked.url.clone().unwrap(),
            sha256: locked.sha256.clone(),
            api_url: locked.api_url.clone(),
        };
        let got = fetched_pin(
            None,
            &outcome(),
            &concrete,
            Some(&locked),
            "ab".repeat(32),
            Some("cd".repeat(32)),
        );
        assert_eq!(got.url, locked.url);
        assert_eq!(got.version, locked.version);
        assert_eq!(got.api_url, locked.api_url);
        assert_eq!(got.repo256, Some("cd".repeat(32)));
        // the tree belongs to the OLD merge — publish re-pins it
        assert_eq!(got.tree256, None);
    }

    #[test]
    fn fresh_resolution_wins_over_the_lock() {
        let locked = locked_pin();
        let meta = gripsack_fetch::ResolvedRelease {
            version: "16.0.0".into(),
            url: "https://ghe.invalid/rel/new.tar.gz".into(),
            api_url: Some("https://ghe.invalid/api/asset/2".into()),
            sha256: None,
        };
        let concrete = gripsack_ir::FetchSpec::Tarball {
            url: meta.url.clone(),
            sha256: None,
            api_url: meta.api_url.clone(),
        };
        let got = fetched_pin(
            Some(&meta),
            &outcome(),
            &concrete,
            Some(&locked),
            "ab".repeat(32),
            None,
        );
        assert_eq!(
            got.url.as_deref(),
            Some("https://ghe.invalid/rel/new.tar.gz")
        );
        assert_eq!(got.version.as_deref(), Some("16.0.0"));
        assert_eq!(
            got.api_url.as_deref(),
            Some("https://ghe.invalid/api/asset/2")
        );
    }
}
