//! The required workspace catalog executes through the existing store, ownership
//! planner, journal, generation publication and rollback. Profile changes admit
//! migration against the current generation while retaining directory safety.
mod acquire;
mod artifact;
pub(crate) mod conda;
mod consumer;
pub(crate) mod hooks;
mod image;
mod inputs;
pub(crate) mod integrity;
mod lowering;
mod prepare;
mod realize;
pub use consumer::{
    AttemptId, BusyPolicy, InvocationId, TaskExecutionBinding, TaskNodeReport, TaskOutputMode,
    TaskPlan, TaskReport, TaskRevision, TaskRunPolicy, TaskStatus, TaskStdin, plan_task,
    read_task_log, read_task_report, run_task,
};
pub use consumer::{ConsumerOutcome, ConsumerRequest, consume, run_package_command};
pub use realize::{BuildOptions, BuildResult, BuiltOutput, build_workspace};
mod content;
mod declarations;
mod definitions;
mod directory_changes;
mod pins;
mod preview;
mod profile;
mod profile_checks;
mod profile_launchers;
mod profile_preparation;
mod retained;
pub(crate) use profile::prepare_apply;
pub(crate) mod roots;
mod selection;
mod solve;
mod stage;
pub(crate) mod update;

use crate::ctx::{Ctx, ExecError};
use crate::deploy::{DeploymentInput, deploy_entry};
use crate::report::{ReportKind, StepReport};
use gripsack_ir::{Diagnostic, Span, codes};
use gripsack_store as store;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

struct PreparedProfile {
    span: Span,
    stage: PathBuf,
    store_path: PathBuf,
    tree: store::hash::PayloadHash,
    files: Vec<content::PreparedFile>,
    env: Vec<store::EnvironmentContribution>,
    build_closure: Vec<PathBuf>,
    deferred: Vec<preview::DeferredFile>,
    deferred_environment: Option<String>,
    deferred_hooks: Vec<String>,
    deferred_checks: Vec<gripsack_ir::workspace_model::WorkspaceFileCheck>,
    intents: Vec<store::IntentRecord>,
    pre_flip_checks: Vec<profile_checks::PreparedCheck>,
}

pub(crate) struct NativeProfiles {
    _capture: tempfile::TempDir,
    profiles: BTreeMap<String, PreparedProfile>,
    partial: bool,
    deadline: std::time::Instant,
}

pub(crate) struct DeploymentOutcome {
    pub deployments: BTreeMap<String, store::DeploymentState>,
    pub reports: Vec<(String, Vec<StepReport>)>,
}

pub(super) fn file_failure(span: &Span, error: impl std::fmt::Display) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::EXEC_STEP, error.to_string())
            .with_label(Some(span.clone()), "profile file prepared here"),
    )
}

impl NativeProfiles {
    pub(crate) fn run_pre_flip_checks(
        &self,
        ctx: &Ctx,
        session: &crate::LifecycleSession,
    ) -> Result<(), ExecError> {
        if self
            .profiles
            .values()
            .all(|profile| profile.pre_flip_checks.is_empty())
        {
            return Ok(());
        }
        let environment = gripsack_process::OperatorEnvironment::capture()?;
        for profile in self.profiles.values() {
            if profile.pre_flip_checks.is_empty() {
                continue;
            }
            let closure = roots::RetentionSet::admit(
                session,
                profile
                    .build_closure
                    .iter()
                    .cloned()
                    .chain(std::iter::once(profile.store_path.clone())),
            )?;
            let protection = profile_checks::Protection {
                session,
                closure: &closure,
            };
            for check in &profile.pre_flip_checks {
                check.run(
                    &ctx.home,
                    &profile.store_path,
                    &environment,
                    self.deadline,
                    &protection,
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn layout_evidence(&self) -> Vec<(String, crate::LayoutEvidence)> {
        self.profiles
            .iter()
            .map(|(name, profile)| {
                (
                    name.clone(),
                    crate::LayoutEvidence {
                        checked_paths: profile.files.len(),
                        deferred: profile
                            .deferred
                            .iter()
                            .map(|file| crate::DeferredLayoutCheck::RecipeOutput(file.note()))
                            .chain(profile.deferred_environment.iter().map(|_| {
                                crate::DeferredLayoutCheck::RuntimeVerification(
                                    "profile environment activation",
                                )
                            }))
                            .chain(profile.deferred_hooks.iter().map(|_| {
                                crate::DeferredLayoutCheck::RuntimeVerification(
                                    "profile hook activation",
                                )
                            }))
                            .chain(profile.deferred_checks.iter().map(|_| {
                                crate::DeferredLayoutCheck::RuntimeVerification(
                                    "profile file check",
                                )
                            }))
                            .collect(),
                    },
                )
            })
            .collect()
    }

    pub(crate) fn activation_intents(&self) -> Vec<store::activation::PendingIntent> {
        self.profiles
            .iter()
            .flat_map(|(name, profile)| {
                profile
                    .intents
                    .iter()
                    .filter(|intent| intent.trigger != gripsack_store::ActivationTrigger::OnRemove)
                    .map(move |intent| store::activation::PendingIntent {
                        output: name.clone(),
                        action: intent.action.clone(),
                        trigger: intent.trigger,
                    })
            })
            .collect()
    }

    pub(crate) fn migration(
        &self,
        home: &Path,
        expected: Option<store::GenerationId>,
        previous: Option<&store::Generation>,
    ) -> Result<Option<crate::deploy::Migration>, ExecError> {
        let Some(expected) = expected else {
            return Ok(None);
        };
        let previous = previous.ok_or_else(|| {
            ExecError::Gate(Diagnostic::error(
                codes::EXEC_STEP,
                "generation-bound migration requires an admitted current generation",
            ))
        })?;
        let destinations = self.profiles.values().flat_map(|profile| {
            profile
                .files
                .iter()
                .map(|file| (file.entry.to.as_str(), &file.entry.mode))
                .chain(
                    profile
                        .deferred
                        .iter()
                        .map(preview::DeferredFile::migration_destination),
                )
        });
        crate::deploy::Migration::admit(
            home,
            expected,
            store::current_generation(home)?,
            previous,
            destinations,
        )
        .map(Some)
    }

    /// All file preparation and all immutable publication complete before the
    /// first destination is touched. Caller holds the existing lifecycle lock.
    pub(crate) fn deploy(
        &self,
        ctx: &Ctx,
        journal: &store::journal::JournalRun<'_>,
        previous: &BTreeMap<String, store::DeploymentState>,
        mut migration: Option<&mut crate::deploy::Migration>,
    ) -> Result<DeploymentOutcome, ExecError> {
        let mut reports = Vec::new();
        for (name, profile) in &self.profiles {
            let present = match std::fs::symlink_metadata(&profile.store_path) {
                Ok(metadata) if metadata.is_dir() => {
                    let actual = store::canonical_tree_hash(&profile.store_path)?;
                    if actual != profile.tree {
                        return Err(file_failure(
                            &profile.span,
                            "retained profile content hash differs; refusing to replace it",
                        ));
                    }
                    true
                }
                Ok(_) => {
                    return Err(file_failure(
                        &profile.span,
                        "profile store root is not a directory",
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(file_failure(&profile.span, error)),
            };
            if !present {
                crate::source::publish(ctx, name, &profile.stage, &profile.store_path)
                    .map_err(|error| file_failure(&profile.span, error))?;
            }
            reports.push((
                name.clone(),
                vec![StepReport {
                    output: name.clone(),
                    summary: if present {
                        "profile content reused".into()
                    } else {
                        "profile content published".into()
                    },
                    kind: if present {
                        ReportKind::Satisfied
                    } else {
                        ReportKind::Fetched
                    },
                }],
            ));
        }
        if let Some(migration) = migration.as_deref_mut() {
            crate::util::crash_hook("directory-before-publish");
            migration.publish_directories(journal)?;
            crate::util::crash_hook("directory-after-publish");
        }
        self.prepare_directory_changes(ctx, journal, previous)?;
        let lineage = crate::deploy::previous_ownership(previous);
        let mut deployments = BTreeMap::new();
        #[cfg(debug_assertions)]
        let mut completed_files = 0usize;
        for (name, profile) in &self.profiles {
            let mut entries = Vec::with_capacity(profile.files.len());
            let mut file_reports = Vec::new();
            for file in &profile.files {
                let (summary, kind) = deploy_entry(
                    &mut entries,
                    ctx,
                    DeploymentInput {
                        journal,
                        owner: name,
                        entry: &file.entry,
                        store_path: &profile.store_path,
                        previous: &lineage,
                        block_id: file.block_id.as_ref(),
                        migration: migration.as_deref(),
                    },
                )
                .map_err(|error| {
                    file_failure(file.entry.span.as_ref().unwrap_or(&profile.span), error)
                })?;
                file_reports.push(StepReport {
                    output: name.clone(),
                    summary,
                    kind,
                });
                #[cfg(debug_assertions)]
                {
                    completed_files += 1;
                    crate::util::crash_hook(&format!("workspace-file:{completed_files}"));
                }
            }
            reports.push((name.clone(), file_reports));
            deployments.insert(
                name.clone(),
                store::DeploymentState {
                    store_path: profile.store_path.clone(),
                    build_only: false,
                    entries,
                    intents: profile.intents.clone(),
                    verified: None,
                    env: profile.env.clone(),
                    tree256: Some(profile.tree.to_string()),
                    build_closure: profile.build_closure.clone(),
                },
            );
        }
        Ok(DeploymentOutcome {
            deployments,
            reports,
        })
    }
}
