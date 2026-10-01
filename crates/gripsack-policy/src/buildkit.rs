//! Decisions consumed by the independent LLB checker. Protobuf decoding,
//! source capture, digest collision resistance and BuildKit execution are
//! separate trusted/tested boundaries; these contracts do not prove the solver.
use vstd::prelude::*;

verus! {

/// A topologically ordered plan cannot reference itself or a future producer.
pub fn backward_edge(producer: usize, consumer: usize) -> (accepted: bool)
    ensures accepted == (producer < consumer),
{
    producer < consumer
}

/// The pinned upstream NetMode/SecurityMode encodings, not user policy knobs.
pub const NETWORK_NONE: i32 = 2;
pub const SECURITY_SANDBOX: i32 = 0;
pub const MOUNT_BIND: i32 = 0;
pub const NO_OUTPUT: i64 = -1;
pub const SELECTED_OUTPUT: i64 = 0;

pub fn isolated_execution(network: i32, security: i32) -> (accepted: bool)
    ensures accepted == (network == NETWORK_NONE && security == SECURITY_SANDBOX),
{
    network == NETWORK_NONE && security == SECURITY_SANDBOX
}

/// A process may mutate only its selected output mount. Every tool/root/input
/// mount is a read-only bind and cannot manufacture another output reference.
pub fn output_mount(kind: i32, readonly: bool, output: i64, selected: bool) -> (accepted: bool)
    ensures accepted == (kind == MOUNT_BIND && readonly == !selected
        && output == (if selected { SELECTED_OUTPUT } else { NO_OUTPUT })),
{
    kind == MOUNT_BIND && readonly == !selected
        && output == (if selected { SELECTED_OUTPUT } else { NO_OUTPUT })
}

/// The byte checker establishes these indices from verified vertex digests.
/// They are vertex positions, never recipe IDs or output indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexIndex { value: usize }
impl VertexIndex {
    pub closed spec fn verified_position(self) -> usize { self.value }
    pub fn new(value: usize) -> (result: Self)
        ensures result.verified_position() == value,
    { Self { value } }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputBinding { pub vertex: VertexIndex, pub output: i64 }

pub fn input_binding(actual: InputBinding, expected: InputBinding) -> (accepted: bool)
    ensures accepted == (actual.vertex.verified_position() == expected.vertex.verified_position()
        && actual.output == expected.output),
{
    actual.vertex.value == expected.vertex.value && actual.output == expected.output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStage { AwaitingAcceptance, Running, Exported, Done, Failed, Cancelled }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent { Accepted, Log, Exported, Done, Failed, Cancelled }

pub open spec fn next_stage(stage: SessionStage, event: SessionEvent) -> Option<SessionStage> {
    match (stage, event) {
        (SessionStage::AwaitingAcceptance, SessionEvent::Accepted) => Some(SessionStage::Running),
        (SessionStage::Running, SessionEvent::Log) => Some(SessionStage::Running),
        (SessionStage::Running, SessionEvent::Exported) => Some(SessionStage::Exported),
        (SessionStage::Exported, SessionEvent::Done) => Some(SessionStage::Done),
        (SessionStage::AwaitingAcceptance, SessionEvent::Failed)
        | (SessionStage::Running, SessionEvent::Failed)
        | (SessionStage::Exported, SessionEvent::Failed) => Some(SessionStage::Failed),
        (SessionStage::AwaitingAcceptance, SessionEvent::Cancelled)
        | (SessionStage::Running, SessionEvent::Cancelled)
        | (SessionStage::Exported, SessionEvent::Cancelled) => Some(SessionStage::Cancelled),
        _ => None,
    }
}

/// Export completion is necessary for Done. Advancing events are fenced after
/// a terminal; exact control replays are checked as stutters by the adapter.
pub fn transition(stage: SessionStage, event: SessionEvent) -> (next: Option<SessionStage>)
    ensures next == next_stage(stage, event),
{
    match (stage, event) {
        (SessionStage::AwaitingAcceptance, SessionEvent::Accepted) => Some(SessionStage::Running),
        (SessionStage::Running, SessionEvent::Log) => Some(SessionStage::Running),
        (SessionStage::Running, SessionEvent::Exported) => Some(SessionStage::Exported),
        (SessionStage::Exported, SessionEvent::Done) => Some(SessionStage::Done),
        (SessionStage::AwaitingAcceptance, SessionEvent::Failed)
        | (SessionStage::Running, SessionEvent::Failed)
        | (SessionStage::Exported, SessionEvent::Failed) => Some(SessionStage::Failed),
        (SessionStage::AwaitingAcceptance, SessionEvent::Cancelled)
        | (SessionStage::Running, SessionEvent::Cancelled)
        | (SessionStage::Exported, SessionEvent::Cancelled) => Some(SessionStage::Cancelled),
        _ => None,
    }
}

pub proof fn done_requires_export(stage: SessionStage)
    ensures next_stage(stage, SessionEvent::Done).is_some() ==> stage == SessionStage::Exported,
{}

pub proof fn terminals_are_fenced(stage: SessionStage, event: SessionEvent)
    requires stage == SessionStage::Done || stage == SessionStage::Failed || stage == SessionStage::Cancelled,
    ensures next_stage(stage, event).is_none(),
{}
}
