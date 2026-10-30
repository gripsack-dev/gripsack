//! Profile production precedes host mutation, but transfers its held native
//! lifecycle authority directly into the existing generation transaction.
use super::{
    NativeProfiles,
    realize::{self, BuildOptions},
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_ir::{
    Diagnostic, Ir, Span, codes,
    workspace_model::{ProfileOutput, WorkspaceOutput, WorkspaceSource},
};
use std::time::Instant;

impl NativeProfiles {
    /// Inspect frozen evidence only. No lifecycle session, worker, source
    /// acquisition, package materialization or activation is acquired here.
    pub(crate) fn prepare_preview(
        ir: &Ir,
        repository: &crate::Repository,
        home: &std::path::Path,
        selected: &[String],
        limits: gripsack_fetch::FetchLimits,
    ) -> Result<Self, ExecError> {
        let predecessor = Self::migration_predecessor(home)?;
        let mut origins = std::collections::BTreeSet::new();
        for output in &ir.workspace.outputs {
            if !selected.is_empty() && !selected.iter().any(|name| name == output.name()) {
                continue;
            }
            let WorkspaceOutput::Profile(profile) = output else {
                continue;
            };
            admit(profile)?;
            if let Some(environment) = &profile.environment {
                origins.insert(environment.as_str());
            }
            for file in &profile.files {
                if let Some(
                    WorkspaceSource::ArtifactFile { output, .. }
                    | WorkspaceSource::Tree { output, .. },
                ) = &file.source
                {
                    origins.insert(output.as_str());
                }
            }
        }
        let retained = super::retained::inspect(ir, repository, home, &origins, limits)?;
        Self::prepare_with(
            ir,
            repository,
            home,
            selected,
            limits,
            Some(&retained),
            true,
            Instant::now() + gripsack_process::Limits::default().timeout,
            predecessor.as_ref(),
            None,
        )
    }
}

pub(crate) fn prepare_apply(
    ir: &Ir,
    ctx: &Ctx,
) -> Result<(LifecycleSession, NativeProfiles), ExecError> {
    let session = LifecycleSession::acquire(&ctx.home)?;
    prepare_apply_in_session(ir, ctx, session)
}

pub(crate) fn prepare_apply_in_session(
    ir: &Ir,
    ctx: &Ctx,
    session: LifecycleSession,
) -> Result<(LifecycleSession, NativeProfiles), ExecError> {
    let workspace = &ir.workspace;
    let mut selected = Vec::new();
    let mut needs_production = false;
    for output in &workspace.outputs {
        if !ctx.only.is_empty() && !ctx.only.iter().any(|name| name == output.name()) {
            continue;
        }
        let WorkspaceOutput::Profile(profile) = output else {
            if ctx.only.is_empty() {
                continue;
            }
            return Err(unavailable(
                output.span(),
                "apply selects profile outputs, not production or invocation outputs",
            ));
        };
        admit(profile)?;
        needs_production |= !profile.hooks.is_empty()
            || !profile.schedules.is_empty()
            || profile.environment.is_some()
            || profile.files.iter().any(|file| !file.checks.is_empty())
            || profile.files.iter().any(|file| {
                matches!(
                    file.source,
                    Some(WorkspaceSource::ArtifactFile { .. } | WorkspaceSource::Tree { .. })
                )
            });
        selected.push(profile.name.clone());
    }
    for name in &ctx.only {
        if !selected.contains(name) {
            return Err(unavailable(
                &workspace.span,
                format!("no profile output named {name:?}"),
            ));
        }
    }
    if selected.is_empty() {
        return Err(unavailable(
            &workspace.span,
            "workspace declares no profile to apply",
        ));
    }
    if !needs_production {
        let mut native = NativeProfiles::prepare(
            ir,
            &ctx.repository,
            &ctx.home,
            &ctx.only,
            ctx.fetch.limits(),
            None,
        )?;
        retain_semantic_evidence(ir, ctx, &session, &mut native, None)?;
        return Ok((session, native));
    }
    let environment = gripsack_process::OperatorEnvironment::capture()?;
    let options = BuildOptions {
        environment: &environment,
        deadline: Instant::now() + gripsack_process::Limits::default().timeout,
    };
    let held = realize::realize_in_session(ir, ctx, &options, &selected, session)?;
    let predecessor = NativeProfiles::migration_predecessor(&ctx.home)?;
    let check_retention = if workspace.outputs.iter().any(|output| {
        matches!(output, WorkspaceOutput::Profile(profile)
            if selected.contains(&profile.name) && profile.files.iter().any(|file| !file.checks.is_empty()))
    }) {
        Some(super::roots::RetentionSet::admit(
            &held.session,
            held.closures.iter().flat_map(|(_, paths)| paths.iter().cloned()),
        )?)
    } else {
        None
    };
    let protection = check_retention
        .as_ref()
        .map(|closure| super::profile_checks::Protection {
            session: &held.session,
            closure,
        });
    let mut native = NativeProfiles::prepare_with(
        ir,
        &ctx.repository,
        &ctx.home,
        &ctx.only,
        ctx.fetch.limits(),
        Some(&held.realization),
        false,
        options.deadline,
        predecessor.as_ref(),
        protection.as_ref(),
    )?;
    for (name, paths) in &held.closures {
        let profile = native.profiles.get_mut(name).ok_or_else(|| {
            unavailable(
                &workspace.span,
                "realized profile is absent from the deployment selection",
            )
        })?;
        profile.build_closure.extend(paths.iter().cloned());
        profile.build_closure.sort();
        profile.build_closure.dedup();
    }
    retain_semantic_evidence(ir, ctx, &held.session, &mut native, Some(&held.realization))?;
    // The still-held lifecycle session now protects all immutable objects until
    // the caller's generation is durable. No permanent Output root is leaked.
    Ok((held.session, native))
}

fn retain_semantic_evidence(
    ir: &Ir,
    ctx: &Ctx,
    session: &LifecycleSession,
    native: &mut NativeProfiles,
    realization: Option<&super::realize::Realization<'_>>,
) -> Result<(), ExecError> {
    use crate::semantic::{capture_profile, retention};
    let pins = super::pins::WorkspacePins::read(&ctx.repository)?;
    let previous = gripsack_store::generations::current(&ctx.home)?
        .map(|id| gripsack_store::generations::admit_manifest(&ctx.home, id))
        .transpose()?;
    for (name, profile) in &mut native.profiles {
        let snapshot = capture_profile(ir, &ctx.repository, &pins, name, realization)
            .map_err(|error| super::file_failure(&profile.span, error))?;
        let previous = previous
            .as_ref()
            .and_then(|generation| generation.deployments.get(name));
        let retained = previous
            .map(|deployment| retention::read_profile(&ctx.home, name, deployment))
            .transpose()
            .map_err(|error| super::file_failure(&profile.span, error))?
            .flatten();
        let reuse = previous
            .zip(retained.as_ref())
            .filter(|(previous, retained)| {
                previous.store_path == profile.store_path
                    && previous.tree256.as_deref() == Some(profile.tree.as_str())
                    && previous.env == profile.env
                    && previous.intents == profile.intents
                    && previous
                        .build_closure
                        .iter()
                        .filter(|root| !retention::is_root(root))
                        .eq(profile.build_closure.iter())
                    && retention::same_desired_state(retained, &snapshot)
            });
        let root = if let Some((previous, _)) = reuse {
            retention::profile_root(previous)
                .map_err(|error| super::file_failure(&profile.span, error))?
                .expect("admitted retained snapshot has a root")
                .to_owned()
        } else {
            retention::publish_profile(session, name, snapshot)
                .map_err(|error| super::file_failure(&profile.span, error))?
        };
        profile.build_closure.push(root);
        profile.build_closure.sort();
        profile.build_closure.dedup();
    }
    Ok(())
}

pub(super) fn admit(profile: &ProfileOutput) -> Result<(), ExecError> {
    super::profile_launchers::destinations(profile)?;
    super::profile_checks::admit(profile)?;
    Ok(())
}
fn unavailable(span: &Span, message: impl Into<String>) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::WORKSPACE_EXEC_UNAVAILABLE, message)
            .with_label(Some(span.clone()), "profile selection declared here"),
    )
}
