//! Frozen job declarations enter the same graph, not a schedule-specific analyzer.
use super::{
    capture::capture_ir,
    model::*,
    normalize::{facts, unknown},
};
use gripsack_store::jobs::{JobId, JobRevision};
use std::path::Path;

pub(super) fn capture(
    home: &Path,
    path: &Path,
    job: &JobId,
    revision: &JobRevision,
    node: &mut SnapshotNode,
    document: &mut SnapshotDocument,
) -> Result<(), SnapshotError> {
    let mut recorded = match crate::workspace::schedules::inspect_capture(home, path, job, revision)
    {
        Ok(recorded) => recorded,
        Err(_) => {
            node.facts.push(unknown(
                "/capture",
                ChangeKind::JobRevision,
                UnknownReason::UnavailableReceipt,
            ));
            return Ok(());
        }
    };
    let ir = recorded
        .get_mut("ir")
        .map(serde_json::Value::take)
        .ok_or(SnapshotError::Invalid("job capture has no declaration"))?;
    let ir: gripsack_ir::Ir = serde_json::from_value(ir)?;
    let captured = capture_ir(&ir, None, SnapshotOrigin::Recorded)?.into_document();
    if document.nodes.len() + captured.nodes.len() + 1 > MAX_NODES {
        return Err(SnapshotError::Bound("nodes"));
    }
    if document.edges.len() + captured.edges.len() + 1 > MAX_EDGES {
        return Err(SnapshotError::Bound("edges"));
    }
    let count: usize = document
        .nodes
        .iter()
        .chain(&captured.nodes)
        .map(|node| node.facts.len())
        .sum();
    if count + node.facts.len() > MAX_FACTS {
        return Err(SnapshotError::Bound("facts"));
    }
    let prefix = format!("{}/capture/", node.id.0);
    let selected_task = recorded
        .get("task")
        .and_then(serde_json::Value::as_str)
        .ok_or(SnapshotError::Invalid("job capture has no selected task"))?;
    let selected_id = super::normalize::subject("output", selected_task);
    for mut captured_node in captured.nodes {
        if captured_node.id == selected_id {
            document.edges.push(SnapshotEdge {
                dependency: SubjectRef(format!("{prefix}{}", captured_node.id.0)),
                consumer: node.id.clone(),
                kind: EdgeKind::Runtime,
            });
        }
        captured_node.id.0.insert_str(0, &prefix);
        document.nodes.push(captured_node);
    }
    for mut edge in captured.edges {
        edge.dependency.0.insert_str(0, &prefix);
        edge.consumer.0.insert_str(0, &prefix);
        document.edges.push(edge);
    }
    // Immutable native package/program evidence is separate from mutable cwd,
    // host environment and last-run observations. Do not export raw root/file hashes.
    for field in [
        "programs",
        "packages",
        "receipts",
        "trigger",
        "unit_directory",
        "task",
    ] {
        if let Some(value) = recorded.get_mut(field) {
            node.facts.extend(facts(
                value.take(),
                &format!("/capture/{field}"),
                "",
                if field == "trigger" || field == "unit_directory" {
                    ChangeKind::Registration
                } else {
                    ChangeKind::JobRevision
                },
                EvidenceOrigin::Receipt,
            ));
        }
    }
    Ok(())
}
