//! Transaction-bound activation plans and durable per-intent outcomes.
//! Pending authority is separate from diagnostic logs. Legacy records migrate
//! once before replay; known failures remain terminal warnings, not a retry
//! queue or an excuse to roll back a committed generation.
mod action_wire;
mod inspect;
mod ledger;
mod model;
mod outcome;
mod pointer;
mod storage;

pub use inspect::{HookInspection, HookState, inspect};
pub use ledger::{
    ActivationBatch, LaunchPermit, ReadyActivation, has_pending, load_pending, prepare,
};
pub use model::IdentityOrigin;
pub use model::{ActivationId, Contributor, EffectiveIntent, IntentId, IntentOrdinal};
pub use outcome::{AdmissionStage, IntentFailure};

/// An admitted declaration, before one-time effective ordering/coalescing.
/// Apply and rollback include removal declarations from the prior manifest;
/// all saved declarations execute only after the new selection commits.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PendingIntent {
    pub module: String,
    pub action: gripsack_ir::Action,
    #[serde(default)]
    pub trigger: gripsack_ir::Trigger,
}
