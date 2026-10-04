//! Native profile files use the existing store, ownership planner, journal,
//! generation publication and rollback. No builder or alternate transaction.
mod acquire;
mod artifact;
pub(crate) mod conda;
mod consumer;
mod image;
mod inputs;
mod lowering;
mod prepare;
mod realize;
pub use consumer::{ConsumerOutcome, ConsumerRequest, consume};
pub use realize::{BuildOptions, BuildResult, BuiltOutput, build_workspace};
mod content;
mod declarations;
mod definitions;
mod pins;
mod preview;
mod profile;
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
use gripsack_ir::workspace::WorkspaceDestination;
use gripsack_ir::{Diagnostic, Span, codes};
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
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
}

pub(crate) struct NativeProfiles {
    _capture: tempfile::TempDir,
    profiles: BTreeMap<String, PreparedProfile>,
    partial: bool,
}

pub(super) fn file_failure(span: &Span, error: impl std::fmt::Display) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::EXEC_STEP, error.to_string())
            .with_label(Some(span.clone()), "profile file prepared here"),
    )
}

impl NativeProfiles {
    pub(crate) fn prepare<'a>(
        ir: &'a gripsack_ir::Ir,
        repo: &Path,
        home: &Path,
        selected: &[String],
        limits: gripsack_fetch::FetchLimits,
        realization: Option<&realize::Realization<'a>>,
    ) -> Result<Option<Self>, ExecError> {
        Self::prepare_with(ir, repo, home, selected, limits, realization, false)
    }

    fn prepare_with<'a>(
        ir: &'a gripsack_ir::Ir,
        repo: &Path,
        home: &Path,
        selected: &[String],
        limits: gripsack_fetch::FetchLimits,
        realization: Option<&realize::Realization<'a>>,
        readonly: bool,
    ) -> Result<Option<Self>, ExecError> {
        let Some(declarations) = declarations::Declarations::from_ir(ir) else {
            return Ok(None);
        };
        let wanted: BTreeSet<&str> = selected.iter().map(String::as_str).collect();
        for name in &wanted {
            if !declarations
                .outputs()
                .any(|output| output.name() == *name && output.files().is_some())
            {
                return Err(file_failure(
                    declarations.span,
                    format!("no workspace profile named {name:?}"),
                ));
            }
        }
        // Selection never hides a physical alias in another declared profile.
        crate::expand::check_destinations(
            declarations
                .outputs()
                .filter_map(|output| output.files().map(|files| (output.name(), files)))
                .flat_map(|(owner, files)| {
                    files.destinations().map(move |(span, destination)| {
                        let (path, block) = match destination {
                            WorkspaceDestination::Symlink { path }
                            | WorkspaceDestination::TrackedCopy { path } => (path, None),
                            WorkspaceDestination::ManagedBlock { path, marker } => {
                                (path, Some(marker.to_lowercase()))
                            }
                        };
                        crate::expand::DestinationDeclaration {
                            owner,
                            path,
                            span: Some(span),
                            block,
                        }
                    })
                }),
        )?;
        let outputs: BTreeMap<_, _> = ir
            .workspace_v6
            .as_ref()
            .into_iter()
            .flat_map(|workspace| &workspace.outputs)
            .map(|output| (output.name(), output))
            .collect();
        let host = realization
            .filter(|_| !readonly)
            .map(|_| consumer::admit::NativeContext::new(&ir.host, home))
            .transpose()?;
        let mut capture = content::Capture::new(repo, limits)?;
        let mut profiles = BTreeMap::new();
        for (index, output) in declarations
            .outputs()
            .filter(|output| output.files().is_some())
            .enumerate()
        {
            if !wanted.is_empty() && !wanted.contains(output.name()) {
                continue;
            }
            output.admit_profile()?;
            let declarations = output.files().ok_or_else(|| {
                file_failure(output.span(), "selected output is not a native profile")
            })?;
            let stage = capture.temporary.path().join(format!("profile-{index}"));
            std::fs::create_dir(&stage)?;
            let declarations = declarations.captured(realization, readonly);
            let mut files = Vec::with_capacity(declarations.size_hint().0);
            let mut deferred = Vec::new();
            for declaration in declarations {
                let file = match declaration? {
                    declarations::Captured::Known(file) => file,
                    declarations::Captured::Deferred(file) => {
                        deferred.push(preview::DeferredFile::new(file)?);
                        continue;
                    }
                };
                content::prepare_into(&mut capture, &file, &stage, &mut files)
                    .map_err(|error| file_failure(file.span, error))?;
            }
            let deferred_environment = output.environment().filter(|_| readonly).map(str::to_owned);
            let env = if let Some(name) = output.environment().filter(|_| !readonly) {
                let realized = realization.ok_or_else(|| {
                    file_failure(
                        output.span(),
                        "profile environment requires protected package realization",
                    )
                })?;
                let Some(gripsack_ir::workspace_v6::WorkspaceOutput::Environment(environment)) =
                    outputs.get(name).copied()
                else {
                    return Err(file_failure(
                        output.span(),
                        "profile environment is absent from the current catalog",
                    ));
                };
                let environment = consumer::admit::EnvironmentPlan::admit(
                    environment,
                    &outputs,
                    realized,
                    host.as_ref().expect("realization admits host"),
                )?;
                let directory = stage.join("commands");
                std::fs::create_dir(&directory)?;
                environment.write_commands(&directory)?;
                environment.profile_env()?
            } else {
                Vec::new()
            };
            let tree = store::canonical_tree_hash(&stage)?;
            let store_path = store::content_path(home, "workspace-files", tree.as_str());
            profiles.insert(
                output.name().to_owned(),
                PreparedProfile {
                    span: output.span().clone(),
                    stage,
                    store_path,
                    tree,
                    files,
                    env,
                    build_closure: Vec::new(),
                    deferred,
                    deferred_environment,
                },
            );
        }
        crate::expand::check_destinations(profiles.iter().flat_map(|(owner, profile)| {
            profile
                .files
                .iter()
                .map(move |file| crate::expand::DestinationDeclaration {
                    owner,
                    path: &file.entry.to,
                    span: file.entry.span.as_ref().or(Some(&profile.span)),
                    block: file
                        .block_id
                        .as_ref()
                        .map(|identity| identity.as_str().to_owned()),
                })
        }))?;
        Ok(Some(Self {
            _capture: capture.temporary,
            profiles,
            partial: !selected.is_empty(),
        }))
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
                            .collect(),
                    },
                )
            })
            .collect()
    }

    pub(crate) fn update_reports(&self) -> Result<crate::UpdateSurvey, ExecError> {
        let mut reports = crate::report::SurveyReports::new(self.profiles.len());
        for (name, profile) in &self.profiles {
            reports.push(crate::UpdateReport {
                module: name.clone(),
                status: crate::UpdateStatus::Skipped {
                    reason: "native profile files are captured directly; no external pins",
                },
                layout: crate::LayoutEvidence {
                    checked_paths: profile.files.len(),
                    deferred: Vec::new(),
                },
            })?;
        }
        reports.finish()
    }

    /// All file preparation and all immutable publication complete before the
    /// first destination is touched. Caller holds the existing lifecycle lock.
    pub(crate) fn deploy(
        &self,
        ctx: &Ctx,
        journal: &store::journal::JournalRun<'_>,
        previous: &BTreeMap<String, store::ModuleState>,
    ) -> Result<crate::schedule::ScheduleOutcome, ExecError> {
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
                    module: name.clone(),
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
        let lineage = crate::schedule::previous_ownership(previous);
        let mut modules = BTreeMap::new();
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
                        version: None,
                        block_id: file.block_id.as_ref(),
                    },
                )
                .map_err(|error| {
                    file_failure(file.entry.span.as_ref().unwrap_or(&profile.span), error)
                })?;
                file_reports.push(StepReport {
                    module: name.clone(),
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
            modules.insert(
                name.clone(),
                store::ModuleState {
                    store_path: profile.store_path.clone(),
                    build_only: false,
                    entries,
                    intents: vec![],
                    verified: None,
                    env: profile.env.clone(),
                    tree256: Some(profile.tree.to_string()),
                    build_closure: profile.build_closure.clone(),
                },
            );
        }
        Ok(crate::schedule::ScheduleOutcome {
            modules,
            reports,
            lock_entries: BTreeMap::new(),
            failed: None,
        })
    }
}
