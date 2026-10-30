use super::{model::*, normalize::*};
use gripsack_ir::{
    Ir,
    workspace_model::{
        AcquisitionSource, WorkspaceOutput, WorkspaceProducer, graph, lock::WorkspaceLock,
    },
};
use gripsack_policy::graph::roles::GraphRole;
use gripsack_store::{GenerationId, generations};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

/// Captures already admitted declarations only; never evaluates or fetches.
pub fn capture_ir(
    ir: &Ir,
    lock: Option<&WorkspaceLock>,
    origin: SnapshotOrigin,
) -> Result<SemanticSnapshot, SnapshotError> {
    SemanticSnapshot::admit(capture_declarations(ir, lock, origin)?)
}

fn capture_declarations(
    ir: &Ir,
    lock: Option<&WorkspaceLock>,
    origin: SnapshotOrigin,
) -> Result<SnapshotDocument, SnapshotError> {
    if ir.workspace.outputs.len() + ir.workspace.inputs.len() >= MAX_NODES {
        return Err(SnapshotError::Bound("nodes"));
    }
    // The typed direct-call boundary has the same byte/depth budget as imported data.
    super::admission::parse_json(&super::admission::encode_bounded(ir)?)?;
    if let Some(lock) = lock {
        lock.validate()
            .map_err(|_| SnapshotError::Invalid("lock admission"))?;
    }
    let mut document = SnapshotDocument::new(origin);
    let mut workspace = SnapshotNode {
        id: SubjectRef("workspace".into()),
        kind: SubjectKind::Workspace,
        facts: facts(
            serde_json::to_value(&ir.host)?,
            "/host",
            "",
            ChangeKind::Eligibility,
            EvidenceOrigin::Declaration,
        ),
        provenance: vec![provenance(&ir.workspace.span)],
    };
    workspace.facts.push(known(
        "/ir_version",
        ChangeKind::Eligibility,
        &ir.ir_version.to_le_bytes(),
        EvidenceOrigin::Declaration,
    ));
    if let Some(name) = &ir.workspace.name {
        workspace.facts.push(known(
            "/presentation",
            ChangeKind::Provenance,
            name.as_bytes(),
            EvidenceOrigin::Declaration,
        ));
    }
    workspace.facts.push(unknown(
        "/probes",
        ChangeKind::Input,
        UnknownReason::MissingProbe,
    ));
    workspace.facts.push(unknown(
        "/facts",
        ChangeKind::Eligibility,
        UnknownReason::Unobserved,
    ));
    workspace.facts.push(unknown(
        "/source_bundle",
        ChangeKind::Provenance,
        UnknownReason::Unobserved,
    ));
    if let Some(lock) = lock {
        workspace.facts.extend(facts(
            serde_json::to_value(&lock.definitions)?,
            "/definitions",
            "",
            ChangeKind::Input,
            EvidenceOrigin::Lock,
        ));
    } else {
        workspace.facts.push(unknown(
            "/definitions",
            ChangeKind::Input,
            UnknownReason::MissingLock,
        ));
    }
    document.nodes.push(workspace);
    for input in &ir.workspace.inputs {
        let mut node = SnapshotNode {
            id: subject("input", &input.name),
            kind: SubjectKind::Input,
            facts: facts(
                serde_json::to_value(&input.origin)?,
                "/declaration",
                "",
                ChangeKind::Input,
                EvidenceOrigin::Declaration,
            ),
            provenance: vec![provenance(&input.span)],
        };
        node.facts.push(unknown(
            "/captured_bytes",
            ChangeKind::Input,
            UnknownReason::MissingInput,
        ));
        document.nodes.push(node);
    }
    for output in &ir.workspace.outputs {
        let value = serde_json::to_value(output)?;
        let id = subject("output", output.name());
        let mut node = SnapshotNode {
            id: id.clone(),
            kind: output_kind(output.kind()),
            facts: facts(
                value.clone(),
                "/declaration",
                output.kind(),
                ChangeKind::Declaration,
                EvidenceOrigin::Declaration,
            ),
            provenance: declaration_provenance(&value),
        };
        if matches!(output.kind(), "task" | "hook" | "check" | "service") {
            node.facts.push(unknown(
                "/external_state",
                ChangeKind::Observation,
                UnknownReason::MutableExternalState,
            ));
            node.facts.push(unknown(
                "/effects",
                ChangeKind::Invocation,
                UnknownReason::OpaqueEffect,
            ));
        }
        if output.kind() == "schedule" {
            node.facts.push(unknown(
                "/installed_revision",
                ChangeKind::JobRevision,
                UnknownReason::UnavailableReceipt,
            ));
        }
        capture_pins(output, lock, &mut node)?;
        // Captured-input references in commands remain distinct from output references.
        input_edges(&value, &id, &mut document.edges);
        document.edges.push(SnapshotEdge {
            dependency: SubjectRef("workspace".into()),
            consumer: id,
            kind: EdgeKind::Runtime,
        });
        document.nodes.push(node);
    }
    for reference in graph::references(&ir.workspace) {
        let kind = match reference.role {
            GraphRole::Production => EdgeKind::Production,
            GraphRole::BuildInput => EdgeKind::BuildInput,
            GraphRole::Runtime => EdgeKind::Runtime,
            GraphRole::Ordering => EdgeKind::Ordering,
            GraphRole::TaskPrereq => EdgeKind::TaskPrerequisite,
            GraphRole::Validation => EdgeKind::Validation,
            GraphRole::Retention => EdgeKind::Retention,
        };
        document.edges.push(SnapshotEdge {
            dependency: subject("output", reference.to),
            consumer: subject("output", reference.from.name()),
            kind,
        });
    }
    for output in &ir.workspace.outputs {
        for reference in graph::input::output_source_input_references(output)
            .into_iter()
            .flatten()
        {
            document.edges.push(SnapshotEdge {
                dependency: subject("input", reference.to),
                consumer: subject("output", output.name()),
                kind: EdgeKind::BuildInput,
            });
        }
    }
    super::identities::capture(&ir.workspace, lock, &mut document);
    document.edges.sort_unstable();
    document.edges.dedup();
    Ok(document)
}

fn output_kind(kind: &str) -> SubjectKind {
    match kind {
        "package" => SubjectKind::Package,
        "recipe" => SubjectKind::Recipe,
        "environment" => SubjectKind::Environment,
        "task" => SubjectKind::Task,
        "schedule" => SubjectKind::Schedule,
        "check" => SubjectKind::Check,
        "profile" => SubjectKind::Profile,
        "hook" => SubjectKind::Hook,
        "service" => SubjectKind::Service,
        _ => SubjectKind::Output,
    }
}
fn declaration_provenance(value: &Value) -> Vec<Provenance> {
    fn collect(value: &Value, result: &mut Vec<Provenance>) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if key == "span" {
                        if let Ok(span) = serde_json::from_value::<gripsack_ir::Span>(value.clone())
                        {
                            result.push(provenance(&span));
                        }
                    } else {
                        collect(value, result);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    collect(value, result);
                }
            }
            _ => {}
        }
    }
    let mut result = Vec::new();
    collect(value, &mut result);
    result
}
fn input_edges(value: &Value, consumer: &SubjectRef, edges: &mut Vec<SnapshotEdge>) {
    match value {
        Value::Object(fields) => {
            if fields.get("kind").and_then(Value::as_str) == Some("input")
                && let Some(name) = fields.get("input").and_then(Value::as_str)
            {
                edges.push(SnapshotEdge {
                    dependency: subject("input", name),
                    consumer: consumer.clone(),
                    kind: EdgeKind::BuildInput,
                });
            }
            for value in fields.values() {
                input_edges(value, consumer, edges);
            }
        }
        Value::Array(values) => {
            for value in values {
                input_edges(value, consumer, edges);
            }
        }
        _ => {}
    }
}
fn capture_pins(
    output: &WorkspaceOutput,
    lock: Option<&WorkspaceLock>,
    node: &mut SnapshotNode,
) -> Result<(), SnapshotError> {
    if let WorkspaceOutput::Service(service) = output {
        let mut found = false;
        if let Some(lock) = lock {
            for (platform, resolution) in &lock.resolutions {
                for pin in &resolution.pins {
                    if pin.output == service.name {
                        found = true;
                        if !matches!(&pin.source, gripsack_ir::workspace_model::LockedSource::OciImage { reference } if reference == &service.image)
                        {
                            node.facts.push(unknown(
                                &format!("/lock/{}/binding", escaped(platform)),
                                ChangeKind::Eligibility,
                                UnknownReason::Unobserved,
                            ));
                        }
                        let mut value = serde_json::to_value(pin)?;
                        strip_locations(&mut value);
                        node.facts.extend(facts(
                            value,
                            &format!("/lock/{}", escaped(platform)),
                            "service",
                            ChangeKind::Input,
                            EvidenceOrigin::Lock,
                        ));
                    }
                }
            }
        }
        if !found {
            node.facts.push(unknown(
                "/lock",
                ChangeKind::Input,
                UnknownReason::MissingLock,
            ));
        }
        node.facts.push(unknown(
            "/realized_image",
            ChangeKind::SelectedArtifact,
            UnknownReason::UnavailableReceipt,
        ));
        return Ok(());
    }
    let source = match output {
        WorkspaceOutput::Package(package) => match &package.producer {
            WorkspaceProducer::Provider { provider } => Some((provider, &package.target)),
            _ => None,
        },
        WorkspaceOutput::Recipe(recipe) => Some((&recipe.source, &recipe.target)),
        _ => None,
    };
    let Some((source, target)) = source else {
        return Ok(());
    };
    let platform = gripsack_ir::workspace_model::lock::platform_key(target);
    let resolution = lock.and_then(|lock| lock.resolutions.get(&platform));
    let pin = resolution.and_then(|resolution| {
        resolution
            .pins
            .iter()
            .find(|pin| pin.output == output.name())
    });
    if let Some(pin) = pin {
        // Identity fields come from admitted lock records; locations are not semantic pins.
        let mut value = serde_json::to_value(pin)?;
        strip_locations(&mut value);
        node.facts.extend(facts(
            value,
            "/lock",
            "",
            ChangeKind::Input,
            EvidenceOrigin::Lock,
        ));
        if pin.source != source.locked() {
            // Pixi resolved source fields can add acquired input bindings; do not fabricate equality.
            node.facts.push(unknown(
                "/lock_declaration_binding",
                ChangeKind::Eligibility,
                UnknownReason::Unobserved,
            ));
        }
        if let Some(resolution) = resolution {
            // Historical unassigned transitives cannot be attributed to this
            // selected profile. Do not invent a dependency on unrelated pins.
            if resolution.transitive.is_empty() {
                node.facts.push(known(
                    "/transitive_pins",
                    ChangeKind::Input,
                    b"none",
                    EvidenceOrigin::Lock,
                ));
            } else {
                node.facts.push(unknown(
                    "/transitive_pins",
                    ChangeKind::Input,
                    UnknownReason::UnsupportedContract,
                ));
            }
        }
    } else {
        node.facts.push(unknown(
            "/lock",
            ChangeKind::Input,
            UnknownReason::MissingLock,
        ));
    }
    node.facts.push(unknown(
        "/realized_artifact",
        ChangeKind::SelectedArtifact,
        UnknownReason::UnavailableReceipt,
    ));
    Ok(())
}
fn strip_locations(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for key in ["url", "api_url", "span"] {
                fields.remove(key);
            }
            for value in fields.values_mut() {
                strip_locations(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                strip_locations(value);
            }
        }
        _ => {}
    }
}

fn logical_ir<'a>(
    ir: &'a Ir,
    repository: &crate::Repository,
) -> Result<std::borrow::Cow<'a, Ir>, SnapshotError> {
    let crate::Repository::Evaluated { sources, .. } = repository else {
        return Ok(std::borrow::Cow::Borrowed(ir));
    };
    let mut logical = ir.clone();
    crate::rebind_captured_ir(&mut logical, sources)
        .map_err(|_| SnapshotError::Invalid("captured source path rebinding"))?;
    Ok(std::borrow::Cow::Owned(logical))
}

/// Trusted evaluation observations enrich declarations without consulting live state.
pub fn capture_evaluated(
    ir: &Ir,
    repository: &crate::Repository,
    lock: Option<&WorkspaceLock>,
    origin: SnapshotOrigin,
) -> Result<SemanticSnapshot, SnapshotError> {
    let logical = logical_ir(ir, repository)?;
    let ir = logical.as_ref();
    let mut document = capture_declarations(ir, lock, origin)?;
    super::evidence::evaluated(ir, repository, &mut document)?;
    SemanticSnapshot::admit(document)
}

/// The retained contract of one prepared profile, not the union selected by apply.
pub(crate) fn capture_profile(
    ir: &Ir,
    repository: &crate::Repository,
    pins: &crate::workspace::pins::WorkspacePins,
    profile: &str,
    realization: Option<&crate::workspace::Realization<'_>>,
) -> Result<SemanticSnapshot, SnapshotError> {
    let logical = logical_ir(ir, repository)?;
    let ir = logical.as_ref();
    let mut document = capture_declarations(ir, pins.lock.as_ref(), SnapshotOrigin::Recorded)?;
    super::selection::profile(ir, &mut document, profile)?;
    super::evidence::evaluated(ir, repository, &mut document)?;
    super::evidence::admitted_pins(ir, pins, &mut document)?;
    if let Some(workspace) = document
        .nodes
        .iter_mut()
        .find(|node| node.kind == SubjectKind::Workspace)
    {
        workspace.facts.push(known(
            "/lock_presence",
            ChangeKind::Provenance,
            if pins.lock.is_some() {
                b"present"
            } else {
                b"absent"
            },
            EvidenceOrigin::Lock,
        ));
    }
    if let Some(realization) = realization {
        realization.capture_semantic_evidence(&mut document)?;
    }
    SemanticSnapshot::admit(document)
}

pub fn capture_proposed(ir: &Ir, repository: &Path) -> Result<SemanticSnapshot, SnapshotError> {
    let repository = crate::Repository::direct(repository.to_owned());
    capture_repository(ir, &repository)
}

/// Read admitted pin state from the caller's selected repository contents.
/// Trusted evaluation observations remain attached to that repository.
pub fn capture_repository(
    ir: &Ir,
    repository: &crate::Repository,
) -> Result<SemanticSnapshot, SnapshotError> {
    let pins = crate::workspace::pins::WorkspacePins::read(repository)
        .map_err(|_| SnapshotError::Invalid("workspace lock could not be admitted"))?;
    capture_evaluated(ir, repository, pins.lock.as_ref(), SnapshotOrigin::Proposed)
}

/// Enrich explicitly supplied IR using bounded read-only imported-package receipts.
pub fn capture_ir_at(
    home: &Path,
    ir: &Ir,
    lock: Option<&WorkspaceLock>,
    origin: SnapshotOrigin,
) -> Result<SemanticSnapshot, SnapshotError> {
    let mut document = capture_declarations(ir, lock, origin)?;
    for output in &ir.workspace.outputs {
        let WorkspaceOutput::Package(package) = output else {
            continue;
        };
        let WorkspaceProducer::Provider {
            provider: AcquisitionSource::Package { manifest, .. },
        } = &package.producer
        else {
            continue;
        };
        let mut node = SnapshotNode {
            id: subject("distribution", output.name()),
            kind: SubjectKind::Distribution,
            facts: Vec::new(),
            provenance: vec![provenance(&package.span)],
        };
        match crate::workspace::sharing::inspect(home, manifest) {
            Ok(Some(receipt)) => {
                node.facts = facts(
                    serde_json::to_value(receipt)?,
                    "/receipt",
                    "",
                    ChangeKind::SelectedArtifact,
                    EvidenceOrigin::Receipt,
                )
            }
            Ok(None) | Err(_) => node.facts.push(unknown(
                "/receipt",
                ChangeKind::Eligibility,
                UnknownReason::UnavailableReceipt,
            )),
        }
        document.edges.push(SnapshotEdge {
            dependency: node.id.clone(),
            consumer: subject("output", output.name()),
            kind: EdgeKind::Runtime,
        });
        document.nodes.push(node);
    }
    SemanticSnapshot::admit(document)
}

pub fn capture_current(home: &Path) -> Result<SemanticSnapshot, SnapshotError> {
    match generations::current(home)? {
        Some(id) => {
            let mut snapshot = capture_generation(home, id)?;
            snapshot.0.origin = SnapshotOrigin::Current;
            Ok(snapshot)
        }
        None => {
            let mut document = SnapshotDocument::new(SnapshotOrigin::Current);
            document.nodes.push(historical_workspace());
            SemanticSnapshot::admit(document)
        }
    }
}
fn historical_workspace() -> SnapshotNode {
    SnapshotNode {
        id: SubjectRef("workspace".into()),
        kind: SubjectKind::Workspace,
        facts: vec![
            unknown(
                "/declarations",
                ChangeKind::Declaration,
                UnknownReason::HistoricalDeclaration,
            ),
            unknown("/lock", ChangeKind::Input, UnknownReason::MissingLock),
            unknown("/probes", ChangeKind::Input, UnknownReason::MissingProbe),
        ],
        provenance: Vec::new(),
    }
}
/// Read frozen per-profile evidence and legacy deployment receipts only.
/// Missing legacy evidence stays historical; corrupt retained evidence is fatal.
pub fn capture_generation(
    home: &Path,
    id: GenerationId,
) -> Result<SemanticSnapshot, SnapshotError> {
    let generation = generations::read_manifest(home, id)?;
    let mut document = SnapshotDocument::new(SnapshotOrigin::Generation);
    document.generation = Some(id.to_string());
    let mut historical = BTreeSet::new();
    let mut declared_checks = BTreeSet::new();
    for (name, deployment) in &generation.deployments {
        match super::retention::read_profile(home, name, deployment)? {
            Some(snapshot) => {
                if snapshot
                    .document()
                    .nodes
                    .iter()
                    .any(|node| node.kind == SubjectKind::Check)
                {
                    declared_checks.insert(name);
                }
                super::retention::merge(&mut document, snapshot, name);
            }
            None => {
                historical.insert(name);
            }
        }
    }
    if !historical.is_empty() || generation.deployments.is_empty() {
        // Do not overwrite captured workspace facts when only some profiles
        // predate retained evidence. The historical scope is explicit.
        let mut legacy = historical_workspace();
        if document.nodes.iter().any(|node| node.id == legacy.id) {
            legacy.id = SubjectRef("historical/workspace".into());
        }
        document.nodes.push(legacy);
    }
    let mut jobs = BTreeSet::new();
    for (name, deployment) in &generation.deployments {
        let mut value = serde_json::to_value(deployment)?;
        // Generic rendered content hashes can reveal low-entropy secrets. Keep policy,
        // mode and destination evidence but not payload hashes or variable bindings.
        if let Some(fields) = value.as_object_mut() {
            fields.remove("tree256");
            fields.remove("store_path");
            fields.remove("build_closure");
        }
        if let Some(entries) = value.get_mut("entries").and_then(Value::as_array_mut) {
            for entry in entries {
                if let Some(fields) = entry.as_object_mut() {
                    fields.remove("hash");
                    fields.remove("prior");
                }
            }
        }
        let mut node = SnapshotNode {
            id: subject("output", name),
            kind: SubjectKind::Deployment,
            facts: facts(
                value,
                "/deployment",
                "profile",
                ChangeKind::Deployment,
                EvidenceOrigin::Receipt,
            ),
            provenance: Vec::new(),
        };
        if historical.contains(name) {
            node.facts.push(unknown(
                "/declaration",
                ChangeKind::Declaration,
                UnknownReason::HistoricalDeclaration,
            ));
        }
        node.facts.push(unknown(
            "/rendered_content",
            ChangeKind::Deployment,
            UnknownReason::Redacted,
        ));
        if deployment.verified.is_none()
            && (historical.contains(name) || declared_checks.contains(name))
        {
            node.facts.push(unknown(
                "/checks",
                ChangeKind::Validation,
                UnknownReason::UnavailableReceipt,
            ));
        }
        for intent in &deployment.intents {
            if let gripsack_store::activation::ActivationAction::ScheduledJob {
                job,
                revision,
                capture,
                enabled,
                ..
            } = &intent.action
            {
                let job_id = subject("job", &job.to_string());
                if jobs.insert(job_id.clone()) {
                    let mut job_node = SnapshotNode {
                        id: job_id.clone(),
                        kind: SubjectKind::Job,
                        facts: vec![
                            known(
                                "/revision",
                                ChangeKind::JobRevision,
                                revision.to_string().as_bytes(),
                                EvidenceOrigin::Receipt,
                            ),
                            known(
                                "/enabled",
                                ChangeKind::Registration,
                                &[*enabled as u8],
                                EvidenceOrigin::Receipt,
                            ),
                            unknown(
                                "/registration_observation",
                                ChangeKind::Observation,
                                UnknownReason::Unobserved,
                            ),
                            unknown(
                                "/last_run",
                                ChangeKind::Observation,
                                UnknownReason::Unobserved,
                            ),
                        ],
                        provenance: Vec::new(),
                    };
                    super::jobs::capture(
                        home,
                        capture,
                        job,
                        revision,
                        &mut job_node,
                        &mut document,
                    )?;
                    document.nodes.push(job_node);
                }
                document.edges.push(SnapshotEdge {
                    dependency: job_id,
                    consumer: node.id.clone(),
                    kind: EdgeKind::Runtime,
                });
            }
        }
        if let Some(captured) = document
            .nodes
            .iter_mut()
            .find(|captured| captured.id == node.id)
        {
            captured.facts.extend(node.facts);
        } else {
            document.nodes.push(node);
        }
    }
    document.edges.sort_unstable();
    document.edges.dedup();
    SemanticSnapshot::admit(document)
}
