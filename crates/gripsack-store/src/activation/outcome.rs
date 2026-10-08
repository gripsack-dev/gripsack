//! Versioned per-intent records. A single transition rewrites one small record,
//! not the immutable action plan or the other intents' outcomes.
use super::model::{ActivationId, IntentId};
use gripsack_policy::activation::{AttemptNumber, IntentState};
use gripsack_process::{ProcessDisposition, ProcessReceipt};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionStage {
    Environment,
    Invocation,
    Execution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IntentFailure {
    Admission {
        stage: AdmissionStage,
        error: gripsack_process::NativeIoError,
    },
    Process,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StateRecord {
    pub version: u32,
    pub instance: ActivationId,
    pub intent: IntentId,
    #[serde(with = "state")]
    pub state: IntentState,
    pub processes: Vec<ProcessReceipt>,
    pub failure: Option<IntentFailure>,
}

impl StateRecord {
    pub(super) fn pending(version: u32, instance: ActivationId, intent: IntentId) -> Self {
        Self {
            version,
            instance,
            intent,
            state: IntentState::Pending,
            processes: Vec::new(),
            failure: None,
        }
    }

    pub(super) fn admit(
        &self,
        version: u32,
        instance: ActivationId,
        intent: &super::model::EffectiveIntent,
    ) -> io::Result<()> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "activation outcome identity or terminal result is corrupt",
            )
        };
        if self.version != version || self.instance != instance || self.intent != intent.id() {
            return Err(invalid());
        }
        let required_processes = match intent.action() {
            super::ActivationAction::CustomShell { .. }
            | super::ActivationAction::WorkspaceHook { .. } => 1,
            super::ActivationAction::Fonts | super::ActivationAction::DesktopEntry => 2,
            super::ActivationAction::Service { .. } => 3,
        };
        if self.processes.len() > required_processes {
            return Err(invalid());
        }
        if !self.processes.is_empty()
            && let super::ActivationAction::CustomShell { script } = intent.action()
        {
            let digest = gripsack_process::Sha256Digest::of(script.as_bytes());
            if self
                .processes
                .iter()
                .any(|process| process.script_sha256 != Some(digest))
            {
                return Err(invalid());
            }
        }
        for process in &self.processes {
            let needs_error = matches!(
                process.disposition,
                ProcessDisposition::Io
                    | ProcessDisposition::CleanupFailure
                    | ProcessDisposition::SpawnFailure
            );
            if needs_error != process.error.is_some()
                || (process.disposition == ProcessDisposition::CleanupFailure)
                    != process.cleanup_cause.is_some()
                || process
                    .error
                    .is_some_and(|error| error.os_code.is_some_and(|code| code <= 0))
            {
                return Err(invalid());
            }
            if process
                .exit_code
                .is_some_and(|code| !(0..=255).contains(&code))
                || process.signal.is_some_and(|signal| signal <= 0)
                || (process.exit_code.is_some() && process.signal.is_some())
                || process
                    .environment_keys
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || process
                    .environment_keys
                    .iter()
                    .any(|key| !environment_key(key))
            {
                return Err(invalid());
            }
        }
        match self.state {
            IntentState::Pending | IntentState::Started { .. } | IntentState::Superseded { .. }
                if self.processes.is_empty() && self.failure.is_none() => {}
            IntentState::Succeeded { .. }
                if self.failure.is_none()
                    && self.processes.len() == required_processes
                    && self.processes.iter().all(process_succeeded) => {}
            IntentState::Failed { .. }
                if matches!(self.failure, Some(IntentFailure::Admission { .. }))
                    || (self.failure == Some(IntentFailure::Process)
                        && self
                            .processes
                            .iter()
                            .any(|process| !process_succeeded(process))) => {}
            _ => return Err(invalid()),
        }
        Ok(())
    }
}

fn process_succeeded(process: &ProcessReceipt) -> bool {
    process.disposition == ProcessDisposition::Exited
        && process.exit_code == Some(0)
        && process.signal.is_none()
}
fn environment_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub(super) mod state {
    use super::*;
    #[derive(Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum Wire {
        Pending,
        Started { attempt: u64 },
        Succeeded { attempt: u64 },
        Failed { attempt: u64 },
        Superseded { last_attempt: Option<u64> },
    }
    pub fn serialize<S: Serializer>(value: &IntentState, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match value {
            IntentState::Pending => Wire::Pending,
            IntentState::Started { attempt } => Wire::Started {
                attempt: attempt.value(),
            },
            IntentState::Succeeded { attempt } => Wire::Succeeded {
                attempt: attempt.value(),
            },
            IntentState::Failed { attempt } => Wire::Failed {
                attempt: attempt.value(),
            },
            IntentState::Superseded { last_attempt } => Wire::Superseded {
                last_attempt: last_attempt.map(AttemptNumber::value),
            },
        };
        wire.serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<IntentState, D::Error> {
        let attempt = |number| {
            AttemptNumber::admit(number).ok_or_else(|| {
                serde::de::Error::custom("activation attempt must be a positive integer")
            })
        };
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Pending => IntentState::Pending,
            Wire::Started { attempt: number } => IntentState::Started {
                attempt: attempt(number)?,
            },
            Wire::Succeeded { attempt: number } => IntentState::Succeeded {
                attempt: attempt(number)?,
            },
            Wire::Failed { attempt: number } => IntentState::Failed {
                attempt: attempt(number)?,
            },
            Wire::Superseded { last_attempt } => IntentState::Superseded {
                last_attempt: last_attempt.map(attempt).transpose()?,
            },
        })
    }
}
