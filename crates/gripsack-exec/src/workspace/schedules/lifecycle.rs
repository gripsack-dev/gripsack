//! Schedules are ordinary owned profile files plus post-commit activation intents.
//! The generation selection is the only active-job registry and lifecycle fence.
use super::{capture, systemd};
use crate::workspace::{content::PreparedFile, file_failure, realize};
use crate::{ExecError, LifecycleSession};
use gripsack_ir::{Ir, workspace_model::ProfileOutput};
use gripsack_store::{
    self as store,
    activation::{ActivationAction, PendingIntent},
    jobs::{JobId, JobRevision},
};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    time::Instant,
};

pub struct ActiveJob {
    pub capture: PathBuf,
    pub runner: PathBuf,
    pub closure: Vec<PathBuf>,
}

/// Lookup cannot be authorized by a caller-supplied manifest or capture pathname.
/// The caller retains this session until its invocation lease is registered.
pub fn active_job(
    session: &LifecycleSession,
    job: &JobId,
    revision: &JobRevision,
) -> Result<ActiveJob, ExecError> {
    let home = gripsack_fs::open(session.home())?;
    if store::journal::pending_recovery(&home)?.is_some() {
        return Err(failure(
            "scheduled dispatch requires transaction reconciliation",
        ));
    }
    let generation = current(session)?;
    let selection = store::generations::current_selection_in(session.home(), &home)?
        .ok_or_else(|| failure("scheduled dispatch has no active selection"))?;
    for state in generation.deployments.values() {
        for intent in &state.intents {
            if let ActivationAction::ScheduledJob {
                job: active,
                revision: expected,
                capture,
                runner,
                unit_directory,
                enabled: true,
                ..
            } = &intent.action
                && active == job
                && expected == revision
            {
                let digest = gripsack_process::Sha256Digest::of(
                    &serde_json::to_vec(&intent.action).map_err(io::Error::other)?,
                );
                let mut registered = false;
                store::activation::inspect(&home, |inspection| {
                    if inspection
                        .activation
                        .is_some_and(|activation| activation.matches_selection(&selection))
                        && inspection.action_sha256 == digest
                    {
                        registered = matches!(
                            inspection.state,
                            store::activation::HookState::Current(
                                gripsack_policy::activation::IntentState::Succeeded { .. }
                            )
                        );
                    }
                    Ok(())
                })?;
                if !registered {
                    return Err(failure(
                        "job registration is pending, failed, or unknown; inspect activation outcomes",
                    ));
                }
                require_owned_entries(state, active)?;
                check_units(active, &state.store_path, unit_directory, true)?;
                let mut closure = state.build_closure.clone();
                closure.push(state.store_path.clone());
                closure.sort();
                closure.dedup();
                return Ok(ActiveJob {
                    capture: capture.clone(),
                    runner: runner.clone(),
                    closure,
                });
            }
        }
    }
    Err(failure(
        "job is disabled, retired, or the callback revision is stale",
    ))
}

fn current(session: &LifecycleSession) -> Result<store::Generation, ExecError> {
    let number = store::current_generation(session.home())?
        .ok_or_else(|| failure("no active profile generation"))?;
    Ok(store::generations::admit_manifest(session.home(), number)?)
}

pub(super) fn unit_directory() -> Result<PathBuf, ExecError> {
    let directory = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(directory) => PathBuf::from(directory),
        None => PathBuf::from(
            std::env::var_os("HOME")
                .ok_or_else(|| failure("HOME is required for user schedules"))?,
        )
        .join(".config"),
    };
    if !directory.is_absolute() {
        return Err(failure("user configuration directory must be absolute"));
    }
    Ok(directory.join("systemd/user"))
}

pub(in crate::workspace) fn destinations(
    home: &Path,
    profile: &ProfileOutput,
) -> Result<Vec<(String, String)>, ExecError> {
    if profile.schedules.is_empty() {
        return Ok(Vec::new());
    }
    let directory = unit_directory()?;
    let mut destinations = Vec::with_capacity(profile.schedules.len() * 2);
    for schedule in &profile.schedules {
        let job = capture::stable_job_id(home, &profile.name, schedule)?;
        let (service, timer) = systemd::unit_names(&job);
        for name in [service, timer] {
            let path = directory
                .join(name)
                .to_str()
                .ok_or_else(|| failure("unit path is not UTF-8"))?
                .to_owned();
            destinations.push((schedule.clone(), path));
        }
    }
    Ok(destinations)
}

/// Saved-plan fences observe native state without registration or realization.
pub(crate) fn review_bindings(
    ir: &Ir,
    home: &Path,
    selected: &[String],
    previous: Option<&store::Generation>,
) -> Result<serde_json::Value, ExecError> {
    let mut jobs = BTreeMap::new();
    let mut installed = BTreeMap::new();
    if let Some(previous) = previous {
        for state in previous.deployments.values() {
            for record in &state.intents {
                if let ActivationAction::ScheduledJob {
                    job,
                    unit_directory,
                    ..
                } = &record.action
                {
                    jobs.insert(job.clone(), unit_directory.clone());
                    installed.insert(job.clone(), &record.action);
                }
            }
        }
    }
    let mut declarations = Vec::new();
    for output in &ir.workspace.outputs {
        let gripsack_ir::workspace_model::WorkspaceOutput::Profile(profile) = output else {
            continue;
        };
        if !selected.is_empty() && !selected.contains(&profile.name) {
            continue;
        }
        if profile.schedules.is_empty() {
            continue;
        }
        let directory = unit_directory()?;
        for schedule in &profile.schedules {
            jobs.insert(
                capture::stable_job_id(home, &profile.name, schedule)?,
                directory.clone(),
            );
            declarations.push(serde_json::json!({"profile": profile.name, "schedule": schedule, "unit_directory": directory}));
        }
    }
    if jobs.is_empty() {
        return Ok(serde_json::json!({"declarations": [], "jobs": []}));
    }
    let capability = systemd::inspect()?;
    if !matches!(capability, systemd::Capability::Available { .. }) {
        return Err(failure(
            "saved schedule plan requires a qualified reachable user manager",
        ));
    }
    let mut observations = Vec::with_capacity(jobs.len());
    for (job, directory) in jobs {
        let status = systemd::status(&job, &directory)?;
        observations.push(review_observation(
            &job,
            &directory,
            &status,
            installed.get(&job).copied(),
        )?);
    }
    Ok(
        serde_json::json!({"declarations": declarations, "capability": capability, "jobs": observations, "registration": inspect(home)?}),
    )
}

/// A task's running/completed/failed state is observational, not authority to
/// change its registration. Keep it in status output, never a saved-plan fence.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
struct UnitAuthority<'a> {
    load_state: &'a str,
    unit_file_state: &'a str,
    fragment_path: &'a str,
    drop_in_paths: &'a str,
    timers_calendar: &'a [String],
}

fn unit_authority(observed: &systemd::ObservedUnit) -> Result<UnitAuthority<'_>, ExecError> {
    let systemd::ObservedUnit::Observed { properties } = observed else {
        return Err(failure(
            "saved schedule plan cannot authorize unknown or foreign manager state",
        ));
    };
    Ok(UnitAuthority {
        load_state: &properties.load_state,
        unit_file_state: &properties.unit_file_state,
        fragment_path: &properties.fragment_path,
        drop_in_paths: &properties.drop_in_paths,
        timers_calendar: &properties.timers_calendar,
    })
}

fn review_observation(
    job: &JobId,
    directory: &Path,
    status: &systemd::UnitStatus,
    installed: Option<&ActivationAction>,
) -> Result<serde_json::Value, ExecError> {
    let service = unit_authority(&status.service)?;
    let timer = unit_authority(&status.timer)?;
    let (service_name, timer_name) = systemd::unit_names(job);
    let bytes = [service_name, timer_name]
        .into_iter()
        .map(|name| {
            let path = directory.join(&name);
            let digest = match std::fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(ExecError::from(error)),
                Ok(metadata)
                    if metadata.is_file() && metadata.len() <= systemd::MAX_UNIT_BYTES as u64 =>
                {
                    use std::io::Read;
                    let parent = gripsack_fs::open(directory)?;
                    let file = gripsack_fs::open_file_nofollow(&parent, Path::new(&name))?;
                    let mut bytes = Vec::new();
                    file.take(systemd::MAX_UNIT_BYTES as u64 + 1)
                        .read_to_end(&mut bytes)?;
                    if bytes.len() > systemd::MAX_UNIT_BYTES {
                        return Err(failure("saved schedule unit exceeds its byte budget"));
                    }
                    Some(gripsack_process::Sha256Digest::of(&bytes))
                }
                Ok(_) => return Err(failure("saved schedule unit is not a bounded regular file")),
            };
            Ok((name, digest))
        })
        .collect::<Result<BTreeMap<_, _>, ExecError>>()?;
    Ok(serde_json::json!({
        "job": job, "unit_directory": directory, "installed": installed,
        "unit_bytes": bytes, "manager": {"service": service, "timer": timer},
    }))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::workspace) fn prepare(
    ir: &Ir,
    repository: &crate::Repository,
    home: &Path,
    realization: &realize::Realization<'_>,
    profile: &ProfileOutput,
    stage: &Path,
    deadline: Instant,
    files: &mut Vec<PreparedFile>,
) -> Result<Vec<capture::PreparedJob>, ExecError> {
    if profile.schedules.is_empty() {
        return Ok(Vec::new());
    }
    match systemd::inspect()? {
        systemd::Capability::Available { .. } => {}
        capability => {
            return Err(file_failure(
                &profile.span,
                format!("qualified user systemd manager required: {capability:?}"),
            ));
        }
    }
    let jobs = capture::prepare(ir, repository, home, realization, profile, stage, deadline)?;
    let directory = unit_directory()?;
    for job in &jobs {
        systemd::inspect_unit(&job.job, &directory)?;
        let units = systemd::render(&job.job, &job.revision, &job.runner, home, &job.trigger)?;
        systemd::validate(
            &units,
            &job.job,
            &job.revision,
            &job.runner,
            home,
            &job.trigger,
        )?;
        let (service, timer) = systemd::unit_names(&job.job);
        for (name, bytes) in [(service, units.service), (timer, units.timer)] {
            std::fs::write(stage.join(&name), bytes.as_bytes())?;
            files.push(PreparedFile {
                entry: store::DeploymentEntry {
                    from: name.clone(),
                    to: directory
                        .join(name)
                        .to_str()
                        .ok_or_else(|| failure("unit path is not UTF-8"))?
                        .to_owned(),
                    mode: store::Ownership::TrackedCopy,
                    vars: Default::default(),
                    marker: None,
                    span: Some(profile.span.clone()),
                },
                block_id: None,
            });
        }
    }
    Ok(jobs)
}

pub(in crate::workspace) fn records(
    jobs: &[capture::PreparedJob],
    payload: &Path,
) -> Result<Vec<store::IntentRecord>, ExecError> {
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    let directory = unit_directory()?;
    Ok(jobs
        .iter()
        .map(|job| store::IntentRecord {
            action: ActivationAction::ScheduledJob {
                job: job.job.clone(),
                revision: job.revision.clone(),
                capture: payload.join(&job.file),
                runner: job.runner.clone(),
                unit_directory: directory.clone(),
                enabled: true,
                closure: job.closure.clone(),
            },
            trigger: store::ActivationTrigger::PostActivate,
        })
        .collect())
}

/// Schedules removed from a still-present profile must retire just as those in
/// a wholly removed profile do. A revision update keeps the stable timer name.
pub(crate) fn removal_intents(
    previous: Option<&store::Generation>,
    next: &BTreeMap<String, store::DeploymentState>,
) -> Vec<PendingIntent> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let active: std::collections::BTreeSet<_> = next
        .values()
        .flat_map(|state| &state.intents)
        .filter_map(|record| match &record.action {
            ActivationAction::ScheduledJob {
                job, enabled: true, ..
            } => Some(job),
            _ => None,
        })
        .collect();
    previous
        .deployments
        .iter()
        .flat_map(|(name, state)| {
            state.intents.iter().filter_map(|record| {
                let ActivationAction::ScheduledJob {
                    job,
                    revision,
                    capture,
                    runner,
                    unit_directory,
                    closure,
                    enabled: true,
                } = &record.action
                else {
                    return None;
                };
                if active.contains(job) {
                    return None;
                }
                Some(PendingIntent {
                    output: name.clone(),
                    trigger: store::ActivationTrigger::OnRemove,
                    action: ActivationAction::ScheduledJob {
                        job: job.clone(),
                        revision: revision.clone(),
                        capture: capture.clone(),
                        runner: runner.clone(),
                        unit_directory: unit_directory.clone(),
                        closure: closure.clone(),
                        enabled: false,
                    },
                })
            })
        })
        .collect()
}

pub(crate) fn registration_intents(
    next: &BTreeMap<String, store::DeploymentState>,
    intents: &mut Vec<PendingIntent>,
) {
    for (name, state) in next {
        for record in &state.intents {
            if matches!(
                record.action,
                ActivationAction::ScheduledJob { enabled: true, .. }
            ) && !intents.iter().any(|intent| intent.action == record.action)
            {
                intents.push(PendingIntent {
                    output: name.clone(),
                    action: record.action.clone(),
                    trigger: record.trigger,
                });
            }
        }
    }
}

/// Called only after the existing activation ledger grants a launch permit.
pub(crate) fn effect(
    action: &ActivationAction,
    home: &Path,
    _permit: &store::activation::LaunchPermit,
) -> Result<Vec<gripsack_process::ProcessReceipt>, ExecError> {
    let ActivationAction::ScheduledJob {
        job,
        capture,
        unit_directory: directory,
        enabled,
        ..
    } = action
    else {
        return Err(failure("not a schedule activation action"));
    };
    let payload = capture
        .parent()
        .ok_or_else(|| failure("schedule payload is absent"))?;
    if *enabled {
        let number = store::current_generation(home)?
            .ok_or_else(|| failure("schedule activation has no current generation"))?;
        let generation = store::generations::admit_manifest(home, number)?;
        let state = generation
            .deployments
            .values()
            .find(|state| state.intents.iter().any(|intent| &intent.action == action))
            .ok_or_else(|| {
                failure("schedule activation is not selected by the current generation")
            })?;
        require_owned_entries(state, job)?;
    }
    check_units(job, payload, directory, *enabled)?;
    if *enabled {
        systemd::register(job, directory)
    } else {
        systemd::remove(job, directory)
    }
}

fn require_owned_entries(state: &store::DeploymentState, job: &JobId) -> Result<(), ExecError> {
    let (service, timer) = systemd::unit_names(job);
    for name in [service, timer] {
        if !state
            .entries
            .iter()
            .any(|entry| entry.from == Path::new(&name) && !entry.preserved_drift)
        {
            return Err(failure(
                "schedule unit ownership is absent or preserved as user drift",
            ));
        }
    }
    Ok(())
}

fn check_units(
    job: &JobId,
    payload: &Path,
    directory: &Path,
    required: bool,
) -> Result<(), ExecError> {
    // Never mutate manager state or dispatch for foreign/drifted unit files.
    let (service, timer) = systemd::unit_names(job);
    for name in [service, timer] {
        let destination = directory.join(&name);
        match std::fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.is_file() => {
                if store::canonical_file_hash(&destination)?
                    != store::canonical_file_hash(&payload.join(name))?
                {
                    return Err(failure(
                        "schedule unit drifted; preserving foreign manager state",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && !required => {}
            Err(error) => return Err(error.into()),
            _ => return Err(failure("schedule unit is not an owned regular file")),
        }
    }
    Ok(())
}

/// Disable is a normal immutable generation transition through the existing
/// ownership planner, journal, rollback adapter and post-commit outcome ledger.
pub fn disable(
    session: &LifecycleSession,
    job: &JobId,
) -> Result<Vec<store::journal::RecoveryNote>, ExecError> {
    let home = gripsack_fs::open(session.home())?;
    store::journal::reconcile(&home, session.home())?;
    crate::activate::resume_activation(session)?;
    let previous = current(session)?;
    let mut target = previous.clone();
    let (service, timer) = systemd::unit_names(job);
    let mut found = false;
    for state in target.deployments.values_mut() {
        let owns_job = state.intents.iter().any(|record| matches!(&record.action, ActivationAction::ScheduledJob { job: active, .. } if active == job));
        if !owns_job {
            continue;
        }
        found = true;
        state.intents.retain(|record| !matches!(&record.action, ActivationAction::ScheduledJob { job: active, .. } if active == job));
        state
            .entries
            .retain(|entry| entry.from != Path::new(&service) && entry.from != Path::new(&timer));
    }
    if !found {
        return Err(failure("job is not active in the current generation"));
    }
    target.number = store::generations::allocate(session.home(), &home)?;
    target.directory_transitions.clear();
    target.migration = None;
    store::generations::publish_generation(&home, &target, None, session.home())?;
    crate::rollback::rollback_generation(session, Some(&previous), &target)
}

#[derive(Debug, serde::Serialize)]
pub struct JobInspection {
    pub profile: String,
    pub job: JobId,
    pub revision: JobRevision,
    pub registration: RegistrationState,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationState {
    Pending,
    Active,
    Failed,
    Unknown,
}

/// Offline status reads admitted manifests and the existing outcome ledger only.
pub fn inspect(home: &Path) -> Result<Vec<JobInspection>, ExecError> {
    let Some(number) = store::current_generation(home)? else {
        return Ok(Vec::new());
    };
    let directory = gripsack_fs::open(home)?;
    let generation = store::generations::admit_manifest(home, number)?;
    let selection = store::generations::current_selection_in(home, &directory)?
        .ok_or_else(|| failure("current selection disappeared during inspection"))?;
    let mut jobs = Vec::new();
    for (profile, state) in &generation.deployments {
        for record in &state.intents {
            let ActivationAction::ScheduledJob {
                job,
                revision,
                enabled: true,
                ..
            } = &record.action
            else {
                continue;
            };
            let digest = gripsack_process::Sha256Digest::of(
                &serde_json::to_vec(&record.action).map_err(io::Error::other)?,
            );
            jobs.push((
                digest,
                JobInspection {
                    profile: profile.clone(),
                    job: job.clone(),
                    revision: revision.clone(),
                    registration: RegistrationState::Unknown,
                },
            ));
        }
    }
    store::activation::inspect(&directory, |inspection| {
        if !inspection
            .activation
            .is_some_and(|activation| activation.matches_selection(&selection))
        {
            return Ok(());
        }
        for (digest, job) in &mut jobs {
            if *digest != inspection.action_sha256 {
                continue;
            }
            use gripsack_policy::activation::IntentState;
            job.registration = match inspection.state {
                store::activation::HookState::Current(IntentState::Pending) => {
                    RegistrationState::Pending
                }
                store::activation::HookState::Current(IntentState::Succeeded { .. }) => {
                    RegistrationState::Active
                }
                store::activation::HookState::Current(IntentState::Failed { .. }) => {
                    RegistrationState::Failed
                }
                _ => RegistrationState::Unknown,
            };
        }
        Ok(())
    })?;
    Ok(jobs.into_iter().map(|(_, job)| job).collect())
}

fn failure(message: &str) -> ExecError {
    io::Error::new(io::ErrorKind::InvalidData, message).into()
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod review_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn job(byte: char) -> JobId {
        JobId::parse(&byte.to_string().repeat(64)).unwrap()
    }

    fn state(job: JobId, revision: char) -> store::DeploymentState {
        store::DeploymentState {
            store_path: "/retained/profile".into(),
            build_only: false,
            entries: Vec::new(),
            intents: vec![store::IntentRecord {
                action: ActivationAction::ScheduledJob {
                    job,
                    revision: JobRevision::parse(&revision.to_string().repeat(64)).unwrap(),
                    capture: "/retained/profile/job.json".into(),
                    runner: "/retained/runner/grip".into(),
                    unit_directory: "/owned/systemd/user".into(),
                    enabled: true,
                    closure: vec!["/retained/runner".into()],
                },
                trigger: store::ActivationTrigger::PostActivate,
            }],
            verified: None,
            env: Vec::new(),
            tree256: None,
            build_closure: vec!["/retained/runner".into()],
        }
    }

    fn generation(state: store::DeploymentState) -> store::Generation {
        store::Generation {
            number: store::GenerationId::new(1),
            deployments: BTreeMap::from([("personal".into(), state)]),
            directory_transitions: Vec::new(),
            migration: None,
        }
    }

    #[test]
    fn removing_schedule_inside_surviving_profile_unregisters_exact_old_job() {
        let previous = generation(state(job('a'), 'b'));
        let mut next = previous.deployments.clone();
        next.get_mut("personal").unwrap().intents.clear();
        let intents = removal_intents(Some(&previous), &next);
        assert_eq!(intents.len(), 1);
        assert!(matches!(&intents[0].action,
            ActivationAction::ScheduledJob { job: removed, enabled: false, .. } if removed == &job('a')));
        assert_eq!(intents[0].trigger, store::ActivationTrigger::OnRemove);
    }

    #[test]
    fn replacing_revision_does_not_disable_stable_timer() {
        let previous = generation(state(job('a'), 'b'));
        let next = generation(state(job('a'), 'c'));
        assert!(removal_intents(Some(&previous), &next.deployments).is_empty());
        let mut intents = Vec::new();
        registration_intents(&next.deployments, &mut intents);
        registration_intents(&next.deployments, &mut intents);
        assert_eq!(intents.len(), 1);
        assert_eq!(
            intents[0].action,
            next.deployments["personal"].intents[0].action
        );
    }

    #[test]
    fn profile_removal_retains_previous_capture_for_unregister() {
        let previous = generation(state(job('a'), 'b'));
        let intents = removal_intents(Some(&previous), &BTreeMap::new());
        assert!(matches!(&intents[0].action,
            ActivationAction::ScheduledJob { capture, enabled: false, .. }
                if capture == Path::new("/retained/profile/job.json")));
    }

    #[test]
    fn foreign_or_drifted_units_never_authorize_registration_or_dispatch() {
        let payload = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let identity = job('a');
        let (service, timer) = systemd::unit_names(&identity);
        for name in [&service, &timer] {
            std::fs::write(payload.path().join(name), "owned").unwrap();
            std::fs::write(destination.path().join(name), "owned").unwrap();
        }
        assert!(check_units(&identity, payload.path(), destination.path(), true).is_ok());
        std::fs::write(destination.path().join(&timer), "foreign").unwrap();
        assert!(check_units(&identity, payload.path(), destination.path(), true).is_err());
        assert!(check_units(&identity, payload.path(), destination.path(), false).is_err());
        assert_eq!(
            std::fs::read(destination.path().join(timer)).unwrap(),
            b"foreign"
        );
    }

    #[test]
    fn retired_units_may_be_absent_but_dispatch_requires_both() {
        let payload = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        assert!(check_units(&job('a'), payload.path(), destination.path(), false).is_ok());
        assert!(check_units(&job('a'), payload.path(), destination.path(), true).is_err());
    }
}

#[cfg(all(test, unix))]
mod admission_integration_tests {
    use super::*;

    #[test]
    fn filesystem_manifest_does_not_grant_callback_authority() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path();
        let session = LifecycleSession::acquire(home).unwrap();
        let directory = gripsack_fs::open(home).unwrap();
        let job = JobId::parse(&"a".repeat(64)).unwrap();
        let revision = JobRevision::parse(&"b".repeat(64)).unwrap();
        let payload = home.join("store/profile");
        let runner_root = home.join("store/runner");
        let units = home.join("units");
        let (service, timer) = systemd::unit_names(&job);
        let entries = [service, timer]
            .into_iter()
            .map(|name| {
                serde_json::from_value(serde_json::json!({
                    "from": name, "to": units.join(&name), "mode": "tracked_copy",
                    "hash": "c".repeat(64)
                }))
                .unwrap()
            })
            .collect();
        let state = store::DeploymentState {
            store_path: payload.clone(),
            build_only: false,
            entries,
            intents: vec![store::IntentRecord {
                action: ActivationAction::ScheduledJob {
                    job: job.clone(),
                    revision: revision.clone(),
                    capture: payload.join("job.json"),
                    runner: runner_root.join("grip"),
                    unit_directory: units,
                    enabled: true,
                    closure: vec![runner_root.clone()],
                },
                trigger: store::ActivationTrigger::PostActivate,
            }],
            verified: None,
            env: Vec::new(),
            tree256: None,
            build_closure: vec![runner_root],
        };
        let generation = store::Generation {
            number: store::GenerationId::new(1),
            deployments: BTreeMap::from([("personal".into(), state)]),
            directory_transitions: Vec::new(),
            migration: None,
        };
        store::write_manifest(&directory, &generation).unwrap();
        std::os::unix::fs::symlink("generations/1", home.join("current")).unwrap();
        let error = active_job(&session, &job, &revision)
            .err()
            .expect("missing registration cannot dispatch");
        assert!(
            error
                .to_string()
                .contains("registration is pending, failed, or unknown")
        );
        let stale = JobRevision::parse(&"d".repeat(64)).unwrap();
        let error = active_job(&session, &job, &stale)
            .err()
            .expect("stale revision cannot dispatch");
        assert!(error.to_string().contains("callback revision is stale"));
        let retired = JobId::parse(&"e".repeat(64)).unwrap();
        assert!(active_job(&session, &retired, &revision).is_err());
    }
}
