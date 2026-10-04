//! Two bounded bridge invocations: pure lowering, then execution of checked
//! bytes. Uses the common admitted native-process supervisor, not a second
//! blocking child/pipe implementation. EOF and process exit never imply Done.
#[cfg(test)]
mod tests;
use crate::{
    identity::{AttemptIdentity, DefinitionDigest, LlbVertexDigest},
    llb::{CheckError, CheckedDefinition},
    plan::{NodeIndex, ValidatedBuildPlan},
    protocol::{self, FromBridge, LowerRequest, ProtocolError},
};
use gripsack_policy::buildkit::{SessionEvent, SessionStage, transition};
use gripsack_process::{
    Control, InputByteLimit, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessReceipt,
    ProcessRole, SelectedProgram,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("bridge admission failed: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Check(#[from] CheckError),
    #[error("bridge process failed or cleanup is unconfirmed")]
    Process {
        receipt: Box<ProcessReceipt>,
        stderr: Vec<u8>,
    },
    #[error("bridge rejected the attempt ({code:?}): {message:?}")]
    Rejected {
        code: protocol::FailureCode,
        message: String,
        vertices: Vec<LlbVertexDigest>,
        nodes: Vec<NodeIndex>,
    },
    #[error("bridge attempt was cancelled; no publication is authorized")]
    Cancelled,
    #[error("bridge ended without its required terminal result")]
    NoTerminal,
}

/// A completed export is not an admitted package. The realization owner must
/// still validate payload, runtime closure, receipts and retained roots.
pub struct CompletedExport {
    definition: DefinitionDigest,
    destination: PathBuf,
    process: ProcessReceipt,
    identity: AttemptIdentity,
    worker: protocol::WorkerBinding,
    scope: crate::worker::WorkerScope,
}
impl CompletedExport {
    pub fn definition(&self) -> DefinitionDigest {
        self.definition
    }
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    pub fn process(&self) -> &ProcessReceipt {
        &self.process
    }
    pub fn identity(&self) -> &AttemptIdentity {
        &self.identity
    }
    pub fn worker(&self) -> protocol::WorkerBinding {
        self.worker
    }
    pub fn scope(&self) -> crate::worker::WorkerScope {
        self.scope
    }
}

pub struct ExportPaths<'a> {
    pub worker: &'a crate::worker::WorkerLease,
    pub retention: Option<&'a std::fs::File>,
    pub inputs: &'a Path,
    pub destination: &'a Path,
}

/// Operator-selected pinned executable and captured environment outlive both
/// calls. The absolute operation deadline is never reset between phases.
pub struct Bridge<'a> {
    environment: &'a OperatorEnvironment,
    program: &'a SelectedProgram,
    directory: &'a Path,
    deadline: Instant,
}
impl<'a> Bridge<'a> {
    pub fn new(
        environment: &'a OperatorEnvironment,
        program: &'a SelectedProgram,
        directory: &'a Path,
        deadline: Instant,
    ) -> Self {
        Self {
            environment,
            program,
            directory,
            deadline,
        }
    }

    pub fn lower(
        &self,
        plan: &ValidatedBuildPlan,
        identity: &AttemptIdentity,
        retention: Option<&std::fs::File>,
    ) -> Result<CheckedDefinition, TransportError> {
        let request = LowerRequest {
            protocol_version: protocol::PROTOCOL_VERSION,
            session: identity.session.clone(),
            attempt: identity.attempt,
            epoch: identity.epoch,
            plan: plan.plan(),
        };
        let leases = retention
            .map(|handle| {
                handle
                    .try_clone()
                    .map(|retention| gripsack_process::ProcessLeases {
                        worker: None,
                        retention: Some(retention),
                    })
            })
            .transpose()?;
        let mut result: Option<Result<CheckedDefinition, TransportError>> = None;
        self.invoke(&[OsStr::new("lower")], &request, leases, |event| {
            if !event.matches(identity) || event.worker_binding().is_some() {
                return Err(ProtocolError::Identity);
            }
            if matches!(&event, FromBridge::Failed { vertices, .. } if !vertices.is_empty()) {
                return Err(ProtocolError::Transition);
            }
            if let Some(previous) = &result {
                let replay = match (previous, &event) {
                    (Ok(checked), FromBridge::Prepared { lowered, .. }) => checked.repeats(lowered),
                    (
                        Err(TransportError::Rejected {
                            code,
                            message,
                            vertices,
                            ..
                        }),
                        FromBridge::Failed {
                            code: next_code,
                            message: next_message,
                            vertices: next_vertices,
                            ..
                        },
                    ) => code == next_code && message == next_message && vertices == next_vertices,
                    _ => false,
                };
                return if replay {
                    Ok(())
                } else {
                    Err(ProtocolError::Transition)
                };
            }
            result = Some(match event {
                FromBridge::Prepared { lowered, .. } => {
                    CheckedDefinition::validate(plan, lowered).map_err(TransportError::Check)
                }
                FromBridge::Failed {
                    code,
                    message,
                    vertices,
                    ..
                } => Err(TransportError::Rejected {
                    code,
                    message,
                    vertices,
                    nodes: Vec::new(),
                }),
                _ => return Err(ProtocolError::Transition),
            });
            Ok(())
        })?;
        result.ok_or(TransportError::NoTerminal)?
    }

    pub fn execute(
        &self,
        definition: CheckedDefinition,
        identity: &AttemptIdentity,
        paths: ExportPaths<'_>,
        mut log: impl FnMut(&[NodeIndex], &str, &[u8], bool),
    ) -> Result<CompletedExport, TransportError> {
        // Paths are core-selected capabilities, never values returned by lowering.
        if !paths.inputs.is_absolute() || !paths.destination.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "bridge requires explicit private absolute paths and Unix socket",
            )
            .into());
        }
        let address = paths.worker.address().connect_arg();
        let leases = gripsack_process::ProcessLeases {
            worker: Some(paths.worker.duplicate_handle()?),
            retention: paths.retention.map(std::fs::File::try_clone).transpose()?,
        };
        let digest = definition.digest();
        let worker = protocol::WorkerBinding {
            instance: crate::identity::WorkerInstanceId::parse(paths.worker.instance_id())?,
            epoch: paths.worker.epoch(),
        };
        let (request, witness) = definition.into_request(identity, worker);
        let mut node_bindings: BTreeMap<String, Vec<NodeIndex>> = BTreeMap::new();
        for binding in witness {
            node_bindings
                .entry(String::from(binding.vertex))
                .or_default()
                .push(binding.node);
        }
        let now = Instant::now();
        let process_end = gripsack_process::execution_deadline(now, self.deadline);
        let solve_end = gripsack_process::execution_deadline(now, process_end);
        let remaining = solve_end.saturating_duration_since(now);
        let timeout_ms = u64::try_from(remaining.as_millis()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "bridge deadline cannot be represented",
            )
        })?;
        let timeout = OsString::from(timeout_ms.to_string());
        let args = [
            OsStr::new("execute"),
            OsStr::new("--address"),
            address.as_os_str(),
            OsStr::new("--inputs"),
            paths.inputs.as_os_str(),
            OsStr::new("--output"),
            paths.destination.as_os_str(),
            OsStr::new("--timeout-ms"),
            timeout.as_os_str(),
        ];
        let mut stage = SessionStage::AwaitingAcceptance;
        let mut failure: Option<TransportError> = None;
        let mut accepted = false;
        let mut exported = false;
        let mut log_count = 0;
        let receipt = self.invoke(&args, &request, Some(leases), |event| {
            if !event.matches(identity) || event.worker_binding() != Some(worker) { return Err(ProtocolError::Identity); }
            let kind = match &event {
                FromBridge::Accepted { daemon_version, platform, .. } => {
                    if daemon_version != protocol::EXPECTED_DAEMON_VERSION || platform != &request.platform {
                        return Err(ProtocolError::Capability);
                    }
                    SessionEvent::Accepted
                },
                FromBridge::Event { .. } => SessionEvent::Log,
                FromBridge::Exported { .. } => SessionEvent::Exported,
                FromBridge::Done { .. } => SessionEvent::Done,
                FromBridge::Failed { .. } => SessionEvent::Failed,
                FromBridge::Cancelled { .. } => SessionEvent::Cancelled,
                FromBridge::Prepared { .. } => return Err(ProtocolError::Transition),
            };
            // Identical control replies stutter, including late duplicates.
            // Identity/capability checks precede this path; a changed terminal
            // still reaches the transition fence and cannot replace its result.
            let replay = match &event {
                FromBridge::Accepted { .. } => accepted,
                FromBridge::Exported { .. } => exported,
                FromBridge::Done { .. } => stage == SessionStage::Done,
                FromBridge::Cancelled { .. } => stage == SessionStage::Cancelled,
                FromBridge::Failed { code, message, vertices, .. } => matches!(&failure,
                    Some(TransportError::Rejected { code: previous_code, message: previous_message, vertices: previous_vertices, .. })
                    if code == previous_code && message == previous_message && vertices == previous_vertices),
                FromBridge::Event { .. } | FromBridge::Prepared { .. } => false,
            };
            if replay { return Ok(()); }
            stage = transition(stage, kind).ok_or(ProtocolError::Transition)?;
            match event {
                FromBridge::Accepted { .. } => accepted = true,
                FromBridge::Exported { .. } => exported = true,
                FromBridge::Event { vertex, chunk, truncated, .. } => {
                    log_count += 1;
                    if log_count > protocol::MAX_LOG_EVENTS { return Err(ProtocolError::EventBudget); }
                    log(node_bindings.get(&vertex).map_or(&[], Vec::as_slice), &vertex, &chunk, truncated);
                },
                FromBridge::Failed { code, message, vertices, .. } => failure = Some(TransportError::Rejected { code, message, vertices, nodes: Vec::new() }),
                _ => {},
            }
            Ok(())
        })?;
        if let Some(TransportError::Rejected {
            vertices, nodes, ..
        }) = &mut failure
        {
            for vertex in vertices {
                if let Some(bound) = node_bindings.get(vertex.as_str()) {
                    nodes.extend_from_slice(bound);
                }
            }
            nodes.sort_unstable_by_key(|node| node.index());
            nodes.dedup();
        }
        if let Some(error) = failure {
            return Err(error);
        }
        match stage {
            SessionStage::Done => Ok(CompletedExport {
                definition: digest,
                destination: paths.destination.to_owned(),
                process: receipt,
                identity: identity.clone(),
                worker,
                scope: paths.worker.scope(),
            }),
            SessionStage::Cancelled => Err(TransportError::Cancelled),
            _ => Err(TransportError::NoTerminal),
        }
    }

    fn invoke(
        &self,
        arguments: &[&OsStr],
        request: &impl Serialize,
        leases: Option<gripsack_process::ProcessLeases>,
        mut receive: impl FnMut(FromBridge) -> Result<(), ProtocolError>,
    ) -> Result<ProcessReceipt, TransportError> {
        let request = protocol::encode_frame(request)?;
        let limits = Limits {
            timeout: self.deadline.saturating_duration_since(Instant::now()),
            operation_deadline: Some(self.deadline),
            input_bytes: InputByteLimit::new(
                protocol::MAX_FRAME_BYTES + std::mem::size_of::<u64>(),
            ),
            ..Limits::default()
        };
        let invocation = Invocation::admit(
            self.environment,
            ProcessRole::Build,
            self.program,
            self.directory,
            limits,
        )?;
        let invocation = match leases {
            Some(leases) => invocation.retain_leases(leases)?,
            None => invocation,
        };
        let mut decoder = protocol::FrameDecoder::default();
        let mut protocol_error = None;
        let mut events = 0;
        let outcome = invocation.run(arguments, NativeInput::Bytes(&request), None, |bytes| {
            let result = decoder.push(bytes, |event| {
                events += 1;
                if events > protocol::MAX_EVENT_COUNT {
                    return Err(ProtocolError::EventBudget);
                }
                receive(event)
            });
            match result {
                Ok(()) => Control::Continue,
                Err(error) => {
                    protocol_error = Some(error);
                    Control::Response
                }
            }
        })?;
        if let Some(error) = protocol_error {
            return Err(error.into());
        }
        decoder.finish()?;
        if !outcome.success {
            return Err(TransportError::Process {
                receipt: Box::new(outcome.receipt),
                stderr: outcome.stderr,
            });
        }
        Ok(outcome.receipt)
    }
}
