//! Native profile files use the existing store, ownership planner, journal,
//! generation publication and rollback. No builder or alternate transaction.
mod content;
mod preview;

use crate::ctx::{Ctx, ExecError};
use crate::deploy::{DeploymentInput, deploy_entry};
use crate::report::{ReportKind, StepReport};
use gripsack_ir::workspace::{Workspace, WorkspaceDestination, WorkspaceOutput};
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
    pub(crate) fn prepare(
        workspace: &Workspace,
        repo: &Path,
        home: &Path,
        selected: &[String],
        limits: gripsack_fetch::FetchLimits,
    ) -> Result<Self, ExecError> {
        let wanted: BTreeSet<&str> = selected.iter().map(String::as_str).collect();
        for name in &wanted {
            if !workspace
                .outputs
                .iter()
                .any(|output| output.name() == *name)
            {
                return Err(file_failure(
                    &workspace.span,
                    format!("no workspace profile named {name:?}"),
                ));
            }
        }
        // Selection never hides a physical alias in another declared profile.
        // Destination admission needs no source capture or immutable publication.
        crate::expand::check_destinations(
            workspace
                .outputs
                .iter()
                .filter_map(|output| match output {
                    WorkspaceOutput::Profile(profile) => Some(profile),
                    _ => None,
                })
                .flat_map(|profile| {
                    profile.files.iter().map(move |file| {
                        let (path, block) = match &file.destination {
                            WorkspaceDestination::Symlink { path }
                            | WorkspaceDestination::TrackedCopy { path } => (path, None),
                            WorkspaceDestination::ManagedBlock { path, marker } => {
                                (path, Some(marker.to_lowercase()))
                            }
                        };
                        crate::expand::DestinationDeclaration {
                            owner: &profile.name,
                            path,
                            span: Some(&file.span),
                            block,
                        }
                    })
                }),
        )?;
        let mut capture = content::Capture::new(repo, limits)?;
        let mut profiles = BTreeMap::new();
        for (index, output) in workspace.outputs.iter().enumerate() {
            if !wanted.is_empty() && !wanted.contains(output.name()) {
                continue;
            }
            let WorkspaceOutput::Profile(profile) = output else {
                return Err(file_failure(
                    output.span(),
                    "selected output is not a native profile",
                ));
            };
            let stage = capture.temporary.path().join(format!("profile-{index}"));
            std::fs::create_dir(&stage)?;
            let mut files = Vec::with_capacity(profile.files.len());
            for file in &profile.files {
                files.push(
                    content::prepare_file(&mut capture, file, &stage)
                        .map_err(|error| file_failure(&file.span, error))?,
                );
            }
            let tree = store::canonical_tree_hash(&stage)?;
            let store_path = store::content_path(home, "workspace-files", tree.as_str());
            profiles.insert(
                profile.name.clone(),
                PreparedProfile {
                    span: profile.span.clone(),
                    stage,
                    store_path,
                    tree,
                    files,
                },
            );
        }
        Ok(Self {
            _capture: capture.temporary,
            profiles,
            partial: !selected.is_empty(),
        })
    }

    pub(crate) fn layout_evidence(&self) -> Vec<(String, crate::LayoutEvidence)> {
        self.profiles
            .iter()
            .map(|(name, profile)| {
                (
                    name.clone(),
                    crate::LayoutEvidence {
                        checked_paths: profile.files.len(),
                        deferred: Vec::new(),
                    },
                )
            })
            .collect()
    }

    pub(crate) fn update_reports(&self) -> Vec<crate::UpdateReport> {
        self.layout_evidence()
            .into_iter()
            .map(|(name, layout)| crate::UpdateReport {
                module: name,
                status: crate::UpdateStatus::Skipped {
                    reason: "native profile files are captured directly; no external pins",
                },
                layout,
            })
            .collect()
    }

    /// All file preparation and all immutable publication complete before the
    /// first destination is touched. Caller holds the existing lifecycle lock.
    pub(crate) fn deploy(
        &self,
        ctx: &Ctx,
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
                    env: vec![],
                    tree256: Some(profile.tree.to_string()),
                    build_closure: vec![],
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
