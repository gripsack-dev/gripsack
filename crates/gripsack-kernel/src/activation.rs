//! The activation machine (plan/0056 §4, F2 step 5): running a committed
//! selection's post-activation hooks from their durable ledger, as a sans-IO
//! state machine.
//!
//! An activation batch is the immutable, ordered list of hook intents saved
//! before the flip, bound to the selection that flip committed, with one
//! durable record per intent (`Pending`, `Started`, `Succeeded`, `Failed`,
//! `Superseded`). The machine reads the batch, then the live `current`
//! selection, and decides:
//!
//! - **authorize** the batch when `current` is exactly its selection
//!   (transaction bytes included), then walk the intents in ledger order:
//!   a terminal record is skipped; any other record gets a new attempt
//!   number (`Pending` → 1, an interrupted `Started n` → `n + 1`, under the
//!   same intent), which is persisted as `Started` before the hook runs; the
//!   hook's return is persisted as `Succeeded` or `Failed` and the machine
//!   moves on — a failure is recorded, never retried, never rolled back
//!   (0001 §3.8);
//! - **supersede** the batch when `current` is anything else: every
//!   non-terminal record becomes `Superseded` and no hook runs at all;
//! - in both cases, once every intent is settled, archive the receipt, then
//!   clear the pending pointer.
//!
//! The driver re-checks `current` against the hook's `expect` selection
//! immediately before launching it and answers `Failed(Drift)` on mismatch.
//!
//! Proved (proofs/Kernel/Activation.lean): a hook runs only right after its
//! `Started` record was acknowledged durable, for the intent under decision,
//! with the attempt number that replays its previous record, and only when
//! the run read `current` as the batch's selection and authorized it; each
//! intent runs at most once per run; a terminal record is never rewritten
//! and never runs; a `Succeeded`/`Failed` record is written only from the
//! returned hook; a superseded batch runs no hook; the machine leaves an
//! intent only once its record is terminal; the pointer is cleared only after
//! the archive, which follows every intent.
//!
//! Aeneas shape (lib.rs): no derives (impls in `convenience.rs`), equality as
//! explicit `same` functions, owned `Copy` facts, no closures; counters
//! advance with `checked_add`, so every step is total.
use crate::recover::Failure;
use crate::selection::SelectionIdentity;

/// A positive attempt number. The wire admits only positive numbers.
pub struct AttemptNumber {
    number: u64,
}

impl AttemptNumber {
    /// Admit a wire number: every positive number, never zero.
    pub fn admit(number: u64) -> Option<AttemptNumber> {
        if number == 0 {
            None
        } else {
            Some(AttemptNumber { number })
        }
    }

    pub fn first() -> AttemptNumber {
        AttemptNumber { number: 1 }
    }

    pub fn value(self) -> u64 {
        self.number
    }

    pub fn same(self, other: AttemptNumber) -> bool {
        self.number == other.number
    }

    pub fn checked_next(self) -> Option<AttemptNumber> {
        if self.number == u64::MAX {
            None
        } else {
            Some(AttemptNumber {
                number: self.number + 1,
            })
        }
    }
}

/// One intent's durable record.
pub enum IntentState {
    Pending,
    /// Attempt `attempt` was durably started; its outcome is unknown until a
    /// terminal record follows. After a crash this is ambiguous: the hook may
    /// or may not have run.
    Started {
        attempt: AttemptNumber,
    },
    Succeeded {
        attempt: AttemptNumber,
    },
    Failed {
        attempt: AttemptNumber,
    },
    /// The batch's selection was no longer current; the intent never runs.
    Superseded {
        last_attempt: Option<AttemptNumber>,
    },
}

/// What a hook returned.
pub enum HookResult {
    Succeeded,
    Failed,
}

/// The next attempt for a record.
pub enum StartDecision {
    Start(AttemptNumber),
    /// The record is terminal: the intent never runs again.
    Terminal,
    /// The attempt counter is exhausted: refuse, keep the evidence.
    Exhausted,
}

/// `Succeeded`, `Failed` and `Superseded` are terminal.
pub fn terminal(state: &IntentState) -> bool {
    match state {
        IntentState::Pending | IntentState::Started { .. } => false,
        IntentState::Succeeded { .. }
        | IntentState::Failed { .. }
        | IntentState::Superseded { .. } => true,
    }
}

/// The last attempt a record names.
pub fn last_attempt(state: &IntentState) -> Option<AttemptNumber> {
    match state {
        IntentState::Pending => None,
        IntentState::Started { attempt }
        | IntentState::Succeeded { attempt }
        | IntentState::Failed { attempt } => Some(*attempt),
        IntentState::Superseded { last_attempt } => *last_attempt,
    }
}

/// The attempt to persist as `Started` before the next launch: the first for
/// a pending intent, the successor of an interrupted one; never for a
/// terminal record.
pub fn next_attempt(state: &IntentState) -> StartDecision {
    match state {
        IntentState::Pending => StartDecision::Start(AttemptNumber::first()),
        IntentState::Started { attempt } => match attempt.checked_next() {
            Some(next) => StartDecision::Start(next),
            None => StartDecision::Exhausted,
        },
        IntentState::Succeeded { .. }
        | IntentState::Failed { .. }
        | IntentState::Superseded { .. } => StartDecision::Terminal,
    }
}

/// The terminal record for a returned attempt: only the matching in-flight
/// attempt settles, so a late result cannot overwrite a later attempt or a
/// terminal record.
pub fn finish_attempt(
    state: &IntentState,
    attempt: AttemptNumber,
    result: HookResult,
) -> Option<IntentState> {
    match state {
        IntentState::Started { attempt: active } => {
            if active.same(attempt) {
                match result {
                    HookResult::Succeeded => Some(IntentState::Succeeded { attempt }),
                    HookResult::Failed => Some(IntentState::Failed { attempt }),
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Supersede a record: a non-terminal record becomes `Superseded`, keeping
/// its last attempt; a terminal record stays as it is.
pub fn supersede(state: &IntentState) -> IntentState {
    match state {
        IntentState::Pending => IntentState::Superseded { last_attempt: None },
        IntentState::Started { attempt } => IntentState::Superseded {
            last_attempt: Some(*attempt),
        },
        IntentState::Succeeded { attempt } => IntentState::Succeeded { attempt: *attempt },
        IntentState::Failed { attempt } => IntentState::Failed { attempt: *attempt },
        IntentState::Superseded { last_attempt } => IntentState::Superseded {
            last_attempt: *last_attempt,
        },
    }
}

/// The pending batch, as the host admitted it.
pub struct BatchFacts {
    /// The selection the flip committed when the batch was saved.
    pub selection: SelectionIdentity,
    /// How many intents the batch orders.
    pub intents: u32,
}

/// Whether the batch runs its hooks or supersedes them.
pub enum Mode {
    Run,
    Supersede,
}

pub enum Outcome {
    /// No batch was pending.
    Nothing,
    /// The batch's hooks ran (each recorded `Succeeded` or `Failed`).
    Activated,
    /// `current` was not the batch's selection: nothing ran.
    Superseded,
}

/// Why the machine stopped. Every record stays as it was last persisted: an
/// interrupted `Started` replays (or is superseded) on the next run.
pub enum Refusal {
    /// The intent's attempt counter is exhausted.
    Exhausted { intent: u32 },
    /// `current` changed between authorization and the launch.
    Drifted { intent: u32 },
    /// A counter overflowed.
    Malformed,
    /// The host failed to execute a command.
    Failed,
    /// An event that does not answer the outstanding command.
    Protocol,
}

pub enum Command {
    /// Admit the pending batch: answer `BatchRead`, or `NoBatch`.
    ReadBatch,
    /// Read the selection `current` names: answer `CurrentRead`.
    ReadCurrent,
    /// Admit the selection's generation and seal it before any hook; answer
    /// `Done`.
    Authorize(SelectionIdentity),
    /// Read intent `k`'s durable record: answer `IntentRead`.
    ReadIntent(u32),
    /// Write intent `intent`'s record durably (file and directory barrier);
    /// answer `Done` only once it is durable.
    Persist {
        intent: u32,
        state: IntentState,
    },
    /// Re-check that `current` is `expect` immediately before launching
    /// (answer `Failed(Drift)` on mismatch), run attempt `attempt` of intent
    /// `intent`, and answer `Returned` with what it returned.
    RunHook {
        intent: u32,
        attempt: AttemptNumber,
        expect: SelectionIdentity,
    },
    /// Write the batch's receipt durably; answer `Done`.
    Archive,
    /// Remove the pending pointer durably; answer `Done`.
    Clear,
    Finish(Outcome),
    Abort(Refusal),
}

pub enum Event {
    Start,
    NoBatch,
    BatchRead(BatchFacts),
    CurrentRead(Option<SelectionIdentity>),
    IntentRead(IntentState),
    Done,
    Returned(HookResult),
    Failed(Failure),
}

pub enum Phase {
    Start,
    ReadingBatch,
    ReadingCurrent,
    Authorizing,
    ReadingIntent,
    /// A `Superseded` record is being persisted.
    Superseding,
    /// A `Started` record is being persisted.
    Starting,
    /// A hook is running.
    Running,
    /// A `Succeeded`/`Failed` record is being persisted.
    Settling,
    Archiving,
    Clearing,
    Finished,
    Aborted,
}

pub struct ActivationState {
    pub phase: Phase,
    pub mode: Mode,
    pub batch: BatchFacts,
    /// What `current` named when the run read it.
    pub current: Option<SelectionIdentity>,
    /// The intent under decision.
    pub index: u32,
    /// Its record: as read, or as the outstanding `Persist` writes it.
    pub record: IntentState,
}

impl ActivationState {
    pub fn new() -> ActivationState {
        ActivationState {
            phase: Phase::Start,
            mode: Mode::Supersede,
            batch: BatchFacts {
                selection: SelectionIdentity::Legacy {
                    generation: crate::GenerationId::new(0),
                },
                intents: 0,
            },
            current: None,
            index: 0,
            record: IntentState::Pending,
        }
    }
}

/// `current` is exactly the batch's selection. (The parameter is not called
/// `selection`: a local named like a module shadows it in the translation.)
pub fn bound(current: &Option<SelectionIdentity>, batch: &SelectionIdentity) -> bool {
    match current {
        Some(current) => current.same(batch),
        None => false,
    }
}

fn refuse(mut state: ActivationState, refusal: Refusal) -> (ActivationState, Command) {
    state.phase = Phase::Aborted;
    (state, Command::Abort(refusal))
}

fn failed_or_protocol(state: ActivationState, event: Event) -> (ActivationState, Command) {
    match event {
        Event::Failed(_) => refuse(state, Refusal::Failed),
        _ => refuse(state, Refusal::Protocol),
    }
}

/// The next intent, or the archive once every intent is settled.
fn proceed(mut state: ActivationState) -> (ActivationState, Command) {
    if state.index < state.batch.intents {
        state.phase = Phase::ReadingIntent;
        let intent = state.index;
        (state, Command::ReadIntent(intent))
    } else {
        state.phase = Phase::Archiving;
        (state, Command::Archive)
    }
}

/// The intent under decision is settled: move to the next one.
fn advance(mut state: ActivationState) -> (ActivationState, Command) {
    match state.index.checked_add(1) {
        None => refuse(state, Refusal::Malformed),
        Some(next) => {
            state.index = next;
            proceed(state)
        }
    }
}

fn persist(
    mut state: ActivationState,
    phase: Phase,
    record: IntentState,
) -> (ActivationState, Command) {
    state.phase = phase;
    state.record = record;
    let intent = state.index;
    (
        state,
        Command::Persist {
            intent,
            state: record,
        },
    )
}

fn on_current(
    mut state: ActivationState,
    current: Option<SelectionIdentity>,
) -> (ActivationState, Command) {
    state.current = current;
    state.index = 0;
    if bound(&current, &state.batch.selection) {
        state.mode = Mode::Run;
        state.phase = Phase::Authorizing;
        let selection = state.batch.selection;
        (state, Command::Authorize(selection))
    } else {
        state.mode = Mode::Supersede;
        proceed(state)
    }
}

fn on_intent(mut state: ActivationState, record: IntentState) -> (ActivationState, Command) {
    state.record = record;
    match state.mode {
        Mode::Supersede => {
            if terminal(&record) {
                advance(state)
            } else {
                persist(state, Phase::Superseding, supersede(&record))
            }
        }
        Mode::Run => match next_attempt(&record) {
            StartDecision::Terminal => advance(state),
            StartDecision::Exhausted => {
                let intent = state.index;
                refuse(state, Refusal::Exhausted { intent })
            }
            StartDecision::Start(attempt) => {
                persist(state, Phase::Starting, IntentState::Started { attempt })
            }
        },
    }
}

/// The `Started` record is durable: launch.
fn on_started(mut state: ActivationState) -> (ActivationState, Command) {
    match state.record {
        IntentState::Started { attempt } => {
            state.phase = Phase::Running;
            let intent = state.index;
            let expect = state.batch.selection;
            (
                state,
                Command::RunHook {
                    intent,
                    attempt,
                    expect,
                },
            )
        }
        _ => refuse(state, Refusal::Protocol),
    }
}

fn on_returned(state: ActivationState, result: HookResult) -> (ActivationState, Command) {
    match state.record {
        IntentState::Started { attempt } => match finish_attempt(&state.record, attempt, result) {
            Some(settled) => persist(state, Phase::Settling, settled),
            None => refuse(state, Refusal::Protocol),
        },
        _ => refuse(state, Refusal::Protocol),
    }
}

fn on_launch_failed(state: ActivationState, failure: Failure) -> (ActivationState, Command) {
    match failure {
        Failure::Drift => {
            let intent = state.index;
            refuse(state, Refusal::Drifted { intent })
        }
        Failure::Io(_) => refuse(state, Refusal::Failed),
    }
}

fn finished(mut state: ActivationState) -> (ActivationState, Command) {
    state.phase = Phase::Finished;
    let outcome = match state.mode {
        Mode::Run => Outcome::Activated,
        Mode::Supersede => Outcome::Superseded,
    };
    (state, Command::Finish(outcome))
}

/// One step: consume the answer to the previous command (or `Start`) and
/// return the next command. Any event that does not answer the outstanding
/// command aborts with [`Refusal::Protocol`]; a failed command aborts with
/// [`Refusal::Failed`] (or [`Refusal::Drifted`] for a launch's re-check).
pub fn activation_step(state: ActivationState, event: Event) -> (ActivationState, Command) {
    match state.phase {
        Phase::Start => match event {
            Event::Start => {
                let mut next = state;
                next.phase = Phase::ReadingBatch;
                (next, Command::ReadBatch)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingBatch => match event {
            Event::NoBatch => {
                let mut next = state;
                next.phase = Phase::Finished;
                (next, Command::Finish(Outcome::Nothing))
            }
            Event::BatchRead(batch) => {
                let mut next = state;
                next.batch = batch;
                next.phase = Phase::ReadingCurrent;
                (next, Command::ReadCurrent)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingCurrent => match event {
            Event::CurrentRead(current) => on_current(state, current),
            _ => failed_or_protocol(state, event),
        },
        Phase::Authorizing => match event {
            Event::Done => proceed(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::ReadingIntent => match event {
            Event::IntentRead(record) => on_intent(state, record),
            _ => failed_or_protocol(state, event),
        },
        Phase::Superseding | Phase::Settling => match event {
            Event::Done => advance(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Starting => match event {
            Event::Done => on_started(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Running => match event {
            Event::Returned(result) => on_returned(state, result),
            Event::Failed(failure) => on_launch_failed(state, failure),
            _ => refuse(state, Refusal::Protocol),
        },
        Phase::Archiving => match event {
            Event::Done => {
                let mut next = state;
                next.phase = Phase::Clearing;
                (next, Command::Clear)
            }
            _ => failed_or_protocol(state, event),
        },
        Phase::Clearing => match event {
            Event::Done => finished(state),
            _ => failed_or_protocol(state, event),
        },
        Phase::Finished | Phase::Aborted => refuse(state, Refusal::Protocol),
    }
}
