//! Persistent intent authority. Only a durably recorded Started transition
//! yields a launch permit; only a matching returned permit can settle it.
use super::{
    PendingIntent,
    model::{self, ActivationId, EffectiveIntent, IdentityOrigin, IntentId, IntentOrdinal, Plan},
    outcome::{IntentFailure, StateRecord},
    pointer::{PendingPointer, PointerRecord},
    storage::{self, Access, RecordKind},
};
use crate::{journal::PendingSelection, private_state};
use gripsack_fs::Dir;
use gripsack_policy::{
    activation::{self, AttemptNumber, IntentState, Outcome, StartDecision},
    selection::{SelectionIdentity, TransactionId},
};
use gripsack_process::{ProcessReceipt, Sha256Digest};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

pub struct ActivationBatch {
    pub(super) directory: Dir,
    outcomes: Dir,
    pub(super) plan: Plan,
    plan_digest: Sha256Digest,
    pub(super) states: Vec<StateRecord>,
    archived: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    instance: ActivationId,
    plan_sha256: Sha256Digest,
    outcomes: Vec<StateRecord>,
}
#[derive(Serialize)]
struct ReceiptReference<'a> {
    version: u32,
    instance: ActivationId,
    plan_sha256: Sha256Digest,
    outcomes: &'a [StateRecord],
}

/// Opaque, non-cloneable authority for precisely one persisted attempt.
pub struct LaunchPermit {
    instance: ActivationId,
    intent: IntentId,
    ordinal: IntentOrdinal,
    attempt: AttemptNumber,
}
impl LaunchPermit {
    pub fn intent_id(&self) -> IntentId {
        self.intent
    }
    pub fn attempt(&self) -> AttemptNumber {
        self.attempt
    }
}

pub struct ReadyActivation {
    batch: ActivationBatch,
    cursor: usize,
    active: Option<IntentOrdinal>,
}

pub fn has_pending(home: &Dir) -> io::Result<bool> {
    match home.symlink_metadata(storage::POINTER) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn prepare(
    home: &Dir,
    selection: &PendingSelection,
    declarations: Vec<PendingIntent>,
) -> io::Result<Option<ActivationBatch>> {
    if declarations.is_empty() {
        return Ok(None);
    }
    if has_pending(home)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "unfinished activation must be admitted before preparing another",
        ));
    }
    let transaction = selection.identity().transaction_id().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "activation requires a transaction-bound selection",
        )
    })?;
    create(
        home,
        ActivationId(*transaction),
        *selection.identity(),
        IdentityOrigin::Transaction,
        declarations,
    )
    .map(Some)
}

fn create(
    home: &Dir,
    instance: ActivationId,
    selection: SelectionIdentity,
    identity_origin: IdentityOrigin,
    declarations: Vec<PendingIntent>,
) -> io::Result<ActivationBatch> {
    if declarations.len() > storage::MAX_INTENTS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "activation intent count exceeds its budget",
        ));
    }
    let intents = model::effective(instance, declarations)?;
    let plan = Plan {
        version: 1,
        instance,
        selection,
        identity_origin,
        intents,
    };
    plan.admit(instance)?;
    let bytes = storage::encode(&plan, RecordKind::Plan)?;
    let plan_digest = Sha256Digest::of(&bytes);
    let root = private_state::ensure_directory(home, Path::new(storage::ROOT))?;
    let name = instance.to_string();
    root.create_dir(&name)?; // collision refusal, never overwrite/reuse an ID
    let directory = storage::open_directory(&root, Path::new(&name), Access::Recover)?;
    let outcomes = private_state::ensure_directory(&directory, Path::new(storage::OUTCOMES))?;
    gripsack_fs::atomic_write_with_mode(&directory, Path::new(storage::PLAN), &bytes, 0o600)?;
    let states: Vec<_> = plan
        .intents
        .iter()
        .map(|intent| StateRecord::pending(instance, intent.id))
        .collect();
    for state in &states {
        storage::write(
            &outcomes,
            Path::new(&state_name(state.intent)),
            state,
            RecordKind::Outcome,
        )?;
    }
    gripsack_fs::fsync_dir(&root, Path::new("."))?;
    storage::write(
        home,
        Path::new(storage::POINTER),
        &PointerRecord {
            version: 1,
            instance,
        },
        RecordKind::Pointer,
    )?;
    Ok(ActivationBatch {
        directory,
        outcomes,
        plan,
        plan_digest,
        states,
        archived: false,
    })
}

pub fn load_pending(home: &Dir, home_path: &Path) -> io::Result<Option<ActivationBatch>> {
    // Historical records contain their full action list, so their admission
    // bound is the plan bound, not the small current-pointer encoding bound.
    let Some(pointer) = storage::read::<PendingPointer>(
        home,
        Path::new(storage::POINTER),
        RecordKind::Plan,
        Access::Recover,
    )?
    else {
        return Ok(None);
    };
    match pointer {
        PendingPointer::Current(instance) => {
            ActivationBatch::open(home, instance, Access::Recover).map(Some)
        }
        PendingPointer::Legacy {
            generation,
            intents,
        } => {
            let current = crate::generations::current_selection_in(home_path, home)?;
            let selection = current
                .filter(|selected| selected.generation() == generation)
                .unwrap_or_else(|| SelectionIdentity::legacy(generation));
            let mut bytes = [0; 32];
            getrandom::fill(&mut bytes).map_err(|error| {
                io::Error::other(format!("activation identity entropy: {error}"))
            })?;
            // The complete migrated plan/pointer is durable BEFORE first replay.
            create(
                home,
                ActivationId(TransactionId::from_bytes(bytes)),
                selection,
                IdentityOrigin::LegacyIdentityUnavailable,
                intents,
            )
            .map(Some)
        }
    }
}

impl ActivationBatch {
    pub(super) fn open(home: &Dir, instance: ActivationId, access: Access) -> io::Result<Self> {
        let root = storage::open_directory(home, Path::new(storage::ROOT), access)?;
        let directory = storage::open_directory(&root, Path::new(&instance.to_string()), access)?;
        let bytes = storage::read_bytes(
            &directory,
            Path::new(storage::PLAN),
            RecordKind::Plan,
            access,
        )?
        .ok_or_else(|| invalid("activation plan is missing"))?;
        let plan_digest = Sha256Digest::of(&bytes);
        let plan: Plan =
            serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
        plan.admit(instance)?;
        if plan.intents.len() > storage::MAX_INTENTS {
            return Err(invalid("activation intent count exceeds its budget"));
        }
        let outcomes = storage::open_directory(&directory, Path::new(storage::OUTCOMES), access)?;
        let receipt: Option<Receipt> = storage::read(
            &directory,
            Path::new(storage::RECEIPT),
            RecordKind::Receipt,
            access,
        )?;
        let (states, archived) = if let Some(receipt) = receipt {
            if receipt.version != 1
                || receipt.instance != instance
                || receipt.plan_sha256 != plan_digest
                || receipt.outcomes.len() != plan.intents.len()
            {
                return Err(invalid("activation receipt does not bind its plan"));
            }
            for (state, intent) in receipt.outcomes.iter().zip(&plan.intents) {
                state.admit(instance, intent)?;
                if !terminal(state.state) {
                    return Err(invalid("archived activation has an unsettled intent"));
                }
            }
            (receipt.outcomes, true)
        } else {
            let mut states = Vec::with_capacity(plan.intents.len());
            for intent in &plan.intents {
                let state: StateRecord = storage::required(
                    &outcomes,
                    Path::new(&state_name(intent.id)),
                    RecordKind::Outcome,
                    access,
                )?;
                state.admit(instance, intent)?;
                states.push(state);
            }
            (states, false)
        };
        Ok(Self {
            directory,
            outcomes,
            plan,
            plan_digest,
            states,
            archived,
        })
    }

    /// The lifecycle lock is held by the caller. A mismatching selection is
    /// explicit supersession, never authority to run a saved action.
    pub fn authorize(
        mut self,
        home: &Dir,
        home_path: &Path,
    ) -> io::Result<Option<ReadyActivation>> {
        let current = crate::generations::current_selection_in(home_path, home)?;
        if current.as_ref() != Some(&self.plan.selection) {
            for index in 0..self.states.len() {
                let prior = &self.states[index];
                let next = activation::supersede(&prior.state);
                if next != prior.state {
                    let state = StateRecord {
                        state: next,
                        ..StateRecord::pending(prior.instance, prior.intent)
                    };
                    self.persist(index, state)?;
                }
            }
            self.archive_and_clear(home)?;
            return Ok(None);
        }
        crate::generations::admit_manifest_at(home, home_path, self.plan.selection.generation())?;
        gripsack_fs::fsync_dir(home, Path::new("."))?;
        Ok(Some(ReadyActivation {
            batch: self,
            cursor: 0,
            active: None,
        }))
    }

    fn persist(&mut self, index: usize, next: StateRecord) -> io::Result<()> {
        next.admit(self.plan.instance, &self.plan.intents[index])?;
        storage::write(
            &self.outcomes,
            Path::new(&state_name(next.intent)),
            &next,
            RecordKind::Outcome,
        )?;
        self.states[index] = next;
        Ok(())
    }

    fn archive_and_clear(self, home: &Dir) -> io::Result<()> {
        if self.states.iter().any(|record| !terminal(record.state)) {
            return Err(invalid("activation still has unsettled intents"));
        }
        if !self.archived {
            storage::write(
                &self.directory,
                Path::new(storage::RECEIPT),
                &ReceiptReference {
                    version: 1,
                    instance: self.plan.instance,
                    plan_sha256: self.plan_digest,
                    outcomes: &self.states,
                },
                RecordKind::Receipt,
            )?;
        }
        let pointer: PendingPointer = storage::required(
            home,
            Path::new(storage::POINTER),
            RecordKind::Plan,
            Access::Recover,
        )?;
        if !matches!(pointer, PendingPointer::Current(instance) if instance == self.plan.instance) {
            return Err(invalid(
                "activation pointer changed before completed-outcome cleanup",
            ));
        }
        gripsack_fs::remove_file(home, Path::new(storage::POINTER))?;
        gripsack_fs::fsync_dir(home, Path::new("."))
    }
}

impl ReadyActivation {
    pub fn next_attempt(&mut self) -> io::Result<Option<(LaunchPermit, &EffectiveIntent)>> {
        if self.active.is_some() {
            return Err(invalid("an activation attempt is still unsettled"));
        }
        while self.cursor < self.batch.plan.intents.len() {
            let index = self.cursor;
            self.cursor += 1;
            let record = &self.batch.states[index];
            let attempt = match activation::next_attempt(&record.state) {
                StartDecision::Terminal => continue,
                StartDecision::Exhausted => {
                    return Err(invalid(
                        "activation attempt counter exhausted; evidence retained",
                    ));
                }
                StartDecision::Start(attempt) => attempt,
            };
            let next = StateRecord {
                state: IntentState::Started { attempt },
                ..StateRecord::pending(record.instance, record.intent)
            };
            self.batch.persist(index, next)?;
            let intent = &self.batch.plan.intents[index];
            self.active = Some(intent.ordinal);
            return Ok(Some((
                LaunchPermit {
                    instance: self.batch.plan.instance,
                    intent: intent.id,
                    ordinal: intent.ordinal,
                    attempt,
                },
                intent,
            )));
        }
        Ok(None)
    }

    pub fn finish(
        &mut self,
        permit: LaunchPermit,
        outcome: Outcome,
        processes: Vec<ProcessReceipt>,
        failure: Option<IntentFailure>,
    ) -> io::Result<()> {
        if self.active != Some(permit.ordinal) || self.batch.plan.instance != permit.instance {
            return Err(invalid(
                "activation result does not own the in-flight attempt",
            ));
        }
        let index = permit.ordinal.index();
        let current = self
            .batch
            .states
            .get(index)
            .ok_or_else(|| invalid("unknown activation intent ordinal"))?;
        if current.intent != permit.intent {
            return Err(invalid("activation result changed its intent identity"));
        }
        let state = activation::finish_attempt(&current.state, permit.attempt, outcome)
            .ok_or_else(|| invalid("activation result is not for the current attempt"))?;
        let next = StateRecord {
            version: 1,
            instance: permit.instance,
            intent: permit.intent,
            state,
            processes,
            failure,
        };
        self.batch.persist(index, next)?;
        self.active = None;
        Ok(())
    }

    pub fn archive(self, home: &Dir) -> io::Result<()> {
        if self.active.is_some() {
            return Err(invalid("activation outcome is not durable"));
        }
        self.batch.archive_and_clear(home)
    }
}

fn terminal(state: IntentState) -> bool {
    matches!(activation::next_attempt(&state), StartDecision::Terminal)
}
fn state_name(intent: IntentId) -> String {
    format!("{intent}.json")
}
fn invalid(message: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
