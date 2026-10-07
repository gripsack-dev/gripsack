//! Exact retained data for a hook; current repository source is never consulted.
use super::*;
use crate::workspace::{artifact, consumer::{self, admit}, realize::Realization};
use gripsack_ir::workspace_v6::{WorkspaceArg, WorkspaceCommand, WorkspacePath, identity::PackageDigest};
use gripsack_process::ProgramIdentity;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io::Read, path::PathBuf};

pub(super) const MAX_CONTEXT_BYTES: u64 = 4 * 1024 * 1024;
const CONTEXT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Context {
    version: u32,
    pub command: super::command::Command,
    pub program: ProgramIdentity,
    packages: BTreeMap<String, PackageDigest>,
    receipts: BTreeMap<PackageDigest, Sha256Digest>,
    roots: BTreeMap<PathBuf, store::hash::PayloadHash>,
}

pub(super) fn capture(
    command: &WorkspaceCommand,
    realization: &Realization,
    home: &Path,
    host: &admit::NativeContext<'_>,
    options: &crate::workspace::BuildOptions<'_>,
) -> Result<Context, ExecError> {
    let base = gripsack_process::EnvironmentOverlay::admit(Vec::new(), Vec::new(), Vec::new())?;
    let prepared = consumer::command::prepare(
        command, None, &admit::EnvironmentPlan::empty(), realization, host,
        &std::env::temp_dir(), options, &base,
    )?;
    let (program, environment) = match command {
        WorkspaceCommand::Exec { argv, env, .. } => (
            argv.first().ok_or_else(|| crate::workspace::file_failure(command.span(), "hook has no program"))?, env,
        ),
        WorkspaceCommand::RunBash { interpreter, env, .. } => (interpreter, env),
    };
    let mut packages = BTreeMap::new();
    let mut receipts = BTreeMap::new();
    let mut roots = BTreeSet::new();
    for argument in command.arguments() {
        retain_argument(argument, realization, home, host, command.span(), &mut packages, &mut receipts, &mut roots)?;
    }
    let program = if matches!(program, WorkspaceArg::PackageCommand { .. }) {
        program.clone()
    } else {
        WorkspaceArg::Literal { value: consumer::task_argument(program, None, realization, host, command.span())?
            .into_string().map_err(|_| io::Error::from(io::ErrorKind::InvalidData))? }
    };
    let environment = environment.iter().map(|(key, argument)| {
        let value = consumer::task_argument(argument, None, realization, host, command.span())?;
        Ok((key.clone(), value.into_string().map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?))
    }).collect::<Result<_, ExecError>>()?;
    if matches!(command.working_directory(), Some(WorkspacePath::Artifact { .. })) {
        retain_path(home, &prepared.cwd, &mut roots)?;
    }
    let roots = roots.into_iter().map(|root| {
        store::canonical_tree_hash(&root).map(|digest| (root, digest))
    }).collect::<io::Result<_>>()?;
    let identity = prepared.program.identity();
    let command = super::command::Command::freeze(program, prepared.argv, environment, prepared.cwd)?;
    Ok(Context { version: CONTEXT_VERSION, command, program: identity, packages, receipts, roots })
}

#[allow(clippy::too_many_arguments)]
fn retain_argument(
    argument: &WorkspaceArg,
    realization: &Realization,
    home: &Path,
    host: &admit::NativeContext<'_>,
    span: &Span,
    packages: &mut BTreeMap<String, PackageDigest>,
    receipts: &mut BTreeMap<PackageDigest, Sha256Digest>,
    roots: &mut BTreeSet<PathBuf>,
) -> Result<(), ExecError> {
    if let WorkspaceArg::PackageCommand { package, .. } = argument {
        let retained = realization.packages.get(package.as_str())
            .ok_or_else(|| crate::workspace::file_failure(span, "hook package is not realized"))?;
        artifact::hook::capture(home, retained, receipts)?;
        retained.retain_into(roots);
        packages.insert(package.clone(), retained.identity);
        return Ok(());
    }
    if matches!(argument, WorkspaceArg::Literal { .. }) {
        return Ok(());
    }
    let value = consumer::task_argument(argument, None, realization, host, span)?;
    retain_path(home, Path::new(&value), roots)?;
    Ok(())
}

fn retain_path(home: &Path, path: &Path, roots: &mut BTreeSet<PathBuf>) -> Result<(), ExecError> {
    let store = home.join("store");
    let relative = path.strip_prefix(&store).map_err(|_| io::Error::new(
        io::ErrorKind::InvalidData, "hook binding is outside the immutable store"))?;
    let component = relative.components().next().ok_or_else(|| io::Error::new(
        io::ErrorKind::InvalidData, "hook binding has no immutable root"))?;
    let root = store.join(component);
    store::paths::validate_store_root(home, &root)?;
    roots.insert(root);
    Ok(())
}

pub(super) fn read(home: &Path, path: &Path, digest: Sha256Digest) -> Result<Context, ExecError> {
    let root = path.parent().ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
    store::paths::validate_store_root(home, root)?;
    let directory = artifact::retained_directory(home, root)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "retained hook context root is missing"))?;
    let name = path.file_name().ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
    let mut file = gripsack_fs::open_file_nofollow(&directory, Path::new(name))?;
    if file.metadata()?.len() > MAX_CONTEXT_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "retained hook context exceeds its byte budget").into());
    }
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_CONTEXT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONTEXT_BYTES || Sha256Digest::of(&bytes) != digest {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "retained hook context identity differs").into());
    }
    let context: Context = serde_json::from_slice(&bytes)?;
    if context.version != CONTEXT_VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported retained hook context version").into());
    }
    for (root, digest) in &context.roots {
        store::paths::validate_store_root(home, root)?;
        if store::canonical_tree_hash(root)? != *digest {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "retained hook input identity differs").into());
        }
    }
    Ok(context)
}

impl Context {
    pub(super) fn encode(&self) -> io::Result<Vec<u8>> {
        struct Encoder(Vec<u8>);
        impl io::Write for Encoder {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if bytes.len() > MAX_CONTEXT_BYTES as usize - self.0.len() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "retained hook context exceeds its byte budget"));
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let mut encoder = Encoder(Vec::new());
        serde_json::to_writer(&mut encoder, self).map_err(io::Error::other)?;
        Ok(encoder.0)
    }

    pub(super) fn realization<'a>(&'a self, home: &Path) -> Result<Realization<'a>, ExecError> {
        let mut loaded = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        let mut packages = BTreeMap::new();
        for (name, identity) in &self.packages {
            let package = artifact::hook::restore(home, *identity, &self.receipts, &mut loaded, &mut visiting)?;
            packages.insert(name.as_str(), package);
        }
        Ok(Realization { recipes: BTreeMap::new(), inputs: BTreeMap::new(), packages })
    }
}
