//! Export receipts, executable byte binding and retained script interpreters.
use super::{AdmittedCommand, BinaryAbi, EXECUTABLE_BYTES, ExecError, HostTarget, Package, PinnedInterpreter, Span, admit_macho, admit_target, elf, gate, operational};
use gripsack_ir::workspace_v6::identity::ExecutableDigest;
use gripsack_process::{Sha256Digest,executable::{ExecutableFormat,Interpreter,classify}};
use sha2::{Digest,Sha256};
use std::{collections::BTreeSet,io::{Read,Seek},path::Path};

/// Verify one exported package command against its publication receipt:
/// the bytes must hash to the recorded digest and the layout must be one of
/// the declared supported native runtime layouts. The user's relocatable
/// layout tag is never proof on its own.
pub(in crate::workspace::consumer) fn admit_command(
    package: &Package,
    selector: &str,
    executable: &ExecutableDigest,
    host: &HostTarget,
    span: &Span,
) -> Result<AdmittedCommand, ExecError> {
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
    let canonical = path
        .canonicalize()
        .map_err(|error| gate(span, format!("exported command {selector:?} is unreadable: {error}")))?;
    if !canonical.starts_with(&payload) {
        return Err(gate(span, "exported command escapes its package payload"));
    }
    let (actual, metadata) = read_executable(&canonical,span)?;
    if actual != Sha256Digest::from_bytes(*executable.bytes())
    {
        return Err(gate(
            span,
            "exported command bytes differ from their publication receipt",
        ));
    }
    let (runtime, interpreter) = match metadata.format {
        Some(ExecutableFormat::Script) => admit_script(&metadata, package, host, span)?,
        Some(ExecutableFormat::Elf) => (
            elf::admit(&metadata, &canonical, &payload, package, declared_abi, host, span)?,None,
        ),
        Some(ExecutableFormat::MachO) => (elf::Runtime {
            directories:admit_macho(&metadata, &payload, host, span)?,gnu_loader:None,
        },None),
        None => return Err(gate(span, "unrecognized executable format")),
    };
    Ok(AdmittedCommand {
        path,
        executable: *executable,
        library_dirs:runtime.directories,
        gnu_loader:runtime.gnu_loader,
        interpreter,
    })
}

fn admit_script(
    metadata: &gripsack_process::executable::ExecutableMetadata,
    package: &Package,
    host: &HostTarget,
    span: &Span,
) -> Result<(elf::Runtime, Option<PinnedInterpreter>), ExecError> {
    let Some(Interpreter::Shebang { program, argument }) = &metadata.interpreter else {
        return Err(gate(span, "script command has no interpreter line"));
    };
    let program = Path::new(program);
    if !program.is_absolute() { return Err(gate(span,"script interpreter must be absolute")); }
    let canonical = program.canonicalize()
        .map_err(|error| gate(span,format!("script interpreter does not resolve: {error}")))?;
    let mut packages = vec![package];
    let mut visited = BTreeSet::new();
    let mut index = 0;
    while index < packages.len() {
        let owner = packages[index]; index += 1;
        if !visited.insert(&owner.root) { continue; }
        let payload = owner.producer.payload.canonicalize().map_err(operational)?;
        if canonical.starts_with(&payload) {
            admit_target(&owner.target,host,span,"script interpreter")?;
            let (executable,metadata) = read_executable(&canonical,span)?;
            let abi = owner.target.abi.as_ref().map(|abi| match abi {
                gripsack_ir::workspace::PlatformAbi::Gnu => BinaryAbi::Gnu,
                gripsack_ir::workspace::PlatformAbi::Musl => BinaryAbi::Musl,
                gripsack_ir::workspace::PlatformAbi::Darwin => BinaryAbi::Darwin,
            });
            let runtime = match metadata.format {
                Some(ExecutableFormat::Elf) => elf::admit(&metadata,&canonical,&payload,owner,abi,host,span)?,
                Some(ExecutableFormat::MachO) => elf::Runtime {
                    directories:admit_macho(&metadata,&payload,host,span)?,gnu_loader:None,
                },
                _ => return Err(gate(span,"pinned script interpreter is not an admitted native binary")),
            };
            return Ok((runtime,Some(PinnedInterpreter {
                path:program.to_owned(),argument:argument.clone(),executable,
            })));
        }
        packages.extend(owner.runtime.iter().map(std::sync::Arc::as_ref));
    }
    let store = package.root.parent().ok_or_else(|| gate(span,"package has no store namespace"))?
        .canonicalize().map_err(operational)?;
    if canonical.starts_with(store) {
        return Err(gate(span,"script interpreter is an undeclared store dependency"));
    }
    // An absolute platform interpreter remains explicit host authority. Only
    // retained package interpreters carry package-closure and byte-pin claims.
    Ok((elf::Runtime::default(),None))
}

fn read_executable(path: &Path,span: &Span) -> Result<(Sha256Digest,gripsack_process::executable::ExecutableMetadata),ExecError> {
    use std::os::unix::fs::PermissionsExt;
    let mut file = std::fs::File::open(path).map_err(operational)?;
    let status = file.metadata().map_err(operational)?;
    if !status.is_file() || status.permissions().mode() & 0o111 == 0 {
        return Err(gate(span,"runtime program is not an executable regular file"));
    }
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8;64*1024];
    loop {
        let count = file.read(&mut buffer).map_err(operational)?;
        if count == 0 { break; }
        total = total.checked_add(count as u64).filter(|total| *total <= EXECUTABLE_BYTES)
            .ok_or_else(|| gate(span,"executable exceeds its byte budget"))?;
        hash.update(&buffer[..count]);
    }
    file.rewind().map_err(operational)?;
    let metadata = classify(&mut file).map_err(|error| gate(span,format!("unsupported executable layout: {error}")))?;
    Ok((Sha256Digest::from_bytes(hash.finalize().into()),metadata))
}
