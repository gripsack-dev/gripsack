//! Native consumer admission: host target compatibility, executable layout
//! verification and environment composition. Everything in this module runs
//! before any process launches; unsupported shapes fail with named reasons
//! and both declaration spans where two declarations collide.
mod command;
pub(super) use command::admit_command;
mod elf;
#[cfg(test)]
mod tests;
use super::super::{artifact::Package, realize::Realization};
use crate::ExecError;
use gripsack_ir::{
    Diagnostic, Span, codes,
    workspace::WorkspacePlatform,
    workspace_v6::{
        EnvironmentOutput, PackageLayoutV6, WorkspaceArg, WorkspaceOutput, identity::ExecutableDigest,
    },
};
use gripsack_policy::target::{
    BinaryAbi, TargetArch, TargetOs, TargetRequirement, supports_target,
};
use gripsack_process::executable::{ExecutableFormat, ObjectKind};
use gripsack_process::{EnvironmentOverlay, Sha256Digest};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

/// One hashed executable cannot exceed the shared selection budget.
const EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;

/// The evaluated host platform as a consumer-side target requirement. The
/// host has no minimum-OS floor fact, so a package declaring a floor is
/// rejected explicitly rather than silently trusted.
pub(in crate::workspace) struct HostTarget {
    requirement: TargetRequirement,
    os: TargetOs,
    arch: TargetArch,
    abi: Option<BinaryAbi>,
    gnu_loader_controls: bool,
}
impl HostTarget {
    pub fn from_facts(facts: &gripsack_ir::HostFacts) -> Result<Self, ExecError> {
        let os = match facts.os.as_str() {
            "linux" => TargetOs::Linux,
            "macos" | "darwin" => TargetOs::Macos,
            other => {
                return Err(failure(&format!(
                    "native consumer admission does not support host os {other:?}"
                )));
            }
        };
        let arch = match facts.arch.as_str() {
            "x86_64" => TargetArch::X86_64,
            "aarch64" => TargetArch::Aarch64,
            other => {
                return Err(failure(&format!(
                    "native consumer admission does not support host architecture {other:?}"
                )));
            }
        };
        let abi = match facts.libc.as_deref() {
            None => None,
            Some("musl") => Some(BinaryAbi::Musl),
            Some("darwin") => Some(BinaryAbi::Darwin),
            Some(libc) if libc.starts_with("glibc") => Some(BinaryAbi::Gnu),
            Some(_) => None,
        };
        let gnu_loader_controls = facts.libc.as_deref()
            .and_then(|value| value.strip_prefix("glibc-"))
            .and_then(|value| {
                let mut fields = value.split('.');
                Some((fields.next()?.parse::<u32>().ok()?,fields.next()?.parse::<u32>().ok()?))
            }).is_some_and(|version| version >= (2,33));
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
            gnu_loader_controls,
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
        (TargetOs::Linux, TargetArch::Aarch64, BinaryAbi::Gnu) => {
            Some("/lib/ld-linux-aarch64.so.1")
        }
        (TargetOs::Linux, TargetArch::X86_64, BinaryAbi::Musl) => {
            Some("/lib/ld-musl-x86_64.so.1")
        }
        (TargetOs::Linux, TargetArch::Aarch64, BinaryAbi::Musl) => {
            Some("/lib/ld-musl-aarch64.so.1")
        }
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
/// - a declared minimum-os floor is rejected because the host reports no
///   os-release fact (unknown data is never claimed to match).
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
        abi: if required.abi.is_none() { None } else { host.abi },
        ..host.requirement
    };
    if supports_target(&required, &available) {
        return Ok(());
    }
    Err(ExecError::Gate(
        Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, format!(
            "{what} targets {} with ABI {:?} and minimum OS {:?}; host {} has ABI {:?} and no admitted OS-release floor",
            platform_name(target), required.abi, required.minimum_os, host_name(host), host.abi,
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
    pub gnu_loader: Option<&'static str>,
    pub interpreter: Option<PinnedInterpreter>,
}



fn admit_macho(
    metadata: &gripsack_process::executable::ExecutableMetadata,
    payload: &Path,
    host: &HostTarget,
    span: &Span,
) -> Result<Vec<PathBuf>, ExecError> {
    if host.os != TargetOs::Macos {
        return Err(gate(span, "Mach-O executable on a non-macOS host"));
    }
    if metadata.object != Some(ObjectKind::Executable) {
        return Err(gate(span, "Mach-O object is not an executable program"));
    }
    for needed in &metadata.needed_libraries {
        let text = needed.to_str().ok_or_else(|| {
            gate(span, "linked library name is not UTF-8")
        })?;
        if text.starts_with("/usr/lib/") || text.starts_with("/System/Library/") {
            continue;
        }
        if let Some(relative) = text
            .strip_prefix("@loader_path/")
            .or_else(|| text.strip_prefix("@executable_path/"))
        {
            if payload.join(relative).is_file() {
                continue;
            }
        }
        return Err(gate(
            span,
            format!("linked library {text:?} is outside the macOS system runtime and the package payload"),
        ));
    }
    Ok(Vec::new())
}

/// A checked environment composition. Commands resolve to exact admitted
/// executable paths inside package payloads; a command name exported by two
/// selected packages names both declaration spans. `materialize` exposes
/// exactly the exported commands through one per-launch directory.
pub(in crate::workspace) struct EnvironmentPlan {
    pub commands: BTreeMap<String, AdmittedCommand>,
    entries: Vec<(OsString, OsString)>,
    declared_path: Vec<PathBuf>,
    declared_library_path: Vec<PathBuf>,
    closure: BTreeSet<PathBuf>,
}
impl EnvironmentPlan {
    pub fn admit(
        declaration: &EnvironmentOutput,
        outputs: &BTreeMap<&str, &WorkspaceOutput>,
        realization: &Realization,
        host: &HostTarget,
    ) -> Result<Self, ExecError> {
        admit_target(
            &declaration.target,
            host,
            &declaration.span,
            "environment",
        )?;
        let mut commands = BTreeMap::new();
        let mut owners: BTreeMap<&str, &WorkspaceOutput> = BTreeMap::new();
        let mut closure = BTreeSet::new();
        for name in &declaration.packages {
            let output = outputs[name.as_str()];
            let package = realization.packages[name.as_str()].as_ref();
            admit_target(&package.target, host, output.span(), "package")?;
            if let PackageLayoutV6::FixedPrefix { prefix } = &package.layout {
                return Err(gate(
                    output.span(),
                    format!(
                        "fixed-prefix package (prefix {}) requires A3-owned prefix materialization; native project consumption admits relocatable packages",
                        prefix.as_str()
                    ),
                ));
            }
            if package.layout == PackageLayoutV6::PrefixMaterialized {
                return Err(gate(output.span(),"prefix-materialized package has no admitted native host-runtime capability"));
            }
            package.retain_into(&mut closure);
            for (command, provided) in &package.commands {
                if let Some(previous) = owners.insert(command, output) {
                    return Err(ExecError::Gate(
                        Diagnostic::error(
                            codes::BAD_WORKSPACE_CONTEXT,
                            format!("command {command:?} is exported by two selected packages"),
                        )
                        .with_label(Some(previous.span().clone()), "first export declared here")
                        .with_label(Some(output.span().clone()), "conflicting export declared here"),
                    ));
                }
                commands.insert(
                    command.clone(),
                    admit_command(
                        package,
                        &provided.selector,
                        &provided.executable,
                        host,
                        output.span(),
                    )?,
                );
            }
        }
        let mut entries = Vec::new();
        let mut declared_path = Vec::new();
        let mut declared_library_path = Vec::new();
        for (key, argument) in &declaration.env {
            let value = environment_value(argument, declaration, realization)?;
            match key.as_str() {
                "PATH" => declared_path = absolute_directories(&value, &declaration.span, "PATH")?,
                "LD_LIBRARY_PATH" => {
                    declared_library_path =
                        absolute_directories(&value, &declaration.span, "LD_LIBRARY_PATH")?
                }
                _ => entries.push((
                    OsString::from(key),
                    OsString::from(value),
                )),
            }
        }
        Ok(Self {
            commands,
            entries,
            declared_path,
            declared_library_path,
            closure,
        })
    }

    pub fn empty() -> Self {
        Self {
            commands: BTreeMap::new(),
            entries: Vec::new(),
            declared_path: Vec::new(),
            declared_library_path: Vec::new(),
            closure: BTreeSet::new(),
        }
    }

    pub fn closure(&self) -> &BTreeSet<PathBuf> {
        &self.closure
    }

    /// Persist literal contributions and an explicit reference to the profile's
    /// command projection. User values never acquire legacy shell expansion.
    pub fn profile_env(&self) -> Result<Vec<gripsack_store::EnvironmentContribution>, ExecError> {
        use gripsack_store::{EnvironmentContribution, StructuredEnvironment};
        let mut variables = Vec::new();
        for (name, paths) in [("PATH", &self.declared_path), ("LD_LIBRARY_PATH", &self.declared_library_path)] {
            if paths.is_empty() { continue; }
            let value = std::env::join_paths(paths).map_err(operational)?
                .into_string().map_err(|_| failure("profile path is not UTF-8"))?;
            variables.push(EnvironmentContribution::Structured(
                StructuredEnvironment::Literal { name:name.into(),op:gripsack_ir::EnvOp::Prepend,value },
            ));
        }
        if !self.commands.is_empty() {
            variables.push(EnvironmentContribution::Structured(
                StructuredEnvironment::StorePath { name:"PATH".into(),op:gripsack_ir::EnvOp::Prepend,path:"commands".into() },
            ));
        }
        for (key, value) in &self.entries {
            let name = key.to_str().ok_or_else(|| failure("profile variable name is not UTF-8"))?;
            if !crate::env::is_identifier(name) {
                return Err(failure("profile variable name is not a POSIX identifier"));
            }
            variables.push(EnvironmentContribution::Structured(
                StructuredEnvironment::Literal {
                    name:name.to_owned(),
                    op:gripsack_ir::EnvOp::Set,
                    value:value.to_str().ok_or_else(|| failure("profile variable value is not UTF-8"))?.to_owned(),
                },
            ));
        }
        Ok(variables)
    }

    /// Shared projection preserves the selected program's argv0, including a
    /// multicall selector exported under another name, and its exact runtime
    /// search plan. The public alias is never substituted for the selector.
    pub fn write_commands(&self, bin_directory: &Path) -> Result<(), ExecError> {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        for (name, command) in &self.commands {
            let destination = bin_directory.join(name);
            let libraries = std::env::join_paths(command.library_dirs.iter().chain(&self.declared_library_path)).map_err(operational)?;
            let libraries = libraries.to_str().ok_or_else(|| failure("library search path is not UTF-8"))?;
            let program = command.path.to_str().ok_or_else(|| failure("command path is not UTF-8"))?;
            let mut file = std::fs::File::create_new(destination).map_err(operational)?;
            writeln!(file,"#!/bin/sh").map_err(operational)?;
            if !libraries.is_empty() {
                write!(file,"LD_LIBRARY_PATH={} ",crate::env::ShellLiteral(libraries)).map_err(operational)?;
            }
            if let Some(loader) = command.gnu_loader {
                let subject = command.interpreter.as_ref().map_or(&command.path,|interpreter| &interpreter.path);
                let subject = subject.to_str().ok_or_else(|| failure("interpreter path is not UTF-8"))?;
                write!(file,"exec {} --inhibit-cache --glibc-hwcaps-mask '' --inhibit-rpath '' --library-path {} --argv0 {} {}",
                    crate::env::ShellLiteral(loader),crate::env::ShellLiteral(libraries),
                    crate::env::ShellLiteral(subject),crate::env::ShellLiteral(subject)).map_err(operational)?;
                if let Some(interpreter) = &command.interpreter {
                    if let Some(argument) = &interpreter.argument {
                        let argument = argument.to_str().ok_or_else(|| failure("interpreter argument is not UTF-8"))?;
                        write!(file," {}",crate::env::ShellLiteral(argument)).map_err(operational)?;
                    }
                    write!(file," {}",crate::env::ShellLiteral(program)).map_err(operational)?;
                }
            } else {
                write!(file,"exec {}",crate::env::ShellLiteral(program)).map_err(operational)?;
            }
            writeln!(file," \"$@\"").map_err(operational)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o755)).map_err(operational)?;
        }
        Ok(())
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
        EnvironmentOverlay::admit(self.entries.iter().cloned(), search, library)
            .map_err(operational)
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
            let canonical = path
                .canonicalize()
                .map_err(|_| gate(&declaration.span, format!("environment artifact {output:?}/{selector:?} does not resolve")))?;
            if !canonical.starts_with(&payload) {
                return Err(gate(&declaration.span, "environment artifact escapes its payload"));
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
        WorkspaceArg::PackageCommand { .. } | WorkspaceArg::Source { .. } | WorkspaceArg::Output { .. } => {
            return Err(gate(
                &declaration.span,
                "this binding kind is not environment data (sema E128)",
            ));
        }
    };
    Ok(value)
}

pub(super) fn artifact_payload(name: &str, realization: &Realization) -> Option<PathBuf> {
    if let Some(recipe) = realization.recipes.get(name) {
        return Some(recipe.payload.clone());
    }
    realization
        .packages
        .get(name)
        .map(|package| package.producer.payload.clone())
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
        module: "workspace".into(),
        step: "consumer".into(),
        detail: detail.into(),
    }
}

pub(super) fn operational(error: impl std::fmt::Display) -> ExecError {
    failure(&error.to_string())
}
