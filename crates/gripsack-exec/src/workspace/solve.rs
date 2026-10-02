//! Shared backend session ownership for recipe and OCI consumers. Native roots
//! protect the immutable inputs and future publication paths while the lifecycle
//! lock is released. Only the caller's completed publication retires the root.
use super::{
    artifact::Artifact,
    lowering::SourceMap,
    realize::BuildOptions,
    roots::{self, BuildAttempt, ProcessLease, RetentionSet},
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_buildkit::{
    identity::{AttemptId, AttemptIdentity, FenceEpoch, SessionId},
    plan::ValidatedBuildPlan,
    transport::{Bridge, CompletedExport, ExportPaths},
    worker::{CleanupConfirmation, OwnedWorker, WorkerProfile},
};
use gripsack_process::{SelectedProgram, Sha256Digest};
use std::{collections::BTreeMap, path::Path};

pub(super) struct CompletedSolve {
    retained: ProcessLease,
    export: CompletedExport,
}
impl CompletedSolve {
    pub fn output(&self) -> &Path {
        self.export.destination()
    }
    pub fn finish(self, session: &LifecycleSession) -> Result<(), ExecError> {
        self.retained.release_build(session, &self.export)
    }
}

pub(super) fn execute(
    ctx: &Ctx,
    options: &BuildOptions<'_>,
    plan: &ValidatedBuildPlan,
    sources: &BTreeMap<String, &Artifact>,
    origins: &mut SourceMap<'_>,
    retention: &RetentionSet,
    session: LifecycleSession,
) -> Result<(LifecycleSession, CompletedSolve), ExecError> {
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(std::io::Error::other)?;
    let identity = AttemptIdentity {
        session: SessionId::new(gripsack_store::hash::hex_sha256(&nonce)).map_err(operational)?,
        attempt: AttemptId::new(1).map_err(operational)?,
        epoch: FenceEpoch::new(1).map_err(operational)?,
    };
    let mut retained = ProcessLease::register(
        &session,
        retention,
        Some(BuildAttempt {
            identity: identity.clone(),
            worker: None,
        }),
    )?;
    let stage_name = retained.id().as_str();
    let staging_parent = ctx.home.join(roots::BUILD_STAGING_DIRECTORY);
    gripsack_fs::create_dir_all(ctx.home_dir()?, Path::new(roots::BUILD_STAGING_DIRECTORY))?;
    let parent =
        gripsack_fs::open_dir_nofollow(ctx.home_dir()?, Path::new(roots::BUILD_STAGING_DIRECTORY))?;
    parent.create_dir(stage_name)?;
    let staging = staging_parent.join(stage_name);
    let staging_cap = gripsack_fs::open_dir_nofollow(&parent, Path::new(stage_name))?;
    use gripsack_fs::cap_std::fs::PermissionsExt;
    staging_cap.set_permissions(".", gripsack_fs::cap_std::fs::Permissions::from_mode(0o700))?;
    for directory in ["inputs", "output"] {
        staging_cap.create_dir(directory)?;
        staging_cap.set_permissions(
            directory,
            gripsack_fs::cap_std::fs::Permissions::from_mode(0o700),
        )?;
    }
    gripsack_fs::fsync_pinned_dir(&parent, &staging_parent)?;
    let inputs = staging.join("inputs");
    let output = staging.join("output");
    for (name, artifact) in sources {
        let destination = inputs.join(name);
        gripsack_fetch::fetch::clone_immutable_tree(
            &artifact.payload,
            &destination,
            ctx.fetch.limits(),
        )?;
        if gripsack_store::canonical_tree_hash(&destination)? != artifact.tree {
            return Err(failure(
                "retained source changed while preparing the declared session snapshot",
            ));
        }
    }
    drop(session);
    // Default helper bytes are pinned to this core release. An explicit override
    // is operator authority; no repository value selects a native executable.
    let selected = match options.bridge {
        Some(program) => {
            SelectedProgram::select(options.environment, program, None, options.deadline)?
        }
        None => {
            let provisioned =
                gripsack_fetch::bridge::ensure(&ctx.home, &ctx.fetch, options.deadline)
                    .map_err(operational)?;
            let expected = Sha256Digest::parse(&provisioned.pin)?;
            SelectedProgram::select(
                options.environment,
                &provisioned.path,
                Some(expected),
                options.deadline,
            )?
        }
    };
    let bridge = Bridge::new(options.environment, &selected, &staging, options.deadline);
    let retention_handle = retained.duplicate_handle()?;
    let checked = bridge
        .lower(plan, &identity, Some(&retention_handle))
        .map_err(operational)?;
    let worker = OwnedWorker::open(
        &ctx.home,
        WorkerProfile::parse("production").map_err(operational)?,
        options.environment,
        options.worker,
        options.deadline,
    )
    .map_err(operational)?;
    let lease = worker.acquire().map_err(operational)?;
    {
        let session = LifecycleSession::acquire(&ctx.home)?;
        retained.bind_worker(&session, retention, &lease)?;
    }
    let result = bridge.execute(
        checked,
        &identity,
        ExportPaths {
            worker: &lease,
            retention: Some(&retention_handle),
            inputs: &inputs,
            destination: &output,
        },
        |nodes, vertex, chunk, truncated| {
            origins.log(nodes, vertex, chunk, truncated, ctx.on_progress.as_ref());
        },
    );
    let export = match result {
        Ok(completed) => {
            worker
                .release(lease, CleanupConfirmation::Confirmed)
                .map_err(operational)?;
            completed
        }
        Err(error) => {
            let cleanup = worker.release(lease, CleanupConfirmation::Uncertain);
            return Err(ExecError::Gate(origins.failure(&error).with_help(format!("build staging and roots retained at {staging:?}; worker quarantine: {cleanup:?}; use `grip builder stop` for fenced recovery"))));
        }
    };
    crate::util::crash_hook("workspace-export-completed");
    let session = LifecycleSession::acquire(&ctx.home)?;
    Ok((session, CompletedSolve { retained, export }))
}
fn operational(error: impl std::fmt::Display) -> ExecError {
    failure(error.to_string())
}
fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        module: "workspace".into(),
        step: "solve".into(),
        detail: detail.into(),
    }
}
