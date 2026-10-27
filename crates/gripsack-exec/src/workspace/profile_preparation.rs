//! Prepare immutable profile candidates and checks before destination mutation.
use super::{
    NativeProfiles, PreparedProfile, consumer, content, declarations, file_failure, hooks, preview,
    profile_checks, profile_launchers, realize,
};
use crate::ExecError;
use gripsack_ir::workspace::WorkspaceDestination;
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

impl NativeProfiles {
    pub(super) fn migration_predecessor(
        home: &Path,
    ) -> Result<Option<store::Generation>, ExecError> {
        store::current_generation(home)?
            .map(|current| {
                store::generations::admit_manifest(home, current).map_err(ExecError::from)
            })
            .transpose()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare<'a>(
        ir: &'a gripsack_ir::Ir,
        repository: &crate::Repository,
        home: &Path,
        selected: &[String],
        limits: gripsack_fetch::FetchLimits,
        realization: Option<&realize::Realization<'a>>,
    ) -> Result<Self, ExecError> {
        let predecessor = Self::migration_predecessor(home)?;
        Self::prepare_with(
            ir,
            repository,
            home,
            selected,
            limits,
            realization,
            false,
            std::time::Instant::now() + gripsack_process::Limits::default().timeout,
            predecessor.as_ref(),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_with<'a>(
        ir: &'a gripsack_ir::Ir,
        repository: &crate::Repository,
        home: &Path,
        selected: &[String],
        limits: gripsack_fetch::FetchLimits,
        realization: Option<&realize::Realization<'a>>,
        readonly: bool,
        deadline: std::time::Instant,
        predecessor: Option<&store::Generation>,
        protection: Option<&profile_checks::Protection<'_>>,
    ) -> Result<Self, ExecError> {
        let declarations = declarations::Declarations::from_ir(ir);
        let repo = repository.contents();
        let logical_source = |path: &str| match repository {
            crate::Repository::Direct(_) => path.to_owned(),
            crate::Repository::Evaluated { sources, .. } => sources.logical_text(path).into_owned(),
        };
        let resolve_destination = |path: &str| match predecessor {
            Some(previous) => crate::deploy::Migration::destination_for_predecessor(previous, path),
            None => store::canonical_dest(path),
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
        let mut launcher_destinations = Vec::new();
        for output in &ir.workspace.outputs {
            if let gripsack_ir::workspace_model::WorkspaceOutput::Profile(profile) = output {
                for path in profile_launchers::destinations(profile)? {
                    launcher_destinations.push((profile.name.as_str(), &profile.span, path));
                }
            }
        }
        // Selection never hides a physical alias in another declared profile.
        crate::deploy::check_destinations_with(
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
                        crate::deploy::DestinationDeclaration {
                            owner,
                            path,
                            span: Some(span),
                            block,
                        }
                    })
                })
                .chain(launcher_destinations.iter().map(|(owner, span, path)| {
                    crate::deploy::DestinationDeclaration {
                        owner,
                        path,
                        span: Some(span),
                        block: None,
                    }
                })),
            resolve_destination,
        )?;
        let outputs: BTreeMap<_, _> = ir
            .workspace
            .outputs
            .iter()
            .map(|output| (output.name(), output))
            .collect();
        let host = realization
            .filter(|_| !readonly)
            .map(|_| consumer::admit::NativeContext::new(&ir.host, home, deadline))
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
            let mut deferred_checks = Vec::new();
            let mut prepared_checks = profile_checks::PreparedChecks::default();
            for declaration in declarations {
                let file = match declaration? {
                    declarations::Captured::Known(file) => file,
                    declarations::Captured::Deferred(file) => {
                        deferred_checks.extend(file.checks.iter().cloned());
                        deferred.push(preview::DeferredFile::new(file)?);
                        continue;
                    }
                };
                let first_file = files.len();
                content::prepare_into(&mut capture, &file, &stage, &mut files)
                    .map_err(|error| file_failure(file.span, error))?;
                if readonly {
                    deferred_checks.extend(file.checks.iter().cloned());
                }
                if !readonly && !file.checks.is_empty() {
                    profile_checks::prepare(
                        file.checks,
                        &files[first_file..],
                        &outputs,
                        realization.ok_or_else(|| {
                            file_failure(file.span, "file checks require protected realization")
                        })?,
                        home,
                        &stage,
                        host.as_ref().ok_or_else(|| {
                            file_failure(file.span, "file checks require native admission")
                        })?,
                        deadline,
                        &resolve_destination,
                        &logical_source,
                        &mut prepared_checks,
                        protection.ok_or_else(|| {
                            file_failure(
                                file.span,
                                "file checks require a held lifecycle and retention set",
                            )
                        })?,
                    )?;
                }
            }
            if readonly
                && let Some(gripsack_ir::workspace_model::WorkspaceOutput::Profile(profile)) =
                    outputs.get(output.name()).copied()
                && let Some(environment) = &profile.environment
            {
                for path in profile_launchers::destinations(profile)? {
                    deferred.push(preview::DeferredFile::launcher(
                        path,
                        profile.span.clone(),
                        environment.clone(),
                    ));
                }
            }
            let deferred_environment = output.environment().filter(|_| readonly).map(str::to_owned);
            let deferred_hooks = if readonly {
                output.hooks().to_vec()
            } else {
                Vec::new()
            };
            let env = if let Some(name) = output.environment().filter(|_| !readonly) {
                let realized = realization.ok_or_else(|| {
                    file_failure(
                        output.span(),
                        "profile environment requires protected package realization",
                    )
                })?;
                let Some(gripsack_ir::workspace_model::WorkspaceOutput::Environment(environment)) =
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
                if let Some(gripsack_ir::workspace_model::WorkspaceOutput::Profile(profile)) =
                    outputs.get(output.name()).copied()
                {
                    profile_launchers::prepare(profile, &environment, home, &stage, &mut files)?;
                }
                environment.profile_env()?
            } else {
                Vec::new()
            };
            let mut prepared_hooks = if output.hooks().is_empty() || readonly {
                Vec::new()
            } else {
                hooks::prepare(
                    output.hooks(),
                    &outputs,
                    realization.ok_or_else(|| {
                        file_failure(output.span(), "hooks require protected realization")
                    })?,
                    home,
                    &stage,
                    host.as_ref().ok_or_else(|| {
                        file_failure(output.span(), "hooks require native admission")
                    })?,
                    deadline,
                )?
            };
            prepared_hooks.extend(prepared_checks.hooks);
            let tree = store::canonical_tree_hash(&stage)?;
            let store_path = store::content_path(home, "workspace-files", tree.as_str());
            let intents = prepared_hooks
                .into_iter()
                .map(|hook| hook.record(&store_path))
                .collect();
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
                    deferred_hooks,
                    deferred_checks,
                    intents,
                    pre_flip_checks: prepared_checks.pre_flip,
                },
            );
        }
        crate::deploy::check_destinations_with(
            profiles.iter().flat_map(|(owner, profile)| {
                profile
                    .files
                    .iter()
                    .map(move |file| crate::deploy::DestinationDeclaration {
                        owner,
                        path: &file.entry.to,
                        span: file.entry.span.as_ref().or(Some(&profile.span)),
                        block: file
                            .block_id
                            .as_ref()
                            .map(|identity| identity.as_str().to_owned()),
                    })
            }),
            resolve_destination,
        )?;
        Ok(Self {
            _capture: capture.temporary,
            profiles,
            partial: !selected.is_empty(),
            deadline,
        })
    }
}
