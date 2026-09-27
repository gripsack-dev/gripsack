//! Durable hook-attempt decisions. Effects remain behind the store's private
//! persist-before-launch permit. Interrupted attempts advance under the same
//! intent identity; durable success/failure/supersession never retries.
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct AttemptNumber { number: u64 }

impl AttemptNumber {
    #[verifier::type_invariant]
    spec fn positive(&self) -> bool { self.number > 0 }

    pub closed spec fn view(self) -> u64 { self.number }

    pub fn admit(number: u64) -> (attempt: Option<Self>)
        ensures
            attempt.is_some() <==> number > 0,
            attempt.is_some() ==> attempt.unwrap().view() == number,
    {
        if number == 0 { None } else { Some(Self { number }) }
    }

    pub fn first() -> (attempt: Self)
        ensures attempt.view() == 1,
    {
        Self { number: 1 }
    }

    pub fn value(self) -> (number: u64)
        ensures number == self.view(), number > 0,
    {
        proof { use_type_invariant(&self); }
        self.number
    }

    pub fn checked_next(self) -> (next: Option<Self>)
        ensures
            next.is_some() <==> self.view() < u64::MAX,
            next.is_some() ==> next.unwrap().view() == self.view() + 1,
    {
        if self.number == u64::MAX { None }
        else { Some(Self { number: self.number + 1 }) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum Outcome { Succeeded, Failed }

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum IntentState {
    Pending,
    Started { attempt: AttemptNumber },
    Succeeded { attempt: AttemptNumber },
    Failed { attempt: AttemptNumber },
    Superseded { last_attempt: Option<AttemptNumber> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum StartDecision {
    Start(AttemptNumber),
    Terminal,
    Exhausted,
}

pub open spec fn last_attempt(state: IntentState) -> Option<AttemptNumber> {
    match state {
        IntentState::Pending => None,
        IntentState::Started { attempt } => Some(attempt),
        IntentState::Succeeded { attempt } => Some(attempt),
        IntentState::Failed { attempt } => Some(attempt),
        IntentState::Superseded { last_attempt } => last_attempt,
    }
}

pub open spec fn terminal(state: IntentState) -> bool {
    match state {
        IntentState::Succeeded { .. } | IntentState::Failed { .. }
            | IntentState::Superseded { .. } => true,
        _ => false,
    }
}

impl IntentState {
    pub fn attempt(&self) -> (attempt: Option<AttemptNumber>)
        ensures attempt == last_attempt(*self),
    {
        match self {
            Self::Pending => None,
            Self::Started { attempt } | Self::Succeeded { attempt }
                | Self::Failed { attempt } => Some(*attempt),
            Self::Superseded { last_attempt } => *last_attempt,
        }
    }
}

/// Called before writing the next Started state. The returned number is not
/// effect authority until that write's required durability barriers succeed.
pub fn next_attempt(state: &IntentState) -> (decision: StartDecision)
    ensures
        (decision == StartDecision::Terminal) <==> terminal(*state),
        (decision == StartDecision::Exhausted) <==> match *state {
            IntentState::Started { attempt } => attempt.view() == u64::MAX,
            _ => false,
        },
        match decision {
            StartDecision::Start(next) => match *state {
                IntentState::Pending => next.view() == 1,
                IntentState::Started { attempt } =>
                    attempt.view() < u64::MAX && next.view() == attempt.view() + 1,
                _ => false,
            },
            _ => true,
        },
{
    match state {
        IntentState::Pending => StartDecision::Start(AttemptNumber::first()),
        IntentState::Started { attempt } => match attempt.checked_next() {
            Some(next) => StartDecision::Start(next),
            None => StartDecision::Exhausted,
        },
        IntentState::Succeeded { .. } | IntentState::Failed { .. }
            | IntentState::Superseded { .. } => StartDecision::Terminal,
    }
}

/// Only the matching in-flight attempt can acquire a terminal result. A late
/// outcome cannot replace a later attempt or overwrite a terminal disposition.
pub fn finish_attempt(state: &IntentState, attempt: AttemptNumber, outcome: Outcome) -> (next: Option<IntentState>)
    ensures
        next.is_some() <==> *state == (IntentState::Started { attempt }),
        next.is_some() ==> next.unwrap() == match outcome {
            Outcome::Succeeded => IntentState::Succeeded { attempt },
            Outcome::Failed => IntentState::Failed { attempt },
        },
{
    match state {
        IntentState::Started { attempt: active } if *active == attempt => Some(match outcome {
            Outcome::Succeeded => IntentState::Succeeded { attempt },
            Outcome::Failed => IntentState::Failed { attempt },
        }),
        _ => None,
    }
}

pub fn supersede(state: &IntentState) -> (next: IntentState)
    ensures
        last_attempt(next) == last_attempt(*state),
        terminal(next),
        terminal(*state) ==> next == *state,
        !terminal(*state) ==> next == (IntentState::Superseded { last_attempt: last_attempt(*state) }),
{
    match state {
        IntentState::Pending => IntentState::Superseded { last_attempt: None },
        IntentState::Started { attempt } => IntentState::Superseded { last_attempt: Some(*attempt) },
        _ => *state,
    }
}

}
