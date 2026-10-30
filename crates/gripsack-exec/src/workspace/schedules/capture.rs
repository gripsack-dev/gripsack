//! Prepare all graph commands and exact native inputs before owned units change.
use super::{
    JobId, JobRevision, runner,
    snapshot::{CAPTURE_VERSION, Snapshot},
};
use crate::{
    Ctx, ExecError, Repository,
    workspace::{
        artifact, consumer,
        realize::{BuildOptions, Realization},
        selection::Selection,
    },
};
use gripsack_ir::{
    Ir,
    workspace_model::{
        ProfileOutput, WorkspaceArg, WorkspaceCalendar, WorkspaceCommand, WorkspaceOutput,
        WorkspaceStep,
    },
};
use gripsack_process::OperatorEnvironment;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Instant,
};

pub(in crate::workspace) struct PreparedJob {
    pub file: PathBuf,
    pub job: JobId,
    pub revision: JobRevision,
    pub runner: PathBuf,
    pub closure: Vec<PathBuf>,
    pub trigger: WorkspaceCalendar,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    ir: &Ir,
    repository: &Repository,
    home: &Path,
    realization: &Realization<'_>,
    profile: &ProfileOutput,
    stage: &Path,
    deadline: Instant,
) -> Result<Vec<PreparedJob>, ExecError> {
    if profile.schedules.is_empty() {
        return Ok(Vec::new());
    }
    let environment = OperatorEnvironment::capture()?;
    let options = BuildOptions {
        environment: &environment,
        deadline,
    };
    let mut ctx = context(home, repository.identity(), "scheduled")?;
    let (runner, runner_root) = runner::retain(&ctx)?;
    let mut prepared = Vec::with_capacity(profile.schedules.len());
    for name in &profile.schedules {
        let schedule = ir
            .workspace
            .outputs
            .iter()
            .find_map(|output| match output {
                WorkspaceOutput::Schedule(schedule) if &schedule.name == name => Some(schedule),
                _ => None,
            })
            .ok_or_else(|| invalid("selected profile schedule is absent"))?;
        ctx.only = vec![schedule.task.clone()];
        let selection = Selection::admit(&ir.workspace, &ctx.only)?;
        let mut captured_ir = ir.clone();
        captured_ir
            .workspace
            .outputs
            .retain(|output| selection.required.contains(output.name()));
        freeze_external_programs(&mut captured_ir, realization, &environment)?;
        let mut packages = BTreeMap::new();
        let mut receipts = BTreeMap::new();
        let mut roots = BTreeSet::from([runner_root.clone()]);
        for (&name, package) in &realization.packages {
            if !selection.required.contains(name) {
                continue;
            }
            artifact::hook::capture(home, package, &mut receipts)?;
            package.retain_into(&mut roots);
            packages.insert(name.to_owned(), package.identity);
        }
        let inputs: BTreeMap<_, _> = realization
            .inputs
            .iter()
            .map(|(&name, path)| (name.to_owned(), path.clone()))
            .collect();
        for path in inputs.values() {
            let relative = path
                .strip_prefix(home.join("store"))
                .map_err(|_| invalid("captured input is outside immutable store"))?;
            let component = relative
                .components()
                .next()
                .ok_or_else(|| invalid("captured input has no store root"))?;
            let root = home.join("store").join(component);
            gripsack_store::paths::validate_store_root(home, &root)?;
            roots.insert(root);
        }
        let scratch = tempfile::tempdir()?;
        let programs =
            consumer::capture_programs(&captured_ir, &ctx, &options, realization, scratch.path())?;
        let job = stable_job_id(home, &profile.name, name)?;
        let snapshot = Snapshot {
            version: CAPTURE_VERSION,
            job: job.clone(),
            task: schedule.task.clone(),
            trigger: schedule.trigger.clone(),
            runner: runner.clone(),
            checkout: repository.identity().to_owned(),
            unit_directory: super::lifecycle::unit_directory()?,
            ir: captured_ir,
            programs,
            packages,
            receipts,
            inputs,
            roots: roots
                .iter()
                .map(|root| {
                    gripsack_store::canonical_tree_hash(root).map(|hash| (root.clone(), hash))
                })
                .collect::<std::io::Result<_>>()?,
        };
        let bytes = snapshot.encode()?;
        let revision = JobRevision::parse(&gripsack_store::hash::hex_sha256(&bytes))?;
        let file = PathBuf::from(format!("job-{}.json", job.as_str()));
        let directory = gripsack_fs::open(stage)?;
        gripsack_fs::atomic_write_with_mode(&directory, &file, &bytes, 0o600)?;
        prepared.push(PreparedJob {
            file,
            job,
            revision,
            runner: runner.clone(),
            closure: roots.into_iter().collect(),
            trigger: schedule.trigger.clone(),
        });
    }
    Ok(prepared)
}

pub(in crate::workspace) fn stable_job_id(
    home: &Path,
    profile: &str,
    schedule: &str,
) -> Result<JobId, ExecError> {
    Ok(JobId::parse(&gripsack_store::hash::hex_sha256(
        &serde_json::to_vec(&(home, profile, schedule))?,
    ))?)
}

pub(in crate::workspace) fn context(
    home: &Path,
    checkout: &Path,
    task: &str,
) -> Result<Ctx, ExecError> {
    Ok(Ctx {
        home: home.to_owned(),
        repository: Repository::Direct(checkout.to_owned()),
        only: vec![task.to_owned()],
        host: gripsack_ir::HostName::parse("scheduled").map_err(ExecError::Gate)?,
        on_progress: None,
        take_over: false,
        take_over_entries: None,
        fetch: Arc::new(gripsack_fetch::FetchContext::default()),
        home_dir: OnceLock::new(),
    })
}

/// External programs become explicit canonical paths. Environment commands stay
/// typed package selections and are re-admitted with their retained loaders.
fn freeze_external_programs(
    ir: &mut Ir,
    realization: &Realization<'_>,
    environment: &OperatorEnvironment,
) -> Result<(), ExecError> {
    let environments: BTreeMap<_, BTreeSet<_>> = ir
        .workspace
        .outputs
        .iter()
        .filter_map(|output| {
            let WorkspaceOutput::Environment(value) = output else {
                return None;
            };
            let commands = value
                .packages
                .iter()
                .filter_map(|name| realization.packages.get(name.as_str()))
                .flat_map(|package| package.commands.keys().cloned())
                .collect();
            Some((value.name.clone(), commands))
        })
        .collect();
    let empty = BTreeSet::new();
    // Shared postconditions may run with more than one environment. Keep a name
    // supplied by any of those environments; the prepared identity fence still
    // checks each individual invocation's exact executable/interpreter bytes.
    let all_commands = environments
        .values()
        .flat_map(|names| names.iter().cloned())
        .collect();
    for output in &mut ir.workspace.outputs {
        match output {
            WorkspaceOutput::Task(task) => {
                let commands = task
                    .environment
                    .as_ref()
                    .and_then(|name| environments.get(name))
                    .unwrap_or(&empty);
                for step in &mut task.steps {
                    if let WorkspaceStep::Command(command) = step {
                        freeze_program(command, commands, environment)?;
                    }
                }
            }
            WorkspaceOutput::Check(check) => {
                freeze_program(&mut check.run, &all_commands, environment)?
            }
            _ => {}
        }
    }
    Ok(())
}
fn freeze_program(
    command: &mut WorkspaceCommand,
    commands: &BTreeSet<String>,
    environment: &OperatorEnvironment,
) -> Result<(), ExecError> {
    let program = match command {
        WorkspaceCommand::Exec { argv, .. } => argv
            .first_mut()
            .ok_or_else(|| invalid("scheduled command has no executable"))?,
        WorkspaceCommand::RunBash { interpreter, .. } => interpreter,
    };
    if let WorkspaceArg::Literal { value } = program
        && !commands.contains(value)
    {
        *value = environment
            .resolve(Path::new(value))?
            .canonical()
            .to_str()
            .ok_or_else(|| invalid("scheduled executable path is not UTF-8"))?
            .to_owned();
    }
    Ok(())
}
fn invalid(message: &str) -> ExecError {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}
