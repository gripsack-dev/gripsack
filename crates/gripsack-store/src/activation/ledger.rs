//! Persistent intent records, executed for the kernel's activation machine
//! (`gripsack_kernel::activation`): each method is one command. Only a
//! durably recorded Started transition yields a launch permit.
use super::{
    PendingIntent,
    model::{self, ActivationId, EffectiveIntent, IdentityOrigin, IntentId, Plan},
    outcome::{IntentFailure, StateRecord},
    pointer::{PendingPointer, PointerRecord},
    storage::{self, Access, RecordKind},
};
use crate::{journal::JournalRun, private_state};
use gripsack_fs::Dir;
use gripsack_kernel::activation::{AttemptNumber, BatchFacts, IntentState, terminal};
use gripsack_kernel::selection::{SelectionIdentity, TransactionId};
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
    intent: IntentId,
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

pub fn has_pending(home: &Dir) -> io::Result<bool> {
    match home.symlink_metadata(storage::POINTER) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn prepare(
    home: &Dir,
    selection: &JournalRun,
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
        version: super::FORMAT_VERSION,
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
        .map(|intent| StateRecord::pending(plan.version, instance, intent.id))
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
            version: plan.version,
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
        PendingPointer::Current { version, instance } => {
            let batch = ActivationBatch::open(home, instance, Access::Recover)?;
            if batch.plan.version != version {
                return Err(invalid("activation pointer version differs from its plan"));
            }
            Ok(Some(batch))
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
            if receipt.version != plan.version
                || receipt.instance != instance
                || receipt.plan_sha256 != plan_digest
                || receipt.outcomes.len() != plan.intents.len()
            {
                return Err(invalid("activation receipt does not bind its plan"));
            }
            for (state, intent) in receipt.outcomes.iter().zip(&plan.intents) {
                state.admit(plan.version, instance, intent)?;
                if !terminal(&state.state) {
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
                state.admit(plan.version, instance, intent)?;
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

    /// The batch as the kernel's activation machine reads it.
    pub fn facts(&self) -> BatchFacts {
        BatchFacts {
            selection: self.plan.selection,
            // MAX_INTENTS bounds every admitted plan well below u32::MAX
            intents: self.plan.intents.len() as u32,
        }
    }

    /// Intent `index`'s durable record.
    pub fn record(&self, index: u32) -> io::Result<IntentState> {
        self.states
            .get(index as usize)
            .map(|record| record.state)
            .ok_or_else(|| invalid("unknown activation intent ordinal"))
    }

    /// `Authorize`: admit the selected generation's manifest and seal the
    /// home before any saved action runs. The caller holds the lifecycle
    /// lock; the kernel issues this only when `current` is the batch's exact
    /// selection.
    pub fn authorize(&self, home: &Dir, home_path: &Path) -> io::Result<()> {
        crate::generations::admit_manifest_at(home, home_path, self.plan.selection.generation())?;
        gripsack_fs::fsync_dir(home, Path::new("."))
    }

    /// `Persist`: write intent `index`'s record durably (file and directory
    /// barrier). A terminal outcome carries the attempt's process receipts.
    pub fn persist(
        &mut self,
        index: u32,
        state: IntentState,
        processes: Vec<ProcessReceipt>,
        failure: Option<IntentFailure>,
    ) -> io::Result<()> {
        let index = index as usize;
        let prior = self
            .states
            .get(index)
            .ok_or_else(|| invalid("unknown activation intent ordinal"))?;
        let next = StateRecord {
            state,
            processes,
            failure,
            ..StateRecord::pending(prior.version, prior.instance, prior.intent)
        };
        next.admit(
            self.plan.version,
            self.plan.instance,
            &self.plan.intents[index],
        )?;
        storage::write(
            &self.outcomes,
            Path::new(&state_name(next.intent)),
            &next,
            RecordKind::Outcome,
        )?;
        self.states[index] = next;
        Ok(())
    }

    /// `RunHook`: the launch authority for attempt `attempt` of intent
    /// `index`, granted only while that intent's durable record is exactly
    /// `Started { attempt }`.
    pub fn launch(
        &self,
        index: u32,
        attempt: AttemptNumber,
    ) -> io::Result<(LaunchPermit, &EffectiveIntent)> {
        if self.record(index)? != (IntentState::Started { attempt }) {
            return Err(invalid("activation attempt is not durably started"));
        }
        let intent = &self.plan.intents[index as usize];
        Ok((
            LaunchPermit {
                intent: intent.id,
                attempt,
            },
            intent,
        ))
    }

    /// `Archive`: write the receipt binding every terminal outcome to the
    /// plan (an archived batch already has one).
    pub fn archive(&self) -> io::Result<()> {
        if self.states.iter().any(|record| !terminal(&record.state)) {
            return Err(invalid("activation still has unsettled intents"));
        }
        if self.archived {
            return Ok(());
        }
        storage::write(
            &self.directory,
            Path::new(storage::RECEIPT),
            &ReceiptReference {
                version: self.plan.version,
                instance: self.plan.instance,
                plan_sha256: self.plan_digest,
                outcomes: &self.states,
            },
            RecordKind::Receipt,
        )
    }

    /// `Clear`: remove the pending pointer durably, only if it still names
    /// this batch.
    pub fn clear(&self, home: &Dir) -> io::Result<()> {
        let pointer: PendingPointer = storage::required(
            home,
            Path::new(storage::POINTER),
            RecordKind::Plan,
            Access::Recover,
        )?;
        if !matches!(pointer, PendingPointer::Current { version, instance } if instance == self.plan.instance && version == self.plan.version)
        {
            return Err(invalid(
                "activation pointer changed before completed-outcome cleanup",
            ));
        }
        gripsack_fs::remove_file(home, Path::new(storage::POINTER))?;
        gripsack_fs::fsync_dir(home, Path::new("."))
    }
}

fn state_name(intent: IntentId) -> String {
    format!("{intent}.json")
}
fn invalid(message: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
