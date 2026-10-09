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
    ) -> Result<Option<Self>, ExecError> {
        let mut origins = std::collections::BTreeSet::new();
        if let Some(workspace) = &ir.workspace_catalog {
            for output in &workspace.outputs {
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
        }
        let retained = super::retained::inspect(ir, repository, home, &origins, limits)?;
        Self::prepare_with(
            ir,
            repository.contents(),
            home,
            selected,
            limits,
            Some(&retained),
            true,
            Instant::now() + gripsack_process::Limits::default().timeout,
        )
    }
}

pub(crate) fn prepare_apply(
    ir: &Ir,
    ctx: &Ctx,
) -> Result<Option<(LifecycleSession, NativeProfiles)>, ExecError> {
    let Some(workspace) = &ir.workspace_catalog else {
        return Ok(None);
    };
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
            || profile.environment.is_some()
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
        return Ok(None);
    }
    let environment = gripsack_process::OperatorEnvironment::capture()?;
    let options = BuildOptions {
        environment: &environment,
        bridge: None,
        worker: Default::default(),
        deadline: Instant::now() + gripsack_process::Limits::default().timeout,
    };
    let mut held = realize::realize_held(ir, ctx, &options, &selected)?;
    let mut native = NativeProfiles::prepare_with(
        ir,
        ctx.repository.contents(),
        &ctx.home,
        &ctx.only,
        ctx.fetch.limits(),
        Some(&held.realization),
        false,
        options.deadline,
    )?
    .ok_or_else(|| {
        unavailable(
            &workspace.span,
            "current profile catalog lost its native adapter",
        )
    })?;
    for (name, paths) in &held.closures {
        let profile = native.profiles.get_mut(name).ok_or_else(|| {
            unavailable(
                &workspace.span,
                "realized profile is absent from the deployment selection",
            )
        })?;
        profile.build_closure = paths.iter().cloned().collect();
    }
    // The still-held lifecycle session now protects all immutable objects until
    // the caller's generation is durable. No permanent Output root is leaked.
    held.finish_build()?;
    Ok(Some((held.session, native)))
}

pub(super) fn admit(profile: &ProfileOutput) -> Result<(), ExecError> {
    if !profile.schedules.is_empty() {
        return Err(unavailable(
            &profile.span,
            "profile schedule registration is not available",
        ));
    }
    if let Some(file) = profile.files.iter().find(|file| !file.checks.is_empty()) {
        return Err(unavailable(
            &file.span,
            "profile file-check execution requires its stage-specific adapter",
        ));
    }
    Ok(())
}
fn unavailable(span: &Span, message: impl Into<String>) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::WORKSPACE_EXEC_UNAVAILABLE, message)
            .with_label(Some(span.clone()), "profile selection declared here"),
    )
}
