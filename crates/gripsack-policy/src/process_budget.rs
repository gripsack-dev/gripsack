//! Shipped process protocol arithmetic and transition decisions.
//! IO observations, clocks, allocation, signals and reaping stay in the supervisor.
mod frame;
mod input;
mod lifecycle;
mod output;
mod tail;
mod transfer;
pub use frame::{FrameAction, FrameBudget, FrameByteLimit};
pub use input::{InputAppend, InputByteLimit, admit_input_append};
pub use lifecycle::{
    ChildLifecycle, CleanupDecision, ObserveAction, ReapObservation, SignalAction,
};
pub use output::{
    RetainedStderrLimit, StderrBudget, StderrByteLimit, StdoutBudget, StdoutByteLimit,
};
pub use tail::{TailAppend, retain_tail};
pub use transfer::{IO_CHUNK_BYTES, InputChunk, InputTransfer};
