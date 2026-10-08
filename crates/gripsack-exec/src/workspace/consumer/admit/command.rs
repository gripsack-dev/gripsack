//! Export receipts, executable byte binding and retained script interpreters.
use super::{
    AdmittedCommand, BinaryAbi, EXECUTABLE_BYTES, ExecError, NativeContext, Package,
    PinnedInterpreter, Span, elf, gate, macho, operational,
};
use gripsack_ir::workspace_v6::identity::ExecutableDigest;
use gripsack_process::{
    Sha256Digest,
    executable::{ExecutableFormat, Interpreter, classify},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Read, Seek},
    path::Path,
};

/// Verify one exported package command against its publication receipt:
/// the bytes must hash to the recorded digest and the layout must be one of
/// the declared supported native runtime layouts. The user's relocatable
/// layout tag is never proof on its own.
pub(in crate::workspace::consumer) fn admit_command(
    package: &Package,
    selector: &str,
    executable: &ExecutableDigest,
    context: &NativeContext<'_>,
    span: &Span,
) -> Result<AdmittedCommand, ExecError> {
    let bytecode = context.admit_package_closure(package, span)?;
    let host = &context.target;
    let declared_abi = package.target.abi.as_ref().map(|abi| match abi {
        gripsack_ir::workspace::PlatformAbi::Gnu => BinaryAbi::Gnu,
        gripsack_ir::workspace::PlatformAbi::Musl => BinaryAbi::Musl,
        gripsack_ir::workspace::PlatformAbi::Darwin => BinaryAbi::Darwin,
    });
    let payload = package
        .producer
        .payload
        .canonicalize()
        .map_err(operational)?;
    let path = payload.join(selector);
    let canonical = path.canonicalize().map_err(|error| {
        gate(
            span,
            format!("exported command {selector:?} is unreadable: {error}"),
        )
    })?;
    if !canonical.starts_with(&payload) {
        return Err(gate(span, "exported command escapes its package payload"));
    }
    let (actual, metadata) = read_executable(&canonical, span)?;
    if actual != Sha256Digest::from_bytes(*executable.bytes()) {
        return Err(gate(
            span,
            "exported command bytes differ from their publication receipt",
        ));
    }
    let runtime = match metadata.format {
        Some(ExecutableFormat::Script) => admit_script(&metadata, package, context, span)?,
        Some(ExecutableFormat::Elf) => CommandRuntime {
            elf: elf::admit(
                &metadata,
                &canonical,
                &payload,
                package,
                declared_abi,
                host,
                span,
            )?,
            ..CommandRuntime::default()
        },
        Some(ExecutableFormat::MachO) => CommandRuntime {
            macho_library_dirs: Some(macho::admit(
                &metadata,
                &canonical,
                package,
                declared_abi,
                host,
                span,
            )?),
            ..CommandRuntime::default()
        },
        None => return Err(gate(span, "unrecognized executable format")),
    };
    Ok(AdmittedCommand {
        path,
        executable: *executable,
        library_dirs: runtime.elf.directories,
        gnu_loader: runtime
            .elf
            .gnu_loader
            .map(|path| context.gnu_loader(path, span))
            .transpose()?,
        macho_library_dirs: runtime.macho_library_dirs,
        interpreter: runtime.interpreter,
        bytecode,
    })
}

#[derive(Default)]
struct CommandRuntime {
    elf: elf::Runtime,
    interpreter: Option<PinnedInterpreter>,
    macho_library_dirs: Option<Vec<std::path::PathBuf>>,
}

fn admit_script(
    metadata: &gripsack_process::executable::ExecutableMetadata,
    package: &Package,
    context: &NativeContext<'_>,
    span: &Span,
) -> Result<CommandRuntime, ExecError> {
    let host = &context.target;
    let Some(Interpreter::Shebang { program, argument }) = &metadata.interpreter else {
        return Err(gate(span, "script command has no interpreter line"));
    };
    let program = Path::new(program);
    if !program.is_absolute() {
        return Err(gate(span, "script interpreter must be absolute"));
    }
    let canonical = program.canonicalize().map_err(|error| {
        gate(
            span,
            format!("script interpreter does not resolve: {error}"),
        )
    })?;
    let mut packages = vec![package];
    let mut visited = BTreeSet::new();
    let mut index = 0;
    while index < packages.len() {
        let owner = packages[index];
        index += 1;
        if !visited.insert(&owner.root) {
            continue;
        }
        let payload = owner.producer.payload.canonicalize().map_err(operational)?;
        if canonical.starts_with(&payload) {
            context.admit_package_closure(owner, span)?;
            let (executable, metadata) = read_executable(&canonical, span)?;
            let abi = owner.target.abi.as_ref().map(|abi| match abi {
                gripsack_ir::workspace::PlatformAbi::Gnu => BinaryAbi::Gnu,
                gripsack_ir::workspace::PlatformAbi::Musl => BinaryAbi::Musl,
                gripsack_ir::workspace::PlatformAbi::Darwin => BinaryAbi::Darwin,
            });
            let (runtime, macho_library_dirs) = match metadata.format {
                Some(ExecutableFormat::Elf) => (
                    elf::admit(&metadata, &canonical, &payload, owner, abi, host, span)?,
                    None,
                ),
                Some(ExecutableFormat::MachO) => (
                    elf::Runtime::default(),
                    Some(macho::admit(&metadata, &canonical, owner, abi, host, span)?),
                ),
                _ => {
                    return Err(gate(
                        span,
                        "pinned script interpreter is not an admitted native binary",
                    ));
                }
            };
            return Ok(CommandRuntime {
                elf: runtime,
                interpreter: Some(PinnedInterpreter {
                    path: program.to_owned(),
                    argument: argument.clone(),
                    executable,
                }),
                macho_library_dirs,
            });
        }
        packages.extend(owner.runtime.iter().map(std::sync::Arc::as_ref));
    }
    let store = package
        .root
        .parent()
        .ok_or_else(|| gate(span, "package has no store namespace"))?
        .canonicalize()
        .map_err(operational)?;
    if canonical.starts_with(store) {
        return Err(gate(
            span,
            "script interpreter is an undeclared store dependency",
        ));
    }
    // An absolute platform interpreter remains explicit host authority. Only
    // retained package interpreters carry package-closure and byte-pin claims.
    Ok(CommandRuntime::default())
}

fn read_executable(
    path: &Path,
    span: &Span,
) -> Result<
    (
        Sha256Digest,
        gripsack_process::executable::ExecutableMetadata,
    ),
    ExecError,
> {
    use std::os::unix::fs::PermissionsExt;
    let mut file = std::fs::File::open(path).map_err(operational)?;
    let status = file.metadata().map_err(operational)?;
    if !status.is_file() || status.permissions().mode() & 0o111 == 0 {
        return Err(gate(
            span,
            "runtime program is not an executable regular file",
        ));
    }
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(operational)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .filter(|total| *total <= EXECUTABLE_BYTES)
            .ok_or_else(|| gate(span, "executable exceeds its byte budget"))?;
        hash.update(&buffer[..count]);
    }
    file.rewind().map_err(operational)?;
    let metadata = classify(&mut file)
        .map_err(|error| gate(span, format!("unsupported executable layout: {error}")))?;
    Ok((Sha256Digest::from_bytes(hash.finalize().into()), metadata))
}
