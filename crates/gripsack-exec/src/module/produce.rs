//! Acquisition and artifact recipes; lifecycle publication stays in the parent.

use super::ModuleRun;
use crate::{
    ctx::ExecError,
    lockfile,
    report::{ReportKind, StepReport, describe_fetch},
    resolve::{module_input, resolve_spec},
    util::{fresh_staging, progress},
    verify::run_shell,
};
use gripsack_ir::{Build, Step, StepAction};
use gripsack_store as store;
use tracing::info;

impl ModuleRun<'_> {
    /// Phase A: fetch and build steps, into staging. Skipped entirely
    /// when satisfied (presence is proof).
    pub(super) fn produce(&mut self) -> Result<(), ExecError> {
        for step in self.plan.steps() {
            if self.present {
                continue;
            }
            let _guards = self.acquire_step(step)?;
            match &step.action {
                StepAction::Fetch { fetch: spec } => self.fetch_step(step, spec)?,
                StepAction::Build {
                    spec: Build::CustomShell { script },
                } => self.build_step(step, script, &[])?,
                StepAction::CustomShell { script, outputs } => {
                    self.build_step(step, script, outputs)?
                }
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
                module_input(
                    self.name,
                    self.module,
                    &self.ctx.repo,
                    self.ir,
                    self.lock,
                    self.plan
                )?
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
                crate::resolve::repo_overlay(self.plan, &self.ctx.repo)?,
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
        let mut command = std::process::Command::new(program);
        command.args(args).current_dir(&workdir);
        command.envs(env);
        self.build_env.apply(&mut command).map_err(fail)?;
        let status = command
            .status()
            .map_err(|e| fail(format!("cannot spawn {program}: {e}")))?;
        if !status.success() {
            return Err(fail(format!("{program} exited {status}")));
        }
        self.check_outputs(step, &dir, outputs)
    }

    fn check_outputs(
        &self,
        step: &Step,
        dir: &std::path::Path,
        outputs: &[String],
    ) -> Result<(), ExecError> {
        for output in outputs {
            if !dir.join(output).exists() {
                return Err(ExecError::Step {
                    module: self.name.into(),
                    step: step.id.clone(),
                    detail: format!("declared output {output:?} missing after the recipe ran"),
                });
            }
        }
        Ok(())
    }

    fn build_step(
        &mut self,
        step: &Step,
        script: &str,
        outputs: &[String],
    ) -> Result<(), ExecError> {
        progress(self.ctx, self.name, "building");
        let dir = self
            .staging
            .get_or_insert_with(|| fresh_staging(self.name))
            .clone();
        // get_or_insert persists the dir: publish's fresh_staging must
        // never wipe what a fetchless build/run step just produced
        std::fs::create_dir_all(&dir)?;
        // the build closure rides the step env (0039): PATH gains the
        // deps' bin dirs, GRIP_DEP_* names their store roots
        run_shell(script, &dir, Some(&self.build_env)).map_err(|detail| ExecError::Step {
            module: self.name.to_string(),
            step: step.id.clone(),
            detail,
        })?;
        self.check_outputs(step, &dir, outputs)
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
