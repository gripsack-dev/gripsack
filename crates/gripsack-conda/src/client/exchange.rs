//! One protocol transaction through the common native lifecycle. No inherited
//! stderr, unbounded blocking pipe reads, detached child, or background reaper.
use super::{CondaError, CondaHelper};
use crate::protocol::{self, Request, Response};
use gripsack_policy::conda_admission::{self, NativeTerminal, OperationContext, ResponseBinding};
use gripsack_process::{
    Control, InputByteLimit, Invocation, Limits, NativeInput, ProcessDisposition, ProcessRole,
    StdoutByteLimit,
};

impl CondaHelper {
    pub(super) fn exchange(&self, request: &Request<'_>) -> Result<Response, CondaError> {
        let input = protocol::encode_request(request)?;
        let limits = Limits {
            operation_deadline: Some(self.deadline),
            input_bytes: InputByteLimit::new(input.len()),
            stdout_bytes: StdoutByteLimit::new(
                protocol::MAX_RESPONSE_BYTES + protocol::FRAME_HEADER_BYTES as u64,
            ),
            ..Limits::default()
        };
        let invocation = Invocation::admit(
            &self.environment,
            ProcessRole::Plugin,
            &self.program,
            &self.directory,
            limits,
        )
        .map_err(CondaError::Native)?;
        let mut output = Vec::new();
        let outcome = invocation
            .run(&[], NativeInput::Bytes(&input), None, |bytes| {
                output.extend_from_slice(bytes);
                Control::Continue
            })
            .map_err(CondaError::Native)?;
        // Project the supervisor's receipt, never a helper-controlled success
        // field. Exited means the child was reaped and streams drained.
        let terminal = match (
            &outcome.receipt.disposition,
            outcome.receipt.exit_code,
            outcome.receipt.signal,
        ) {
            (ProcessDisposition::Exited, Some(code), None) => NativeTerminal::Exited(code),
            _ => NativeTerminal::Interrupted,
        };
        if !conda_admission::terminal_succeeded(terminal) {
            return Err(CondaError::Process {
                receipt: Box::new(outcome.receipt),
                stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
            });
        }
        let response = protocol::decode_response_frame(&output)?;
        if let Some(actual) = response_binding(&response) {
            conda_admission::admit_response(terminal, request_binding(request), actual).map_err(
                |refusal| CondaError::Echo(format!("response binding refused: {refusal:?}")),
            )?;
        }
        Ok(response)
    }
}

fn request_binding<'a>(request: &'a Request<'_>) -> ResponseBinding<'a> {
    let (platform, context) = match request {
        Request::Resolve(sent) => (sent.platform.as_str(), OperationContext::Resolve),
        Request::ImportPixi(sent) => (sent.platform.as_str(), OperationContext::ImportPixi),
        Request::Materialize(sent) => (
            sent.closure.platform.as_str(),
            OperationContext::Materialize {
                closure_digest: &sent.lock_digest,
                final_prefix: &sent.final_prefix,
            },
        ),
    };
    ResponseBinding {
        attempt: request.attempt(),
        platform,
        context,
    }
}

fn response_binding(response: &Response) -> Option<ResponseBinding<'_>> {
    let (attempt, platform, context) = match response {
        Response::Resolved(received) => (
            received.attempt,
            received.environment.platform.as_str(),
            OperationContext::Resolve,
        ),
        Response::Imported(received) => (
            received.attempt,
            received.environment.platform.as_str(),
            OperationContext::ImportPixi,
        ),
        Response::Materialized(received) => (
            received.attempt,
            received.platform.as_str(),
            OperationContext::Materialize {
                closure_digest: &received.lock_digest,
                final_prefix: &received.final_prefix,
            },
        ),
        // Errors are never advisory success; callers retain typed helper errors
        // and separately reject stale optional attempt echoes.
        Response::Error(_) => return None,
    };
    Some(ResponseBinding {
        attempt,
        platform,
        context,
    })
}
