//! Linux x86_64 native admission: host target compatibility, executable layout
//! verification and environment composition. Everything in this module runs
//! before any process launches; unsupported shapes fail with named reasons
//! and both declaration spans where two declarations collide.
pub(super) mod bytecode;
mod command;
pub(in crate::workspace) use command::EnvironmentResolution;
pub(super) use command::{admit_command, admit_command_in_scope};
mod elf;
mod native;
mod platform;
pub(in crate::workspace) use native::NativeContext;
#[cfg(all(test, target_os = "linux", target_env = "gnu"))]
mod host_runtime_tests;
#[cfg(test)]
mod tests;
use super::super::{artifact::Package, realize::Realization};
use crate::ExecError;
use gripsack_ir::{
    Diagnostic, Span, codes,
    workspace::WorkspacePlatform,
    workspace_model::{
        CatalogPackageLayout, EnvironmentOutput, WorkspaceArg, WorkspaceOutput,
        identity::ExecutableDigest, lock::BytecodePolicy,
    },
};
use gripsack_policy::target::{
    BinaryAbi, TargetArch, TargetOs, TargetRequirement, supports_target,
};
use gripsack_process::{EnvironmentOverlay, Sha256Digest};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

/// One hashed executable cannot exceed the shared selection budget.
const EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;

/// Borrowed native runtime scopes survive nested environment interception.
/// Image ownership remains the leaf package; enclosing selected graphs still
/// grant runtime reachability. The chain is stack-bound, not another Arc graph.
pub(in crate::workspace) struct RuntimeScope<'a> {
    package: &'a Package,
    parent: Option<&'a RuntimeScope<'a>>,
}
impl<'a> RuntimeScope<'a> {
    pub(super) fn new(package: &'a Package, parent: Option<&'a Self>) -> Self {
        Self { package, parent }
    }

    pub(super) fn packages(&self) -> impl Iterator<Item = &Package> {
        std::iter::successors(Some(self), |scope| scope.parent).map(|scope| scope.package)
    }
}

/// Evaluated OS/architecture/ABI plus the native invocation's independently
/// measured OS release. Missing capability is never a wildcard.
pub(in crate::workspace) struct HostTarget {
    requirement: TargetRequirement,
    os: TargetOs,
    arch: TargetArch,
    abi: Option<BinaryAbi>,
}
impl HostTarget {
    pub fn from_facts(facts: &gripsack_ir::HostFacts) -> Result<Self, ExecError> {
        let os = match facts.os.as_str() {
            "linux" => TargetOs::Linux,
            other => {
                return Err(failure(&format!(
                    "native consumer admission does not support host os {other:?}"
                )));
            }
        };
        let arch = match facts.arch.as_str() {
            "x86_64" => TargetArch::X86_64,
            other => {
                return Err(failure(&format!(
                    "native consumer admission does not support host architecture {other:?}"
                )));
            }
        };
        let abi = match facts.libc.as_deref() {
            None => None,
            Some("musl") => Some(BinaryAbi::Musl),
            Some("darwin") => return Err(failure("Darwin ABI is not supported on Linux x86_64")),
            Some(libc) if libc.starts_with("glibc") => Some(BinaryAbi::Gnu),
            Some(_) => None,
        };
        Ok(Self {
            requirement: TargetRequirement {
                os,
                arch,
                abi,
                minimum_os: None,
            },
            os,
            arch,
            abi,
        })
    }
}

/// The one platform loader a relocatable dynamic executable may name for a
/// declared target ABI. Anything else is a loader assumption we reject.
fn platform_loader(os: TargetOs, arch: TargetArch, abi: BinaryAbi) -> Option<&'static str> {
    match (os, arch, abi) {
        (TargetOs::Linux, TargetArch::X86_64, BinaryAbi::Gnu) => {
            Some("/lib64/ld-linux-x86-64.so.2")
        }
        (TargetOs::Linux, TargetArch::X86_64, BinaryAbi::Musl) => Some("/lib/ld-musl-x86_64.so.1"),
        _ => None,
    }
}

/// Compare one declared target against the host before any launch, on the
/// same policy dimensions as the production kernel (os, arch, ABI, minimum
/// floor). The native capability rule is exact, never a wildcard:
///
/// - os and arch must equal the evaluated host facts;
/// - a declared ABI must equal the host's libc fact (an unknown host ABI
///   can never satisfy it);
/// - an undeclared ABI admits only executables the classifier proves carry
///   no dynamic ABI dependency (static ELF or scripts) — a dynamic loader
///   dependency without a declared ABI is rejected by ELF admission;
/// - a declared minimum-os floor must not exceed the measured host release.
///
/// A Linux artifact is refused by Mac admission (and vice versa) with its
/// own span.
pub(super) fn admit_target(
    target: &WorkspacePlatform,
    host: &HostTarget,
    span: &Span,
    what: &str,
) -> Result<(), ExecError> {
    let required = target.policy_requirement();
    // ABI-independent native capability is separate from the host's dynamic
    // loader capability. Every command claiming it must pass static/script
    // layout admission below; None never satisfies a declared dynamic ABI.
    let available = TargetRequirement {
        abi: if required.abi.is_none() {
            None
        } else {
            host.abi
        },
        ..host.requirement
    };
    if supports_target(&required, &available) {
        return Ok(());
    }
    Err(ExecError::Gate(
        Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, format!(
            "{what} targets {} with ABI {:?} and minimum OS {:?}; host {} has ABI {:?} and measured OS release {:?}",
            platform_name(target), required.abi, required.minimum_os, host_name(host), host.abi, host.requirement.minimum_os,
        )).with_label(Some(span.clone()), "incompatible target declared here"),
    ))
}

fn platform_name(target: &WorkspacePlatform) -> String {
    format!("{:?}-{:?}", target.os, target.arch).to_lowercase()
}
fn host_name(host: &HostTarget) -> String {
    format!("{:?}-{:?}", host.os, host.arch).to_lowercase()
}
pub(in crate::workspace) struct PinnedInterpreter {
    pub path: PathBuf,
    pub argument: Option<OsString>,
    pub executable: Sha256Digest,
}
pub(in crate::workspace) struct AdmittedCommand {
    pub path: PathBuf,
    pub executable: ExecutableDigest,
    /// Checked per-command lookup plan, including translated main-image origin
    /// paths. A changed dependency selection during translation is refused.
    pub library_dirs: Vec<PathBuf>,
    pub gnu_loader: Option<std::sync::Arc<gripsack_process::SelectedGnuLoader>>,
    pub interpreter: Option<PinnedInterpreter>,
    pub bytecode: Option<BytecodePolicy>,
    /// Imported contributions are defaults; recipient environment/task values win.
    pub entries: Vec<(OsString, OsString)>,
    pub arguments: Vec<OsString>,
    pub imported: Option<std::sync::Arc<super::launcher::ImportedEnvironment>>,
}

/// Authenticated package export metadata, deliberately not native admission.
/// Nested portable projections capture this before their enclosing runtime
/// graph exists; only common command admission can produce AdmittedCommand.
pub(in crate::workspace) struct CapturedCommand {
    package: std::sync::Arc<Package>,
    span: Span,
}

/// One environment composition with either captured export evidence or admitted
/// native commands. The default type is executable authority; CapturedCommand
/// permits passive nested projection but has no invocation/materialize methods.
/// Both phases share root ordering, collision checks and typed contributions.
pub(in crate::workspace) struct EnvironmentPlan<C = AdmittedCommand> {
    pub commands: BTreeMap<String, C>,
    pub bytecode: Option<BytecodePolicy>,
    entries: Vec<(OsString, OsString)>,
    profile_entries_start: usize,
    declared_path: Vec<PathBuf>,
    declared_library_path: Vec<PathBuf>,
    closure: BTreeSet<PathBuf>,
    launchers: BTreeMap<String, std::sync::Arc<super::launcher::Binding>>,
    packages: Vec<std::sync::Arc<Package>>,
}
impl EnvironmentPlan {
    pub fn admit(
        declaration: &EnvironmentOutput,
        outputs: &BTreeMap<&str, &WorkspaceOutput>,
        realization: &Realization,
        context: &NativeContext<'_>,
    ) -> Result<Self, ExecError> {
        Self::admit_with_exports(declaration, outputs, realization, context, &[])
    }

    pub fn admit_with_exports(
        declaration: &EnvironmentOutput,
        outputs: &BTreeMap<&str, &WorkspaceOutput>,
        realization: &Realization,
        context: &NativeContext<'_>,
        exports: &[String],
    ) -> Result<Self, ExecError> {
        EnvironmentPlan::<CapturedCommand>::capture_with_exports(
            declaration,
            outputs,
            realization,
            context,
            exports,
        )?
        .admit_commands(declaration, context)
    }
}

impl EnvironmentPlan<CapturedCommand> {
    /// Capture exact commands and typed contributions without assuming a child
    /// is a standalone native closure. Extra exports never become env roots.
    pub fn capture_with_exports(
        declaration: &EnvironmentOutput,
        outputs: &BTreeMap<&str, &WorkspaceOutput>,
        realization: &Realization,
        context: &NativeContext<'_>,
        exports: &[String],
    ) -> Result<Self, ExecError> {
        let host = &context.target;
        admit_target(&declaration.target, host, &declaration.span, "environment")?;
        let mut commands = BTreeMap::new();
        let mut owners: BTreeMap<&str, &WorkspaceOutput> = BTreeMap::new();
        let mut closure = BTreeSet::new();
        let mut bytecode = None;
        let mut launchers = BTreeMap::new();
        let mut packages = Vec::new();
        let mut imported_entries = Vec::new();
        let mut selected = BTreeSet::new();
        for name in declaration.packages.iter().chain(exports) {
            if !selected.insert(name) {
                continue;
            }
            let output = outputs[name.as_str()];
            let package = realization.packages[name.as_str()].as_ref();
            packages.push(std::sync::Arc::clone(&realization.packages[name.as_str()]));
            bytecode = context
                .admit_package_closure(package, output.span())?
                .or(bytecode);
            package.retain_into(&mut closure);
            if declaration.packages.contains(name) {
                imported_entries.extend(crate::workspace::sharing::projected_entries(
                    context.home,
                    package,
                )?);
            }
            let binding =
                super::launcher::Binding::capture(context.home, package)?.map(std::sync::Arc::new);
            for command in package.commands.keys() {
                if let Some(previous) = owners.insert(command, output) {
                    return Err(ExecError::Gate(
                        Diagnostic::error(
                            codes::BAD_WORKSPACE_CONTEXT,
                            format!("command {command:?} is exported by two selected packages"),
                        )
                        .with_label(Some(previous.span().clone()), "first export declared here")
                        .with_label(
                            Some(output.span().clone()),
                            "conflicting export declared here",
                        ),
                    ));
                }
                if let Some(binding) = &binding {
                    launchers.insert(command.clone(), std::sync::Arc::clone(binding));
                }
                commands.insert(
                    command.clone(),
                    CapturedCommand {
                        package: std::sync::Arc::clone(&realization.packages[name.as_str()]),
                        span: output.span().clone(),
                    },
                );
            }
        }
        let profile_entries_start = imported_entries.len();
        let mut entries = imported_entries;
        let mut declared_path = Vec::new();
        let mut declared_library_path = Vec::new();
        for (key, argument) in &declaration.env {
            match argument {
                WorkspaceArg::Artifact { output, .. } => {
                    if let Some(package) = realization.packages.get(output.as_str()) {
                        package.retain_into(&mut closure);
                    }
                }
                WorkspaceArg::Input { input } => {
                    if let Some(path) = realization.inputs.get(input.as_str()) {
                        let root = path
                            .ancestors()
                            .find(|path| {
                                gripsack_store::paths::validate_store_root(context.home, path)
                                    .is_ok()
                            })
                            .ok_or_else(|| {
                                failure("environment input has no retained store root")
                            })?;
                        closure.insert(root.to_path_buf());
                    }
                }
                _ => {}
            }
            let value = environment_value(argument, declaration, realization)?;
            match key.as_str() {
                "PATH" => declared_path = absolute_directories(&value, &declaration.span, "PATH")?,
                "LD_LIBRARY_PATH" => {
                    declared_library_path =
                        absolute_directories(&value, &declaration.span, "LD_LIBRARY_PATH")?
                }
                _ => entries.push((OsString::from(key), OsString::from(value))),
            }
        }
        bytecode::apply(bytecode, &mut entries, &declaration.span)?;
        Ok(Self {
            commands,
            bytecode,
            entries,
            profile_entries_start,
            declared_path,
            declared_library_path,
            closure,
            launchers,
            packages,
        })
    }

    fn admit_commands(
        self,
        declaration: &EnvironmentOutput,
        context: &NativeContext<'_>,
    ) -> Result<EnvironmentPlan, ExecError> {
        let commands = self
            .commands
            .into_iter()
            .map(|(name, captured)| {
                let provided = &captured.package.commands[&name];
                let command = admit_command(
                    &captured.package,
                    &provided.selector,
                    &provided.executable,
                    context,
                    &captured.span,
                )?;
                Ok((name, command))
            })
            .collect::<Result<BTreeMap<_, _>, ExecError>>()?;
        if let Some(loader) = commands
            .values()
            .find_map(|command| command.gnu_loader.as_ref())
        {
            for key in declaration.env.keys() {
                loader
                    .check_environment_key(std::ffi::OsStr::new(key))
                    .map_err(|error| gate(&declaration.span, error.to_string()))?;
            }
        }
        Ok(EnvironmentPlan {
            commands,
            bytecode: self.bytecode,
            entries: self.entries,
            profile_entries_start: self.profile_entries_start,
            declared_path: self.declared_path,
            declared_library_path: self.declared_library_path,
            closure: self.closure,
            launchers: self.launchers,
            packages: self.packages,
        })
    }
}

impl EnvironmentPlan {
    pub fn empty() -> Self {
        Self {
            commands: BTreeMap::new(),
            bytecode: None,
            entries: Vec::new(),
            profile_entries_start: 0,
            declared_path: Vec::new(),
            declared_library_path: Vec::new(),
            closure: BTreeSet::new(),
            launchers: BTreeMap::new(),
            packages: Vec::new(),
        }
    }

    /// Persist literal contributions and an explicit reference to the profile's
    /// command projection. User values never acquire legacy shell expansion.
    pub fn profile_env(&self) -> Result<Vec<gripsack_store::EnvironmentContribution>, ExecError> {
        use gripsack_store::{EnvironmentContribution, StructuredEnvironment};
        let mut variables = Vec::new();
        for (name, paths) in [
            ("PATH", &self.declared_path),
            ("LD_LIBRARY_PATH", &self.declared_library_path),
        ] {
            if paths.is_empty() {
                continue;
            }
            let value = std::env::join_paths(paths)
                .map_err(operational)?
                .into_string()
                .map_err(|_| failure("profile path is not UTF-8"))?;
            variables.push(EnvironmentContribution::Structured(
                StructuredEnvironment::Literal {
                    name: name.into(),
                    op: gripsack_store::EnvironmentOperation::Prepend,
                    value,
                },
            ));
        }
        if !self.commands.is_empty() {
            variables.push(EnvironmentContribution::Structured(
                StructuredEnvironment::StorePath {
                    name: "PATH".into(),
                    op: gripsack_store::EnvironmentOperation::Prepend,
                    path: "commands".into(),
                },
            ));
        }
        for (key, value) in self.entries.iter().skip(self.profile_entries_start) {
            let name = key
                .to_str()
                .ok_or_else(|| failure("profile variable name is not UTF-8"))?;
            if !crate::env::is_identifier(name) {
                return Err(failure("profile variable name is not a POSIX identifier"));
            }
            variables.push(EnvironmentContribution::Structured(
                StructuredEnvironment::Literal {
                    name: name.to_owned(),
                    op: gripsack_store::EnvironmentOperation::Set,
                    value: value
                        .to_str()
                        .ok_or_else(|| failure("profile variable value is not UTF-8"))?
                        .to_owned(),
                },
            ));
        }
        Ok(variables)
    }

    /// Shared projection preserves the selected program's argv0, including a
    /// multicall selector exported under another name, and its exact runtime
    /// search plan. The public alias is never substituted for the selector.
    pub fn write_commands(&self, bin_directory: &Path) -> Result<(), ExecError> {
        let projection = if self.launchers.is_empty() {
            None
        } else {
            let overlay = EnvironmentOverlay::admit(
                self.entries.iter().cloned(),
                [],
                self.declared_library_path.iter().cloned(),
            )
            .map_err(operational)?;
            Some(super::launcher::encode_projection(&overlay)?)
        };
        self.write_projected_commands(bin_directory, projection.as_deref())
    }

    fn write_projected_commands(
        &self,
        bin_directory: &Path,
        projection: Option<&str>,
    ) -> Result<(), ExecError> {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        for (name, command) in &self.commands {
            if let Some(binding) = self.launchers.get(name) {
                binding.write(
                    bin_directory,
                    name,
                    projection.ok_or_else(|| {
                        failure("managed command projection has no environment binding")
                    })?,
                )?;
                continue;
            }
            let destination = bin_directory.join(name);
            let libraries = std::env::join_paths(
                command
                    .library_dirs
                    .iter()
                    .chain(&self.declared_library_path),
            )
            .map_err(operational)?;
            let libraries = libraries
                .to_str()
                .ok_or_else(|| failure("library search path is not UTF-8"))?;
            let program = command
                .path
                .to_str()
                .ok_or_else(|| failure("command path is not UTF-8"))?;
            let mut file = std::fs::File::create_new(destination).map_err(operational)?;
            writeln!(file, "#!/bin/sh").map_err(operational)?;
            if let Some(BytecodePolicy::Suppress) = command.bytecode {
                writeln!(file, "export PYTHONDONTWRITEBYTECODE=1").map_err(operational)?;
            }
            if !libraries.is_empty() {
                write!(
                    file,
                    "LD_LIBRARY_PATH={} ",
                    crate::env::ShellLiteral(libraries)
                )
                .map_err(operational)?;
            }
            if let Some(loader) = &command.gnu_loader {
                let loader = loader
                    .path()
                    .to_str()
                    .ok_or_else(|| failure("platform loader path is not UTF-8"))?;
                let subject = command
                    .interpreter
                    .as_ref()
                    .map_or(&command.path, |interpreter| &interpreter.path);
                let subject = subject
                    .to_str()
                    .ok_or_else(|| failure("interpreter path is not UTF-8"))?;
                write!(file,"exec {} --inhibit-cache --glibc-hwcaps-mask '' --inhibit-rpath '' --library-path {} --argv0 {} {}",
                    crate::env::ShellLiteral(loader),crate::env::ShellLiteral(libraries),
                    crate::env::ShellLiteral(subject),crate::env::ShellLiteral(subject)).map_err(operational)?;
                if let Some(interpreter) = &command.interpreter {
                    if let Some(argument) = &interpreter.argument {
                        let argument = argument
                            .to_str()
                            .ok_or_else(|| failure("interpreter argument is not UTF-8"))?;
                        write!(file, " {}", crate::env::ShellLiteral(argument))
                            .map_err(operational)?;
                    }
                    write!(file, " {}", crate::env::ShellLiteral(program)).map_err(operational)?;
                }
            } else {
                write!(file, "exec {}", crate::env::ShellLiteral(program)).map_err(operational)?;
            }
            writeln!(file, " \"$@\"").map_err(operational)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o755))
                .map_err(operational)?;
        }
        Ok(())
    }

    /// A task/check's declarations extend the environment. Give that merged
    /// overlay its own bin under the existing process lease, so nested launches
    /// preserve its names without rewriting another step's live projection.
    pub(super) fn project_command_environment(
        &self,
        base: &EnvironmentOverlay,
        composed: EnvironmentOverlay,
    ) -> Result<EnvironmentOverlay, ExecError> {
        if self.launchers.is_empty() {
            return Ok(composed);
        }
        let bin = base
            .search_prefix()
            .first()
            .ok_or_else(|| failure("task environment has no leased command projection"))?;
        let parent = bin
            .parent()
            .ok_or_else(|| failure("command projection has no leased parent"))?;
        let directory = tempfile::Builder::new()
            .prefix("command-")
            .tempdir_in(parent)?
            .keep();
        let projection = super::launcher::encode_projection(&composed)?;
        self.write_projected_commands(&directory, Some(&projection))?;
        let search =
            std::iter::once(directory).chain(composed.search_prefix().iter().skip(1).cloned());
        EnvironmentOverlay::admit(
            composed
                .entries()
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
            search,
            composed.library_prefix().iter().cloned(),
        )
        .map_err(operational)
    }

    /// Expose exactly the exported commands through `bin_directory` and build
    /// the process overlay: [bin dir, declared PATH dirs] then the operator
    /// PATH; the launched command's admitted library dirs precede the
    /// declared library segment.
    pub fn materialize(
        &self,
        bin_directory: &Path,
        launched: Option<&AdmittedCommand>,
    ) -> Result<EnvironmentOverlay, ExecError> {
        self.write_commands(bin_directory)?;
        let search: Vec<PathBuf> = std::iter::once(bin_directory.to_path_buf())
            .chain(self.declared_path.iter().cloned())
            .collect();
        let mut library: Vec<PathBuf> = launched
            .map(|command| command.library_dirs.clone())
            .unwrap_or_default();
        library.extend(self.declared_library_path.iter().cloned());
        let entries = launched
            .into_iter()
            .flat_map(|command| command.entries.iter())
            .chain(&self.entries)
            .cloned();
        let overlay = EnvironmentOverlay::admit(entries, search, library).map_err(operational)?;
        super::launcher::compose_imported(launched, overlay)
    }
}

impl<C> EnvironmentPlan<C> {
    pub fn closure(&self) -> &BTreeSet<PathBuf> {
        &self.closure
    }
    /// Explicit profile launchers freeze activation values and the complete
    /// receipt graph, unlike historical projections that grant ambient names.
    pub(in crate::workspace) fn write_managed_commands(
        &self,
        home: &Path,
        directory: &Path,
        names: &[String],
    ) -> Result<(), ExecError> {
        self.write_managed_commands_with_arguments(home, directory, names, &BTreeMap::new())
            .map(|_| ())
    }

    pub(in crate::workspace) fn write_managed_commands_with_arguments(
        &self,
        home: &Path,
        directory: &Path,
        names: &[String],
        arguments: &BTreeMap<String, Vec<String>>,
    ) -> Result<String, ExecError> {
        super::launcher::ProfileEnvironment {
            commands: &self.commands,
            packages: &self.packages,
            entries: &self.entries,
            search_directories: &self.declared_path,
            library_directories: &self.declared_library_path,
            retention: &self.closure,
        }
        .write_with_arguments(home, directory, names, arguments)
    }
}

fn environment_value(
    argument: &WorkspaceArg,
    declaration: &EnvironmentOutput,
    realization: &Realization,
) -> Result<String, ExecError> {
    let value = match argument {
        WorkspaceArg::Literal { value } => value.clone(),
        WorkspaceArg::Artifact { output, selector } => {
            let payload = artifact_payload(output, realization).ok_or_else(|| {
                gate(
                    &declaration.span,
                    format!("environment value names unrealized artifact {output:?}"),
                )
            })?;
            let path = payload.join(selector);
            let canonical = path.canonicalize().map_err(|_| {
                gate(
                    &declaration.span,
                    format!("environment artifact {output:?}/{selector:?} does not resolve"),
                )
            })?;
            if !canonical.starts_with(&payload) {
                return Err(gate(
                    &declaration.span,
                    "environment artifact escapes its payload",
                ));
            }
            utf8_path(&canonical, &declaration.span)?
        }
        WorkspaceArg::Input { input } => {
            let path = realization.inputs.get(input.as_str()).ok_or_else(|| {
                gate(
                    &declaration.span,
                    format!("environment value names uncaptured input {input:?}"),
                )
            })?;
            utf8_path(path, &declaration.span)?
        }
        WorkspaceArg::PackageCommand { .. }
        | WorkspaceArg::Source { .. }
        | WorkspaceArg::Output { .. } => {
            return Err(gate(
                &declaration.span,
                "this binding kind is not environment data (sema E128)",
            ));
        }
    };
    Ok(value)
}

pub(super) fn artifact_payload(name: &str, realization: &Realization) -> Option<PathBuf> {
    realization
        .packages
        .get(name)
        .map(|package| package.payload().to_path_buf())
}

fn absolute_directories(value: &str, span: &Span, key: &str) -> Result<Vec<PathBuf>, ExecError> {
    let mut directories = Vec::new();
    for entry in value.split(':').filter(|entry| !entry.is_empty()) {
        let path = PathBuf::from(entry);
        if !path.is_absolute() {
            return Err(gate(
                span,
                format!("declared {key} entry {entry:?} is not absolute"),
            ));
        }
        directories.push(path);
    }
    Ok(directories)
}

pub(super) fn utf8_path(path: &Path, span: &Span) -> Result<String, ExecError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| gate(span, "path is not UTF-8"))
}

pub(super) fn gate(span: &Span, detail: impl Into<String>) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, detail.into())
            .with_label(Some(span.clone()), "consumer declaration here"),
    )
}

pub(super) fn failure(detail: &str) -> ExecError {
    ExecError::Step {
        output: "workspace".into(),
        step: "consumer".into(),
        detail: detail.into(),
    }
}

pub(super) fn operational(error: impl std::fmt::Display) -> ExecError {
    failure(&error.to_string())
}
