//! Advisory Conda response admission, not publication authority. The adapter
//! supplies the supervised terminal outcome and borrowed decoded fields. Frame
//! version/length/EOF and native lifecycle observations are outside this model;
//! archive, receipt and installed-tree truth require independent core validation.
use vstd::prelude::*;

verus! {
/// A reaped normal exit retains its actual code; all other native outcomes
/// (signal, deadline, output limit, cleanup failure) are interrupted terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTerminal { Exited(i32), Interrupted }

/// Materialization cannot be confused with a solve or import. Its digest binds
/// the complete frozen closure INCLUDING materializer policy, not package names.
#[derive(Debug, Clone, Copy)]
pub enum OperationContext<'a> {
    Resolve,
    ImportPixi,
    Materialize { closure_digest: &'a str, final_prefix: &'a str },
}

/// Views borrow the original request/response; admission never copies fields.
#[derive(Debug, Clone, Copy)]
pub struct ResponseBinding<'a> {
    pub attempt: u64,
    pub platform: &'a str,
    pub context: OperationContext<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionRefusal { NativeTerminal, Attempt, Platform, Operation, Closure, FinalPrefix }

pub open spec fn same_context(expected: OperationContext, actual: OperationContext) -> bool {
    match (expected, actual) {
        (OperationContext::Resolve, OperationContext::Resolve) => true,
        (OperationContext::ImportPixi, OperationContext::ImportPixi) => true,
        (OperationContext::Materialize { closure_digest: ed, final_prefix: ep },
         OperationContext::Materialize { closure_digest: ad, final_prefix: ap }) =>
            ed@ == ad@ && ep@ == ap@,
        _ => false,
    }
}

pub open spec fn admissible(terminal: NativeTerminal, expected: ResponseBinding, actual: ResponseBinding) -> bool {
    terminal == NativeTerminal::Exited(0)
        && expected.attempt == actual.attempt
        && expected.platform@ == actual.platform@
        && same_context(expected.context, actual.context)
}

/// Used before decoding too: a valid frame cannot erase a failed terminal.
pub fn terminal_succeeded(terminal: NativeTerminal) -> (success: bool)
    ensures success == (terminal == NativeTerminal::Exited(0)),
{
    matches!(terminal, NativeTerminal::Exited(0))
}

/// Total exact-binding contract over arbitrary decoded byte strings. No helper
/// success flag or precondition assumes the conclusion. Refusal never grants
/// publication permission; Ok admits only advisory data for independent checks.
pub fn admit_response(terminal: NativeTerminal, expected: ResponseBinding<'_>, actual: ResponseBinding<'_>)
    -> (decision: Result<(), AdmissionRefusal>)
    ensures decision.is_ok() == admissible(terminal, expected, actual),
{
    if !terminal_succeeded(terminal) {
        return Err(AdmissionRefusal::NativeTerminal);
    }
    if expected.attempt != actual.attempt {
        return Err(AdmissionRefusal::Attempt);
    }
    if expected.platform != actual.platform {
        return Err(AdmissionRefusal::Platform);
    }
    match (expected.context, actual.context) {
        (OperationContext::Resolve, OperationContext::Resolve) => Ok(()),
        (OperationContext::ImportPixi, OperationContext::ImportPixi) => Ok(()),
        (OperationContext::Materialize { closure_digest: ed, final_prefix: ep },
         OperationContext::Materialize { closure_digest: ad, final_prefix: ap }) => {
            if ed != ad {
                Err(AdmissionRefusal::Closure)
            } else if ep != ap {
                Err(AdmissionRefusal::FinalPrefix)
            } else {
                Ok(())
            }
        },
        _ => Err(AdmissionRefusal::Operation),
    }
}
}
