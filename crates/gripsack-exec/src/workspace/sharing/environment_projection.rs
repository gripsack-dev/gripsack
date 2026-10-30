//! Recipient-bound command projections over admitted passive environment members.
//! Publication reuses package receipts, environment admission and managed launchers.
use super::{
    environment::{AdmittedEnvironment, AdmittedMember, AdmittedPackage},
    receipt, runtime,
};
use crate::workspace::{
    artifact::{self, Artifact, Package, ProducerIdentity},
    consumer::admit::{CapturedCommand, EnvironmentPlan, NativeContext},
    prepare::PreparedSource,
    realize::Realization,
    roots::{self, RetentionSet, RootId},
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_fetch::FetchLimits;
use gripsack_ir::{
    Span,
    workspace::{PlatformAbi, PlatformArch, PlatformOs, WorkspacePlatform},
    workspace_model::{
        AcquisitionSource, CatalogPackageLayout, EnvironmentOutput, PackageOutput, WorkspaceOutput,
        WorkspaceProducer, lock::ResolvedPinFields,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

const VERSION: u32 = 3;
const RECEIPT: &str = "projection.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionReceipt {
    version: u32,
    authority: String,
    manifest: String,
    home: PathBuf,
    core: PathBuf,
    tree: gripsack_store::hash::PayloadHash,
    retention: BTreeSet<PathBuf>,
    #[serde(skip)]
    activation: String,
}

/// Only derived projections are owned here; admitted package receipts stay
/// borrowed, including potentially large Conda inventories.
enum MemberSource<'a> {
    Package(&'a AdmittedPackage),
    Environment(Arc<ProjectedEnvironment>),
}

impl MemberSource<'_> {
    fn source(&self) -> &PreparedSource {
        match self {
            Self::Package(package) => &package.source,
            Self::Environment(environment) => &environment.source,
        }
    }

    fn fixed_prefix(&self) -> Option<&PathBuf> {
        match self {
            Self::Package(package) => package.fixed_prefix.as_ref(),
            Self::Environment(_) => None,
        }
    }

    fn nested_runtime(&self) -> &[Arc<Package>] {
        match self {
            Self::Package(_) => &[],
            Self::Environment(environment) => &environment.nested_runtime,
        }
    }
}

/// Realized member input used by declaration and SCC publication passes.
struct MemberProduction<'a> {
    input: MemberSource<'a>,
    target: WorkspacePlatform,
}

struct ProjectedEnvironment {
    source: PreparedSource,
    nested_runtime: Vec<Arc<Package>>,
}

/// Manifest identity is sufficient only inside this fixed authority/home/core
/// operation. Never reuse this memo across independent admissions or authorities.
struct ProjectionOperation<'a> {
    ctx: &'a Ctx,
    session: &'a LifecycleSession,
    authority: &'a str,
    core: PathBuf,
    limits: FetchLimits,
    projected: BTreeMap<String, Arc<ProjectedEnvironment>>,
    admission: super::ReadmissionScope,
}

fn destination(
    home: &Path,
    authority: &str,
    identity: &str,
    core: &Path,
) -> Result<PathBuf, ExecError> {
    let key = gripsack_store::hash::hex_sha256(&serde_json::to_vec(&(
        VERSION, authority, identity, home, core,
    ))?);
    Ok(gripsack_store::content_path(
        home,
        "package-environment-projection",
        &key,
    ))
}

pub(in crate::workspace) fn acquire(
    ctx: &Ctx,
    session: &LifecycleSession,
    identity: &str,
    limits: FetchLimits,
) -> Result<PreparedSource, ExecError> {
    if ctx.home != session.home() {
        return Err(runtime::failure(
            "environment projection session belongs to another home",
        ));
    }
    match runtime::admit_source(&ctx.home, identity, limits)? {
        Some(runtime::AdmittedSource {
            environment: Some(environment),
            ..
        }) => {
            let source = project(ctx, session, identity, &environment, limits)?;
            admit_captured_source(&ctx.home, identity, &source)?;
            Ok(source)
        }
        Some(runtime::AdmittedSource { prepared, .. }) => Ok(prepared),
        None => Err(runtime::failure(
            "package is not imported; import its signed bundle first",
        )),
    }
}

pub(in crate::workspace) fn inspect_source(
    home: &Path,
    identity: &str,
    limits: FetchLimits,
) -> Result<Option<PreparedSource>, ExecError> {
    match runtime::admit_source(home, identity, limits)? {
        Some(runtime::AdmittedSource {
            environment: Some(environment),
            ..
        }) => {
            let core = std::env::current_exe()?.canonicalize()?;
            let source = retained(home, identity, &environment, &core, limits)?;
            if let Some(source) = &source {
                admit_captured_source(home, identity, source)?;
            }
            Ok(source)
        }
        Some(runtime::AdmittedSource { prepared, .. }) => Ok(Some(prepared)),
        None => Ok(None),
    }
}

fn admit_captured_source(
    home: &Path,
    identity: &str,
    source: &PreparedSource,
) -> Result<(), ExecError> {
    let directory = artifact::retained_directory(home, &source.artifact.root)?
        .ok_or_else(|| runtime::failure("captured environment projection disappeared"))?;
    let record = read_record(&directory)?;
    let detected = crate::facts::detect();
    let facts = gripsack_ir::HostFacts {
        os: detected.os.into(),
        arch: detected.arch.into(),
        libc: detected.libc.clone(),
        tags: Vec::new(),
    };
    let context = NativeContext::new(
        &facts,
        home,
        Instant::now() + gripsack_process::Limits::default().timeout,
    )?;
    let span = Span {
        file: format!("<package:{identity}>"),
        line: 1,
        col: None,
    };
    crate::workspace::consumer::launcher::admit_imported_environment(
        home,
        &record.activation,
        &context,
        &span,
    )
}

fn source(root: PathBuf, record: ProjectionReceipt) -> PreparedSource {
    let payload = root.join("payload");
    let mut retention = record.retention;
    retention.insert(root.clone());
    PreparedSource {
        resolved: ResolvedPinFields {
            sha256: Some(record.manifest),
            tree256: Some(record.tree.to_string()),
            ..Default::default()
        },
        artifact: Arc::new(Artifact {
            root,
            payload,
            tree: record.tree,
            retention,
        }),
        conda: None,
    }
}

fn retained(
    home: &Path,
    authority: &str,
    environment: &AdmittedEnvironment,
    core: &Path,
    limits: FetchLimits,
) -> Result<Option<PreparedSource>, ExecError> {
    let root = destination(home, authority, &environment.identity, core)?;
    let Some(directory) = artifact::retained_directory(home, &root)? else {
        return Ok(None);
    };
    let record = read_record(&directory)?;
    if record.authority != authority
        || record.manifest != environment.identity
        || record.core != core
        || record.home != home
        || !environment.artifact.retention.is_subset(&record.retention)
    {
        return Err(runtime::failure(
            "environment projection binding differs from admitted members",
        ));
    }
    check_record(home, &root, &record, limits)?;
    Ok(Some(source(root, record)))
}

fn read_record(directory: &gripsack_fs::Dir) -> Result<ProjectionReceipt, ExecError> {
    let mut record: ProjectionReceipt = serde_json::from_slice(&receipt::read(
        directory,
        Path::new(RECEIPT),
        receipt::MAX_MATERIALIZATION_RECORD_BYTES as u64,
    )?)?;
    record.activation = String::from_utf8(receipt::read(
        directory,
        Path::new("payload/activation.json"),
        crate::workspace::consumer::launcher::MAX_PROJECTION_BYTES as u64,
    )?)
    .map_err(|error| runtime::failure(error.to_string()))?;
    Ok(record)
}

fn check_record(
    home: &Path,
    root: &Path,
    record: &ProjectionReceipt,
    limits: FetchLimits,
) -> Result<(), ExecError> {
    if record.version != VERSION
        || record.home != home
        || destination(home, &record.authority, &record.manifest, &record.core)? != root
    {
        return Err(runtime::failure(
            "unsupported or foreign environment projection receipt",
        ));
    }
    for path in &record.retention {
        if artifact::retained_directory(home, path)?.is_none() {
            return Err(runtime::failure(
                "environment projection retained member is missing",
            ));
        }
    }
    let checked = crate::workspace::stage::validate_output_tree(
        &root.join("payload"),
        gripsack_ir::workspace::RecipeOutputKind::Tree,
        "portable environment projection",
        limits,
    )?;
    if checked.tree_hash() != &record.tree {
        return Err(runtime::failure(
            "environment projection payload differs from receipt",
        ));
    }
    Ok(())
}

/// Prefix-materialized without a Conda receipt is valid only for this separately
/// receipted home-bound projection. A layout tag or metadata directory is not proof.
pub(in crate::workspace) fn readmit_projection(
    admission: &mut super::ReadmissionScope,
    payload: &Path,
    retention: &BTreeSet<PathBuf>,
) -> Result<(), ExecError> {
    let home = admission.home();
    let root = payload
        .parent()
        .ok_or_else(|| runtime::failure("projection payload has no root"))?;
    if payload != root.join("payload") || !retention.contains(root) {
        return Err(runtime::failure(
            "prefix-materialized payload has no environment projection authority",
        ));
    }
    let directory = artifact::retained_directory(home, root)?
        .ok_or_else(|| runtime::failure("environment projection receipt missing"))?;
    let record = read_record(&directory)?;
    check_record(home, root, &record, admission.limits())?;
    if !record.retention.is_subset(retention) {
        return Err(runtime::failure(
            "package omitted environment projection retention",
        ));
    }
    let environment = admission
        .admit_environment(&record.authority)?
        .ok_or_else(|| runtime::failure("projection import is not a passive environment"))?;
    if !contains(&environment, &record.manifest)
        || !environment.artifact.retention.is_subset(&record.retention)
    {
        return Err(runtime::failure(
            "projection is not bound to the imported environment closure",
        ));
    }
    Ok(())
}

fn contains(environment: &AdmittedEnvironment, identity: &str) -> bool {
    let mut pending = vec![environment];
    let mut visited = BTreeSet::new();
    while let Some(environment) = pending.pop() {
        if !visited.insert(environment.identity.as_str()) {
            continue;
        }
        if environment.identity == identity {
            return true;
        }
        for member in environment.components.values() {
            if let AdmittedMember::Environment(child) = member {
                pending.push(child);
            }
        }
    }
    false
}

fn project(
    ctx: &Ctx,
    session: &LifecycleSession,
    authority: &str,
    environment: &AdmittedEnvironment,
    limits: FetchLimits,
) -> Result<PreparedSource, ExecError> {
    let mut operation = ProjectionOperation {
        ctx,
        session,
        authority,
        core: std::env::current_exe()?.canonicalize()?,
        limits,
        projected: BTreeMap::new(),
        admission: super::ReadmissionScope::new(&ctx.home, limits),
    };
    // Only children need captured roots for attachment to a parent. The root
    // source is returned directly and receives ordinary native admission.
    project_uncached(&mut operation, environment)
}

fn project_node(
    operation: &mut ProjectionOperation<'_>,
    environment: &AdmittedEnvironment,
) -> Result<Arc<ProjectedEnvironment>, ExecError> {
    if let Some(projected) = operation.projected.get(&environment.identity) {
        return Ok(Arc::clone(projected));
    }
    let source = project_uncached(operation, environment)?;
    let directory = artifact::retained_directory(&operation.ctx.home, &source.artifact.root)?
        .ok_or_else(|| runtime::failure("nested environment capture disappeared"))?;
    let record = read_record(&directory)?;
    let nested_runtime = crate::workspace::consumer::launcher::captured_imported_packages(
        &mut operation.admission,
        &record.activation,
    )?;
    let projected = Arc::new(ProjectedEnvironment {
        source,
        nested_runtime,
    });
    operation
        .projected
        .insert(environment.identity.clone(), Arc::clone(&projected));
    Ok(projected)
}

fn project_uncached(
    operation: &mut ProjectionOperation<'_>,
    environment: &AdmittedEnvironment,
) -> Result<PreparedSource, ExecError> {
    let ctx = operation.ctx;
    let session = operation.session;
    let authority = operation.authority;
    let limits = operation.limits;
    if let Some(source) = retained(&ctx.home, authority, environment, &operation.core, limits)? {
        return Ok(source);
    }
    let root = destination(&ctx.home, authority, &environment.identity, &operation.core)?;
    let target = workspace_target(&environment.manifest.target);
    let span = Span {
        file: format!("<package:{}>", environment.identity),
        line: 1,
        col: None,
    };
    let mut members = BTreeMap::new();
    for (name, member) in &environment.components {
        let production = match member {
            AdmittedMember::Package(package) => MemberProduction {
                input: MemberSource::Package(package),
                target: workspace_target(&package.target),
            },
            AdmittedMember::Environment(child) => MemberProduction {
                input: MemberSource::Environment(project_node(operation, child)?),
                target: workspace_target(&child.manifest.target),
            },
        };
        members.insert(name.as_str(), production);
    }
    let core = &operation.core;
    let mut declarations = BTreeMap::new();
    for component in &environment.manifest.components {
        let production = &members[component.id.as_str()];
        let source = production.input.source();
        let target = &production.target;
        let fixed_prefix = production.input.fixed_prefix();
        let commands = environment
            .manifest
            .commands
            .iter()
            .filter(|(_, command)| command.component == component.id)
            .map(|(name, command)| (name.clone(), command.executable.clone()))
            .collect();
        let projected = source
            .artifact
            .root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("-package-environment-projection"));
        // The same signed child may be realized under several independent
        // authorities. Its immutable receipt retains the complete parent trust
        // context; bind that context (and all producer placement evidence) into
        // the explicit internal publication context rather than weakening
        // retention or replacing another parent's receipt. The provider remains
        // the actual signed identity, which also binds its original graph.
        let context = gripsack_store::hash::hex_sha256(&serde_json::to_vec(&(
            VERSION,
            authority,
            &environment.identity,
            &ctx.home,
            &core,
            &source.artifact.root,
            &source.artifact.payload,
            &source.artifact.retention,
            &source.conda,
        ))?);
        declarations.insert(
            component.id.clone(),
            WorkspaceOutput::Package(PackageOutput {
                name: format!("{}@{context}", component.id),
                span: span.clone(),
                producer: WorkspaceProducer::Provider {
                    provider: AcquisitionSource::Package {
                        manifest: environment.identity.clone(),
                        span: span.clone(),
                    },
                },
                commands,
                runtime: component.runtime.clone(),
                host_runtime: None,
                target: target.clone(),
                layout: if let Some(prefix) = fixed_prefix {
                    CatalogPackageLayout::FixedPrefix {
                        prefix: gripsack_ir::workspace::InstallPrefix(
                            prefix
                                .to_str()
                                .ok_or_else(|| runtime::failure("fixed prefix is not UTF-8"))?
                                .into(),
                        ),
                    }
                } else if source.conda.is_some() || projected {
                    CatalogPackageLayout::PrefixMaterialized
                } else {
                    CatalogPackageLayout::Relocatable
                },
                archive_root: None,
            }),
        );
    }
    let mut packages: BTreeMap<&str, Arc<Package>> = BTreeMap::new();
    for group in runtime_groups(&environment.manifest.components) {
        let names: BTreeSet<_> = group
            .iter()
            .map(|&index| environment.manifest.components[index].id.as_str())
            .collect();
        let cyclic = names.len() > 1
            || group.iter().any(|&index| {
                let component = &environment.manifest.components[index];
                component.runtime.contains(&component.id)
            });
        let mut bases = BTreeMap::new();
        for &name in &names {
            let WorkspaceOutput::Package(original) = &declarations[name] else {
                unreachable!()
            };
            let mut declaration = original.clone();
            if cyclic {
                declaration.name.push_str("/scc-base");
                // Only views export commands; bases authenticate member
                // payloads and outgoing runtime evidence.
                declaration.commands.clear();
                declaration
                    .runtime
                    .retain(|dependency| !names.contains(dependency.as_str()));
            }
            let mut runtime: Vec<_> = declaration
                .runtime
                .iter()
                .map(|name| Arc::clone(&packages[name.as_str()]))
                .collect();
            let mut runtime_names: BTreeMap<_, _> = declaration
                .runtime
                .iter()
                .map(|name| (name.clone(), packages[name.as_str()].identity))
                .collect();
            // A nested projection's wrapper prefix is not its native closure.
            // Attach its captured package roots explicitly; these are ordinary
            // immutable receipts and undergo the same ABI/current-trust checks.
            for nested in members[name].input.nested_runtime() {
                let key = format!("@nested/{}", nested.identity);
                declaration.runtime.push(key.clone());
                runtime_names.insert(key, nested.identity);
                runtime.push(Arc::clone(nested));
            }
            let source = members[name].input.source();
            let package = artifact::publish_package_with_context(
                ctx,
                session,
                &declaration,
                Arc::clone(&source.artifact),
                ProducerIdentity {
                    pin: &source.resolved,
                    conda: source.conda.as_ref(),
                },
                artifact::PackageRuntime {
                    packages: runtime,
                    identities: &runtime_names,
                },
                Some(declaration.name.as_bytes()),
            )?;
            bases.insert(name, package);
        }
        if !cyclic {
            packages.extend(bases);
            continue;
        }
        // Finite SCC lowering: each member view keeps its original source
        // prefix and includes every SCC base. Bases retain the outgoing DAG
        // edges; therefore each view reaches exactly the signed runtime closure,
        // including self-edges, without cyclic Arc graphs or recursive digests.
        // All bases remain real packages subject to ABI/current-trust admission.
        let runtime_names = bases
            .iter()
            .map(|(name, package)| ((*name).to_owned(), package.identity))
            .collect();
        for &name in &names {
            let WorkspaceOutput::Package(original) = &declarations[name] else {
                unreachable!()
            };
            let mut declaration = original.clone();
            declaration.name.push_str("/scc-view");
            declaration.runtime = names.iter().map(|name| (*name).to_owned()).collect();
            let source = members[name].input.source();
            let package = artifact::publish_package_with_context(
                ctx,
                session,
                &declaration,
                Arc::clone(&source.artifact),
                ProducerIdentity {
                    pin: &source.resolved,
                    conda: source.conda.as_ref(),
                },
                artifact::PackageRuntime {
                    packages: bases.values().cloned().collect(),
                    identities: &runtime_names,
                },
                Some(declaration.name.as_bytes()),
            )?;
            packages.insert(name, package);
        }
    }
    let declaration = EnvironmentOutput {
        name: environment.identity.clone(),
        span,
        packages: environment.manifest.roots.clone(),
        target,
        prefix: None,
        env: environment
            .manifest
            .environment
            .iter()
            .map(|(name, value)| (name.clone(), value.clone().into()))
            .collect(),
    };
    let outputs = declarations
        .iter()
        .map(|(name, output)| (name.as_str(), output))
        .collect();
    let realization = Realization {
        packages,
        inputs: BTreeMap::new(),
    };
    let detected = crate::facts::detect();
    let facts = gripsack_ir::HostFacts {
        os: detected.os.into(),
        arch: detected.arch.into(),
        libc: detected.libc.clone(),
        tags: Vec::new(),
    };
    let context = NativeContext::new(
        &facts,
        &ctx.home,
        Instant::now() + gripsack_process::Limits::default().timeout,
    )?;
    let export_owners = environment
        .manifest
        .commands
        .values()
        .map(|command| command.component.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let plan = EnvironmentPlan::<CapturedCommand>::capture_with_exports(
        &declaration,
        &outputs,
        &realization,
        &context,
        &export_owners,
    )?;
    let mut retention = environment.artifact.retention.clone();
    for package in realization.packages.values() {
        package.retain_into(&mut retention);
    }
    retention.extend(plan.closure().iter().cloned());
    // Register every input before exposing command bytes. Publication and the
    // final root replacement remain under the same lifecycle exclusion.
    let id = RootId::from_identity(root.as_os_str().as_encoded_bytes());
    roots::register_output_root(
        session,
        &id,
        &RetentionSet::admit(session, retention.iter().cloned())?,
    )?;
    let temporary = tempfile::Builder::new()
        .prefix("grip-environment-projection-")
        .tempdir()?;
    let payload = temporary.path().join("payload");
    let bin = payload.join("bin");
    std::fs::create_dir_all(&bin)?;
    let names = environment
        .manifest
        .commands
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let arguments = environment
        .manifest
        .commands
        .iter()
        .filter(|(_, command)| !command.args.is_empty())
        .map(|(name, command)| (name.clone(), command.args.clone()))
        .collect();
    let activation =
        plan.write_managed_commands_with_arguments(&ctx.home, &bin, &names, &arguments)?;
    std::fs::write(payload.join("activation.json"), &activation)?;
    let tree = gripsack_store::canonical_tree_hash(&payload)?;
    let record = ProjectionReceipt {
        version: VERSION,
        authority: authority.into(),
        manifest: environment.identity.clone(),
        home: ctx.home.clone(),
        core: core.clone(),
        tree,
        retention,
        activation,
    };
    std::fs::write(
        temporary.path().join(RECEIPT),
        receipt::encode_record(&record)?,
    )?;
    crate::source::publish(
        ctx,
        "package-environment-projection",
        temporary.path(),
        &root,
    )?;
    let source = source(root, record);
    roots::register_output_root(
        session,
        &id,
        &RetentionSet::admit(session, source.artifact.retention.iter().cloned())?,
    )?;
    Ok(source)
}

/// Tarjan's dependency-first SCC order. Portable admission bounds this local
/// graph to the closure component budget; independent manifest recursion is
/// separately bounded. No runtime edge is discarded by projection.
fn runtime_groups(components: &[super::format::Component]) -> Vec<Vec<usize>> {
    struct Graph {
        edges: Vec<Vec<usize>>,
        indices: Vec<Option<usize>>,
        low: Vec<usize>,
        active: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        groups: Vec<Vec<usize>>,
    }
    fn visit(node: usize, graph: &mut Graph) {
        let index = graph.next;
        graph.next += 1;
        graph.indices[node] = Some(index);
        graph.low[node] = index;
        graph.stack.push(node);
        graph.active[node] = true;
        for edge in 0..graph.edges[node].len() {
            let dependency = graph.edges[node][edge];
            if graph.indices[dependency].is_none() {
                visit(dependency, graph);
                graph.low[node] = graph.low[node].min(graph.low[dependency]);
            } else if graph.active[dependency] {
                graph.low[node] = graph.low[node].min(graph.indices[dependency].unwrap());
            }
        }
        if graph.low[node] == index {
            let mut group = Vec::new();
            loop {
                let member = graph.stack.pop().unwrap();
                graph.active[member] = false;
                group.push(member);
                if member == node {
                    break;
                }
            }
            graph.groups.push(group);
        }
    }
    let names: BTreeMap<_, _> = components
        .iter()
        .enumerate()
        .map(|(index, component)| (component.id.as_str(), index))
        .collect();
    let mut graph = Graph {
        edges: components
            .iter()
            .map(|component| {
                component
                    .runtime
                    .iter()
                    .map(|name| names[name.as_str()])
                    .collect()
            })
            .collect(),
        indices: vec![None; components.len()],
        low: vec![0; components.len()],
        active: vec![false; components.len()],
        stack: Vec::new(),
        next: 0,
        groups: Vec::new(),
    };
    for node in 0..components.len() {
        if graph.indices[node].is_none() {
            visit(node, &mut graph);
        }
    }
    graph.groups
}

fn workspace_target(target: &super::target::PortableTarget) -> WorkspacePlatform {
    WorkspacePlatform {
        os: PlatformOs::Linux,
        arch: PlatformArch::X86_64,
        abi: match target.abi {
            super::target::RuntimeAbi::Linux {} => None,
            super::target::RuntimeAbi::Glibc { .. } => Some(PlatformAbi::Gnu),
        },
        minimum_os: None,
    }
}

pub(in crate::workspace) fn admit_projected_command(
    home: &Path,
    package: &Package,
    selector: &str,
    context: &NativeContext<'_>,
    span: &Span,
    scope: &crate::workspace::consumer::admit::RuntimeScope<'_>,
    resolution: crate::workspace::consumer::admit::EnvironmentResolution,
) -> Result<crate::workspace::consumer::admit::AdmittedCommand, ExecError> {
    let directory = artifact::retained_directory(home, &package.producer.root)?
        .ok_or_else(|| runtime::failure("environment projection disappeared"))?;
    let record = read_record(&directory)?;
    let alias = selector
        .strip_prefix("bin/")
        .filter(|alias| !alias.contains('/'))
        .ok_or_else(|| runtime::failure("environment command must select bin/<alias>"))?;
    crate::workspace::consumer::launcher::admit_imported_command(
        home,
        &record.activation,
        alias,
        context,
        span,
        scope,
        resolution,
    )
}

pub(in crate::workspace) fn projected_entries(
    home: &Path,
    package: &Package,
) -> Result<Vec<(std::ffi::OsString, std::ffi::OsString)>, ExecError> {
    if !matches!(package.layout, CatalogPackageLayout::PrefixMaterialized)
        || package.conda.is_some()
    {
        return Ok(Vec::new());
    }
    let directory = artifact::retained_directory(home, &package.producer.root)?
        .ok_or_else(|| runtime::failure("environment projection disappeared"))?;
    let record = read_record(&directory)?;
    crate::workspace::consumer::launcher::imported_entries(&record.activation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_dag_membership_reaches_leaf_and_finishes_absent_lookup() {
        let directory = tempfile::tempdir().unwrap();
        let artifact = Arc::new(Artifact {
            root: directory.path().into(),
            payload: directory.path().into(),
            tree: gripsack_store::canonical_tree_hash(directory.path()).unwrap(),
            retention: BTreeSet::new(),
        });
        let mut child: Option<Arc<AdmittedEnvironment>> = None;
        let mut leaf_identity = String::new();
        for depth in 0..26 {
            let mut manifest: super::super::format::PortableManifest =
                serde_json::from_value(serde_json::json!({
                    "version": 1,
                    "namespace": "fixture/environment",
                    "publisher": "fixture-publisher",
                    "target": {
                        "os": "linux", "arch": "x86_64",
                        "abi": {"kind": "linux"},
                        "minimum_kernel": "3.2", "cpu": "x86_64"
                    },
                    "layout": {"kind": "environment"},
                    "components": [], "roots": [], "commands": {},
                    "environment": {}, "licenses": [],
                    "redistribution_authorized": true
                }))
                .unwrap();
            let mut components = BTreeMap::new();
            if let Some(child) = child {
                for name in ["left", "right"] {
                    manifest.components.push(super::super::format::Component {
                        id: name.into(),
                        payload: None,
                        tree: None,
                        runtime: Vec::new(),
                        conda: None,
                        external: Some(super::super::format::ExternalReference {
                            manifest: child.identity.clone(),
                            namespace: child.manifest.namespace.clone(),
                            publisher: child.manifest.publisher.clone(),
                        }),
                        inventory: Vec::new(),
                    });
                    manifest.roots.push(name.into());
                    components.insert(name.into(), AdmittedMember::Environment(Arc::clone(&child)));
                }
            }
            let identity =
                gripsack_store::hash::hex_sha256(&serde_json::to_vec(&manifest).unwrap());
            if depth == 0 {
                leaf_identity = identity.clone();
            }
            child = Some(Arc::new(AdmittedEnvironment {
                manifest,
                identity,
                artifact: Arc::clone(&artifact),
                components,
            }));
        }
        let root = child.unwrap();
        assert!(contains(&root, &root.identity));
        assert!(contains(&root, &leaf_identity));
        // A negative lookup must examine 26 nodes, not all 2**25 leaf paths.
        assert!(!contains(&root, &"f".repeat(64)));
    }
}
