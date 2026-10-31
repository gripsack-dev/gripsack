//! Native provider acquisition and retained package publication.
//! Executable recipes and OCI image authoring have no realization backend.
use super::{
    artifact::{self, Package, ProducerIdentity},
    prepare::Prepared,
    roots::{self, RetentionSet, RootId},
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_ir::{
    Ir,
    workspace_model::{WorkspaceOutput, WorkspaceProducer},
};
use gripsack_process::OperatorEnvironment;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

pub struct BuildOptions<'a> {
    pub environment: &'a OperatorEnvironment,
    pub deadline: Instant,
}
#[derive(Serialize)]
pub struct BuiltOutput {
    pub name: String,
    pub kind: &'static str,
    pub path: PathBuf,
    pub commands: BTreeMap<String, PathBuf>,
}
#[derive(Serialize)]
pub struct BuildResult {
    pub outputs: Vec<BuiltOutput>,
}

pub(crate) struct Realization<'a> {
    pub(super) packages: BTreeMap<&'a str, Arc<Package>>,
    /// Captured workspace input binding paths (file inputs carry their
    /// `value` member); empty when the selection declares no inputs.
    pub(super) inputs: BTreeMap<&'a str, PathBuf>,
}

impl Realization<'_> {
    /// Project only selected, already-admitted native receipts; never reopen a
    /// store object or include home paths, attempts, or mutable observations.
    pub(crate) fn capture_semantic_evidence(
        &self,
        document: &mut crate::semantic::SnapshotDocument,
    ) -> Result<(), crate::semantic::SnapshotError> {
        use crate::semantic::evidence::{record_identity, record_value};
        use crate::semantic::{ChangeKind, EvidenceOrigin};
        use gripsack_ir::workspace_model::identity::DefinitionDigest;
        for (name, package) in &self.packages {
            let id = crate::semantic::evidence::output_subject(name);
            let Some(node) = document.nodes.iter_mut().find(|node| node.id == id) else {
                continue;
            };
            // A redacted declaration must not regain its secret/config digest
            // indirectly through a producer or package identity.
            if crate::semantic::evidence::is_redacted(node) {
                continue;
            }
            record_identity(
                node,
                "/package_identity",
                ChangeKind::Materialization,
                DefinitionDigest::from_bytes(*package.identity.bytes()),
                EvidenceOrigin::Receipt,
            );
            record_value(
                node,
                "/realized_artifact",
                ChangeKind::SelectedArtifact,
                serde_json::json!({
                    "tree": package.producer.tree,
                    "commands": package.commands,
                    "target": package.target,
                    "layout": package.layout,
                    "archive_root": package.archive_root,
                    "selected_root": package.selected_root,
                    "host_runtime": package.host_runtime,
                    "conda": package.conda.as_ref().map(|receipt| serde_json::json!({
                        "platform": receipt.platform,
                        "materialization": receipt.materialization,
                        "closure": receipt.closure,
                        "system": receipt.system,
                    })),
                }),
                EvidenceOrigin::Receipt,
            );
        }
        Ok(())
    }
}

pub fn build_workspace(
    ir: &Ir,
    ctx: &Ctx,
    options: &BuildOptions<'_>,
) -> Result<BuildResult, ExecError> {
    let workspace = &ir.workspace;
    let selection = super::selection::Selection::admit(workspace, &ctx.only)?;
    for name in &ctx.only {
        if !matches!(
            selection.outputs[name.as_str()],
            WorkspaceOutput::Package(_)
        ) {
            return Err(failure(format!("{name:?} is not a direct package output")));
        }
    }
    let HeldRealization {
        realization: realized,
        session,
        closures,
    } = realize_held(ir, ctx, options, &ctx.only)?;
    for (name, paths) in closures {
        let id = RootId::from_identity(&serde_json::to_vec(&(ctx.repository.identity(), &name))?);
        let consumer = RetentionSet::admit(&session, paths)?;
        roots::register_output_root(&session, &id, &consumer)?;
    }
    let mut outputs = Vec::with_capacity(ctx.only.len());
    for name in &ctx.only {
        let output = match selection.outputs[name.as_str()] {
            WorkspaceOutput::Package(_) => {
                let package = &realized.packages[name.as_str()];
                BuiltOutput {
                    name: name.clone(),
                    kind: "package",
                    path: package.payload().to_path_buf(),
                    commands: package
                        .commands
                        .keys()
                        .map(|command| {
                            (
                                command.clone(),
                                package.command(command).expect("provided command"),
                            )
                        })
                        .collect(),
                }
            }
            _ => unreachable!("build consumer admitted"),
        };
        outputs.push(output);
    }
    Ok(BuildResult { outputs })
}

fn input_paths<'a>(prepared: &Prepared<'a>) -> BTreeMap<&'a str, PathBuf> {
    prepared
        .inputs
        .iter()
        .map(|(name, input)| (*name, input.binding_path()))
        .collect()
}

/// A realization that still holds its lifecycle session. Callers register
/// their own consumer roots (project selections, process leases, generation
/// closures) atomically with the realization — there is no GC window between
/// publication and root acquisition. `build_workspace` keeps per-output
/// roots; native consumers register project/process roots instead.
pub(super) struct HeldRealization<'a> {
    pub realization: Realization<'a>,
    pub session: LifecycleSession,
    /// Exact realized closure per selected output.
    pub closures: Vec<(String, BTreeSet<PathBuf>)>,
}

pub(super) fn realize_held<'a>(
    ir: &'a Ir,
    ctx: &Ctx,
    _options: &BuildOptions<'_>,
    selected: &[String],
) -> Result<HeldRealization<'a>, ExecError> {
    let session = LifecycleSession::acquire(&ctx.home)?;
    realize_in_session(ir, ctx, _options, selected, session)
}

pub(super) fn realize_in_session<'a>(
    ir: &'a Ir,
    ctx: &Ctx,
    _options: &BuildOptions<'_>,
    selected: &[String],
    session: LifecycleSession,
) -> Result<HeldRealization<'a>, ExecError> {
    let prepared = Prepared::frozen(ir, ctx, &session, selected)?;
    let mut retention = BTreeSet::new();
    for source in prepared.sources.values() {
        retention.extend(source.artifact.retention.iter().cloned());
    }
    for input in prepared.inputs.values() {
        retention.extend(input.artifact.retention.iter().cloned());
    }
    for identity in prepared.packages.values() {
        retention.insert(gripsack_store::content_path(
            &ctx.home,
            "workspace-package",
            &identity.to_string(),
        ));
    }
    let retention = RetentionSet::admit(&session, retention)?;
    let packages = publish_packages(&prepared, ctx, &session)?;
    let mut closures = Vec::with_capacity(selected.len());
    for name in selected {
        let paths = if let Some(package) = packages.get(name.as_str()) {
            let mut paths = BTreeSet::new();
            package.retain_into(&mut paths);
            paths
        } else {
            prepared.retained_inputs(ctx, name)
        };
        if !paths.is_subset(retention.paths()) {
            return Err(failure(
                "realized consumer closure contains an object outside its protected build roots",
            ));
        }
        closures.push((name.clone(), paths));
    }
    Ok(HeldRealization {
        realization: Realization {
            packages,
            inputs: input_paths(&prepared),
        },
        session,
        closures,
    })
}

fn publish_packages<'a>(
    prepared: &Prepared<'a>,
    ctx: &Ctx,
    session: &LifecycleSession,
) -> Result<BTreeMap<&'a str, Arc<Package>>, ExecError> {
    let mut packages = BTreeMap::new();
    let mut pending: BTreeSet<_> = prepared
        .selection
        .required
        .iter()
        .copied()
        .filter(|name| {
            matches!(
                prepared.selection.outputs[name],
                WorkspaceOutput::Package(_)
            )
        })
        .collect();
    while !pending.is_empty() {
        let before = pending.len();
        for name in pending.iter().copied().collect::<Vec<_>>() {
            let WorkspaceOutput::Package(package) = prepared.selection.outputs[name] else {
                unreachable!();
            };
            if package
                .runtime
                .iter()
                .any(|runtime| !packages.contains_key(runtime.as_str()))
            {
                continue;
            }
            let (producer, identity) = match &package.producer {
                WorkspaceProducer::Provider { .. } => (
                    Arc::clone(&prepared.sources[name].artifact),
                    ProducerIdentity {
                        pin: &prepared.sources[name].resolved,
                        conda: prepared.sources[name].conda.as_ref(),
                    },
                ),
                WorkspaceProducer::Recipe { .. } => {
                    return Err(failure("executable recipe production has been retired"));
                }
            };
            let runtime = package
                .runtime
                .iter()
                .map(|name| Arc::clone(&packages[name.as_str()]))
                .collect();
            packages.insert(
                name,
                artifact::publish_package(
                    ctx,
                    session,
                    package,
                    producer,
                    identity,
                    runtime,
                    &prepared.packages,
                )?,
            );
            pending.remove(name);
        }
        if pending.len() == before {
            return Err(failure("runtime package closure cannot be published"));
        }
    }
    Ok(packages)
}
fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        output: "workspace".into(),
        step: "build".into(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod evidence_tests;
