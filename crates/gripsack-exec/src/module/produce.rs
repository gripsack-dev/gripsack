//! Acquisition and artifact recipes; lifecycle publication stays in the parent.

use super::ModuleRun;
use crate::{
    ctx::ExecError,
    report::{ReportKind, StepReport, describe_fetch},
    source,
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
            let _step = tracing::info_span!("step", step = %step.id).entered();
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
        let pin = source::fetch(
            self.ctx,
            source::FetchInputs {
                name: self.name,
                spec,
                locked: self.locked,
                dest: stage,
            },
        )
        .map_err(|error| match error {
            ExecError::Fetch(gripsack_fetch::FetchError::Diagnostics(_)) => error,
            other => ExecError::Step {
                module: self.name.into(),
                step: step.id.clone(),
                detail: other.to_string(),
            },
        })?;
        let resolved = pin.resolved.as_ref().expect("acquisition creates a pin");
        let sha = resolved
            .sha256
            .as_deref()
            .expect("acquisition hashes its payload");
        self.version = resolved.version.clone();
        // Finalize a deferred identity (finding C): the first fetch's
        // sha joins the store-path input — identical to what the lock
        // gives every later apply. Presence was never checked against
        // the provisional path, so this is the path publish must use.
        // Input-addressed only: content-addressed modules (0014)
        // finalize at publish, where the merged staging's tree exists.
        if self.identity_pending && !self.content_addressed {
            let input = format!("{}|payload={sha}", self.recipes.input(self.name, self.lock));
            self.store_path = store::store_path(&self.ctx.home, self.name, &input);
        }
        self.lock_entry = Some(pin);
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
