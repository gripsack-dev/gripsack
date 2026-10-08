//! Stream bounded, admitted intent summaries without executing, migrating or
//! chmodding state. Immutable plans supply identity, not today's repository.
use super::{
    ledger::ActivationBatch,
    model::{self, ActivationId, Contributor, IdentityOrigin, IntentId},
    outcome::{IntentFailure, StateRecord},
    pointer::PendingPointer,
    storage::{self, Access, RecordKind},
};
use crate::GenerationId;
use gripsack_fs::Dir;
use gripsack_policy::activation::IntentState;
use gripsack_process::{ProcessReceipt, Sha256Digest};
use serde::{Serialize, Serializer, ser::SerializeStruct};
use std::{io, path::Path};

const MAX_HISTORY_ENTRIES: usize = 100_000;

#[derive(Debug, Clone, Copy)]
pub enum HookState {
    Current(IntentState),
    LegacyAmbiguous,
}
impl Serialize for HookState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Current(state) => super::outcome::state::serialize(state, serializer),
            Self::LegacyAmbiguous => {
                let mut state = serializer.serialize_struct("HookState", 1)?;
                state.serialize_field("kind", "legacy_ambiguous")?;
                state.end()
            }
        }
    }
}

impl std::fmt::Display for HookState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (state, attempt) = match self {
            Self::Current(IntentState::Pending) => ("pending", None),
            Self::Current(IntentState::Started { attempt }) => ("ambiguous", Some(*attempt)),
            Self::Current(IntentState::Succeeded { attempt }) => ("succeeded", Some(*attempt)),
            Self::Current(IntentState::Failed { attempt }) => {
                ("failed, no automatic retry", Some(*attempt))
            }
            Self::Current(IntentState::Superseded { last_attempt }) => {
                ("superseded", *last_attempt)
            }
            Self::LegacyAmbiguous => ("legacy identity unavailable", None),
        };
        formatter.write_str(state)?;
        if let Some(attempt) = attempt {
            write!(formatter, "  attempt {}", attempt.value())?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct HookInspection<'a> {
    pub activation: Option<ActivationId>,
    pub intent: Option<IntentId>,
    #[serde(with = "crate::generation_wire")]
    pub generation: GenerationId,
    pub pending: bool,
    pub identity_origin: IdentityOrigin,
    pub action_sha256: Sha256Digest,
    pub contributors: &'a [Contributor],
    pub state: HookState,
    pub processes: &'a [ProcessReceipt],
    pub failure: Option<IntentFailure>,
}

pub fn inspect(
    home: &Dir,
    mut visit: impl FnMut(HookInspection<'_>) -> io::Result<()>,
) -> io::Result<()> {
    let pointer: Option<PendingPointer> = storage::read(
        home,
        Path::new(storage::POINTER),
        RecordKind::Plan,
        Access::Inspect,
    )?;
    let active = match pointer {
        Some(PendingPointer::Current { version, instance }) => {
            let batch = ActivationBatch::open(home, instance, Access::Inspect)?;
            if batch.plan.version != version {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "activation pointer version differs from its plan",
                ));
            }
            emit(&batch, true, &mut visit)?;
            Some(instance)
        }
        Some(PendingPointer::Legacy {
            generation,
            intents,
        }) => {
            if intents.len() > storage::MAX_INTENTS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "legacy activation intent count exceeds its budget",
                ));
            }
            for declaration in intents {
                let action_sha256 = model::digest(&declaration.action)?;
                let contributors = [Contributor {
                    module: declaration.module,
                    trigger: declaration.trigger,
                }];
                visit(HookInspection {
                    activation: None,
                    intent: None,
                    generation,
                    pending: true,
                    identity_origin: IdentityOrigin::LegacyIdentityUnavailable,
                    action_sha256,
                    contributors: &contributors,
                    state: HookState::LegacyAmbiguous,
                    processes: &[],
                    failure: None,
                })?;
            }
            None
        }
        None => None,
    };
    let root = match storage::open_directory(home, Path::new(storage::ROOT), Access::Inspect) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for (index, entry) in root.read_dir(".")?.enumerate() {
        if index == MAX_HISTORY_ENTRIES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "activation history inventory exceeds its admission budget",
            ));
        }
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(transaction) = crate::selection_wire::parse_transaction(name) else {
            continue;
        };
        let instance = ActivationId(transaction);
        if active == Some(instance) {
            continue;
        }
        let directory = storage::open_directory(&root, Path::new(name), Access::Inspect)?;
        match directory.symlink_metadata(storage::RECEIPT) {
            Ok(_) => {}
            // An unpublished preparation has no receipt/active pointer. It
            // conveys no effect authority and is not invented completed work.
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
        let batch = ActivationBatch::open(home, instance, Access::Inspect)?;
        emit(&batch, false, &mut visit)?;
    }
    Ok(())
}

fn emit(
    batch: &ActivationBatch,
    pending: bool,
    visit: &mut impl FnMut(HookInspection<'_>) -> io::Result<()>,
) -> io::Result<()> {
    for (
        intent,
        StateRecord {
            state,
            processes,
            failure,
            ..
        },
    ) in batch.plan.intents.iter().zip(&batch.states)
    {
        visit(HookInspection {
            activation: Some(batch.plan.instance),
            intent: Some(intent.id),
            generation: batch.plan.selection.generation(),
            pending,
            identity_origin: batch.plan.identity_origin,
            action_sha256: intent.action_digest,
            contributors: &intent.contributors,
            state: HookState::Current(*state),
            processes,
            failure: *failure,
        })?;
    }
    Ok(())
}
