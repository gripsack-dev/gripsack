//! Private bounded per-occasion receipts. Unknown running records are never
//! promoted to success by observing a vanished PID or an elapsed deadline.
use super::{failure, task_graph::TaskRevision};
use crate::ExecError;
use gripsack_process::{ProcessDisposition, ProcessReceipt, Sha256Digest};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

const REPORT_DIRECTORY: &str = "task-invocations";
const MAX_REPORT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REPORT_ENTRIES: usize = 10000;
pub(super) const MAX_LOG_BYTES: usize = 1024 * 1024;

macro_rules! occasion_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Sha256Digest);
        impl $name {
            pub fn parse(value: &str) -> std::io::Result<Self> {
                Sha256Digest::parse(value).map(Self)
            }
            pub fn as_digest(&self) -> Sha256Digest {
                self.0
            }
            fn fresh() -> Result<Self, ExecError> {
                let mut bytes = [0u8; 32];
                getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
                Ok(Self(Sha256Digest::of(&bytes)))
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(output, "{}", self.0)
            }
        }
    };
}
occasion_id!(InvocationId);
occasion_id!(AttemptId);

/// Concrete realization/worktree binding, distinct from reusable declaration
/// semantics and from each occasion's random invocation/attempt identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskExecutionBinding(pub(super) Sha256Digest);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
    SkippedBusy,
    Indeterminate,
}
impl TaskStatus {
    pub(super) fn state(self) -> gripsack_policy::task::TaskState {
        use gripsack_policy::task::TaskState;
        match self {
            Self::Succeeded => TaskState::Succeeded,
            Self::Failed => TaskState::Failed,
            Self::Blocked => TaskState::Blocked,
            Self::Cancelled => TaskState::Cancelled,
            Self::SkippedBusy => TaskState::SkippedBusy,
            Self::Indeterminate => TaskState::Indeterminate,
        }
    }
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Succeeded => 0,
            Self::Failed => 1,
            Self::Blocked => 3,
            Self::Cancelled => 130,
            Self::SkippedBusy => 75,
            Self::Indeterminate => 70,
        }
    }
    pub(super) fn from_receipt(receipt: &ProcessReceipt) -> Self {
        match receipt.disposition {
            ProcessDisposition::CleanupFailure => Self::Indeterminate,
            ProcessDisposition::Cancelled => Self::Cancelled,
            ProcessDisposition::Exited
                if receipt.exit_code == Some(0) && receipt.signal.is_none() =>
            {
                Self::Succeeded
            }
            ProcessDisposition::Exited if receipt.signal.is_some() => Self::Cancelled,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskNodeReport {
    pub task: String,
    pub status: TaskStatus,
    pub declared_steps: usize,
    pub declared_checks: usize,
    pub completed_steps: usize,
    pub completed_checks: usize,
    pub processes: Vec<ProcessReceipt>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskReport {
    pub version: u32,
    pub invocation: InvocationId,
    pub attempt: AttemptId,
    pub revision: TaskRevision,
    pub execution_binding: Option<TaskExecutionBinding>,
    pub task: String,
    pub status: TaskStatus,
    pub nodes: Vec<TaskNodeReport>,
    pub process_root: Option<String>,
    pub root_retained: bool,
    pub logs_truncated: bool,
    pub error: Option<String>,
}
impl TaskReport {
    pub(super) fn new(task: &str, revision: TaskRevision) -> Result<Self, ExecError> {
        Ok(Self {
            version: 1,
            invocation: InvocationId::fresh()?,
            attempt: AttemptId::fresh()?,
            revision,
            execution_binding: None,
            task: task.into(),
            status: TaskStatus::Indeterminate,
            nodes: Vec::new(),
            process_root: None,
            root_retained: false,
            logs_truncated: false,
            error: None,
        })
    }
    pub fn last_receipt(&self) -> Option<&ProcessReceipt> {
        self.nodes
            .iter()
            .rev()
            .find_map(|node| node.processes.last())
    }
    pub fn path(&self, home: &Path) -> PathBuf {
        home.join(REPORT_DIRECTORY)
            .join(format!("{}.json", self.invocation))
    }
    pub(super) fn persist(&self, home: &Path) -> Result<(), ExecError> {
        let dir = directory(home, true)?;
        let name = format!("{}.json", self.invocation);
        match dir.symlink_metadata(&name) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut count = 0;
                for entry in dir.entries()? {
                    entry?;
                    count += 1;
                    if count + 2 > MAX_REPORT_ENTRIES {
                        return Err(failure(
                            "task history budget exhausted; archive completed receipts explicitly",
                        ));
                    }
                }
            }
            Err(error) => return Err(error.into()),
        }
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() as u64 > MAX_REPORT_BYTES {
            return Err(failure("task receipt exceeds byte budget"));
        }
        gripsack_fs::atomic_write_with_mode(
            &dir,
            Path::new(&format!("{}.json", self.invocation)),
            &bytes,
            0o600,
        )?;
        Ok(())
    }
}
fn directory(home: &Path, create: bool) -> Result<gripsack_fs::Dir, ExecError> {
    let home = if create {
        gripsack_fs::open_or_create(home)?
    } else {
        gripsack_fs::open(home)?
    };
    if create {
        gripsack_fs::create_dir_all(&home, Path::new(REPORT_DIRECTORY))?;
    }
    let dir = gripsack_fs::open_dir_nofollow(&home, Path::new(REPORT_DIRECTORY))?;
    Ok(dir)
}
pub fn read_task_report(home: &Path, invocation: &InvocationId) -> Result<TaskReport, ExecError> {
    let dir = directory(home, false)?;
    let bytes = read_bounded(&dir, &format!("{invocation}.json"), MAX_REPORT_BYTES)?;
    let report: TaskReport = serde_json::from_slice(&bytes)?;
    if report.version != 1
        || report.invocation != *invocation
        || report.nodes.len() > super::task_graph::MAX_TASK_NODES
    {
        return Err(failure(
            "task receipt identity/version/node budget mismatch",
        ));
    }
    if let Some(root) = &report.process_root {
        Sha256Digest::parse(root)?;
    }
    let mut total_actions = 0usize;
    for node in &report.nodes {
        total_actions = total_actions
            .checked_add(node.declared_steps)
            .and_then(|count| count.checked_add(node.declared_checks))
            .ok_or_else(|| failure("task receipt action budget overflow"))?;
        if total_actions > super::task_graph::MAX_TASK_ACTIONS
            || node.declared_steps == 0
            || node.completed_steps > node.declared_steps
            || node.completed_checks > node.declared_checks
            || node.processes.len() > node.declared_steps + node.declared_checks
            || (node.status == TaskStatus::Succeeded
                && (node.completed_steps != node.declared_steps
                    || node.completed_checks != node.declared_checks
                    || node
                        .processes
                        .iter()
                        .any(|receipt| TaskStatus::from_receipt(receipt) != TaskStatus::Succeeded)))
        {
            return Err(failure("task receipt completion evidence is inconsistent"));
        }
    }
    if report.status == TaskStatus::Succeeded
        && (report.root_retained
            || report.nodes.is_empty()
            || report.execution_binding.is_none()
            || report
                .nodes
                .iter()
                .any(|node| node.status != TaskStatus::Succeeded)
            || report
                .nodes
                .last()
                .is_none_or(|node| node.task != report.task))
    {
        return Err(failure(
            "task receipt cannot establish successful graph completion",
        ));
    }
    Ok(report)
}
pub fn read_task_log(home: &Path, invocation: &InvocationId) -> Result<Vec<u8>, ExecError> {
    read_task_report(home, invocation)?;
    read_bounded(
        &directory(home, false)?,
        &format!("{invocation}.log"),
        MAX_LOG_BYTES as u64,
    )
}
fn read_bounded(dir: &gripsack_fs::Dir, name: &str, limit: u64) -> Result<Vec<u8>, ExecError> {
    let file = gripsack_fs::open_file_nofollow(dir, Path::new(name))?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(failure("task record exceeds byte budget"));
    }
    Ok(bytes)
}
pub(super) fn persist_log(home: &Path, report: &TaskReport, bytes: &[u8]) -> Result<(), ExecError> {
    if bytes.len() > MAX_LOG_BYTES {
        return Err(failure("task log exceeds byte budget"));
    }
    gripsack_fs::atomic_write_with_mode(
        &directory(home, true)?,
        Path::new(&format!("{}.log", report.invocation)),
        bytes,
        0o600,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observed_zero_exit_cannot_override_failed_or_unknown_supervision() {
        use gripsack_process::{ByteBinding, Enforcement};
        for disposition in [
            ProcessDisposition::Exited,
            ProcessDisposition::Deadline,
            ProcessDisposition::Cancelled,
            ProcessDisposition::InputLimit,
            ProcessDisposition::LineLimit,
            ProcessDisposition::StdoutLimit,
            ProcessDisposition::StderrLimit,
            ProcessDisposition::Io,
            ProcessDisposition::CleanupFailure,
            ProcessDisposition::SpawnFailure,
        ] {
            let receipt = ProcessReceipt {
                executable_sha256: Sha256Digest::of(b"fixture"),
                script_sha256: None,
                loader_sha256: None,
                byte_binding: ByteBinding::ExecutableHandle,
                enforcement: Enforcement::ProcessGroup,
                environment_keys: Vec::new(),
                deadline_millis: 1,
                stdout_limit: 1,
                stderr_limit: 1,
                exit_code: Some(0),
                signal: None,
                disposition: disposition.clone(),
                error: None,
                cleanup_cause: None,
            };
            let status = TaskStatus::from_receipt(&receipt);
            assert_eq!(
                gripsack_policy::task::prerequisite_succeeded(status.state()),
                disposition == ProcessDisposition::Exited
            );
            if disposition == ProcessDisposition::CleanupFailure {
                assert_eq!(status, TaskStatus::Indeterminate);
            }
        }
    }
    #[test]
    fn unknown_observation_cannot_be_read_as_success() {
        let home = tempfile::tempdir().unwrap();
        let revision = TaskRevision(Sha256Digest::of(b"declaration"));
        let mut report = TaskReport::new("root", revision).unwrap();
        report.persist(home.path()).unwrap();
        assert_eq!(
            read_task_report(home.path(), &report.invocation)
                .unwrap()
                .status,
            TaskStatus::Indeterminate
        );
        report.status = TaskStatus::Succeeded;
        report.persist(home.path()).unwrap();
        assert!(read_task_report(home.path(), &report.invocation).is_err());
    }
    #[test]
    fn receipt_identity_and_log_symlinks_are_rejected() {
        let home = tempfile::tempdir().unwrap();
        let mut report =
            TaskReport::new("root", TaskRevision(Sha256Digest::of(b"declaration"))).unwrap();
        let first = report.invocation.clone();
        report.persist(home.path()).unwrap();
        let path = report.path(home.path());
        report.invocation = InvocationId::fresh().unwrap();
        std::fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
        assert!(read_task_report(home.path(), &first).is_err());
        report.persist(home.path()).unwrap();
        std::os::unix::fs::symlink(
            &path,
            home.path()
                .join(REPORT_DIRECTORY)
                .join(format!("{}.log", report.invocation)),
        )
        .unwrap();
        assert!(read_task_log(home.path(), &report.invocation).is_err());
    }
}
