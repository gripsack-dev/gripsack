//! The current generation and process-root transfer share one admission lock.
use super::{JobId, JobRevision, capture, lifecycle, snapshot};
use crate::{
    ExecError, LifecycleSession,
    workspace::{BuildOptions, TaskReport, consumer},
};
use std::{collections::BTreeSet, path::Path, time::Instant};

pub fn dispatch(home: &Path, job: &JobId, revision: &JobRevision) -> Result<TaskReport, ExecError> {
    let session = LifecycleSession::acquire(home)?;
    let active = lifecycle::active_job(&session, job, revision)?;
    let snapshot = snapshot::read(home, &active.capture, job, revision)?;
    if active.runner != snapshot.runner {
        return Err(std::io::Error::other("scheduled runner differs from active revision").into());
    }
    let mut closure: BTreeSet<_> = active.closure.into_iter().collect();
    closure.extend(snapshot.roots.keys().cloned());
    let profile_root = active
        .capture
        .parent()
        .ok_or_else(|| std::io::Error::other("capture has no root"))?;
    closure.insert(profile_root.to_owned());
    crate::workspace::sharing::readmit_roots(home, closure.iter().map(|path| path.as_path()))?;
    let realization = snapshot.realization(home)?;
    let ctx = capture::context(home, &snapshot.checkout, &snapshot.task)?;
    let environment = gripsack_process::OperatorEnvironment::capture()?;
    let options = BuildOptions {
        environment: &environment,
        deadline: Instant::now() + gripsack_process::Limits::default().timeout,
    };
    // run_captured writes its own invocation receipt and registers its process
    // lease before dropping this exact session. Update/disable can then retire
    // the generation without changing this already-admitted invocation's tools.
    consumer::run_captured(
        &snapshot.ir,
        &ctx,
        &options,
        session,
        &realization,
        closure,
        &snapshot.programs,
        consumer::ScheduledInvocation {
            job: job.clone(),
            revision: revision.clone(),
        },
    )
}
