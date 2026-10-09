//! Narrow OS runtime authority for coherent Conda prefixes and packages with
//! explicit GNU host-runtime policy; never ld.so.cache, PATH or host packages.
use super::{BinaryAbi, ExecError, HostTarget, Span, elf, gate, operational, platform_loader};
use crate::workspace::conda::SystemRuntime;
use gripsack_policy::target::{TargetArch, TargetOs};
use std::path::{Path, PathBuf};

pub(super) fn admit(
    system: &SystemRuntime,
    host: &HostTarget,
    span: &Span,
) -> Result<(), ExecError> {
    for loader in &system.loaders {
        let expected = match (host.os, host.abi) {
            (TargetOs::Macos, Some(BinaryAbi::Darwin)) => Some("/usr/lib/dyld"),
            (_, Some(abi)) => platform_loader(host.os, host.arch, abi),
            _ => None,
        };
        if expected != Some(loader.as_str()) {
            return Err(gate(
                span,
                format!("Conda prefix requires incompatible platform loader {loader:?}"),
            ));
        }
    }
    for name in &system.libraries {
        if host.os == TargetOs::Macos
            && host.abi == Some(BinaryAbi::Darwin)
            && super::macho::system_library(name)
        {
            // Modern macOS system libraries live in the dyld shared cache;
            // a nonexistent on-disk dylib is not evidence that they are absent.
            continue;
        }
        let path=library(name,host,span)?.ok_or_else(|| gate(span,format!(
            "Conda prefix requires {name:?} outside the explicit OS runtime; include that library in the frozen package closure"
        )))?;
        let mut file = std::fs::File::open(&path.canonical).map_err(operational)?;
        let status = file.metadata().map_err(operational)?;
        if !status.is_file() || status.len() > super::EXECUTABLE_BYTES {
            return Err(gate(
                span,
                "platform runtime library is not a bounded regular file",
            ));
        }
        let metadata = gripsack_process::executable::classify(&mut file).map_err(operational)?;
        elf::header(&metadata, host, false, span)?;
    }
    Ok(())
}

pub(super) fn library(
    name: &str,
    host: &HostTarget,
    span: &Span,
) -> Result<Option<elf::SelectedLibrary>, ExecError> {
    if host.os != TargetOs::Linux || host.abi != Some(BinaryAbi::Gnu) {
        return Ok(None);
    }
    let loader = platform_loader(host.os, host.arch, BinaryAbi::Gnu).expect("supported GNU target");
    if Path::new(loader)
        .file_name()
        .is_some_and(|base| base == name)
    {
        return elf::SelectedLibrary::new(PathBuf::from(loader))
            .map(Some)
            .map_err(operational);
    }
    // These public SONAMEs belong to glibc. X11, OpenSSL, libstdc++, GPU
    // drivers and other independent packages never inherit this authority.
    if !matches!(
        name,
        "libc.so.6"
            | "libm.so.6"
            | "libdl.so.2"
            | "libpthread.so.0"
            | "librt.so.1"
            | "libutil.so.1"
            | "libresolv.so.2"
            | "libanl.so.1"
    ) {
        return Ok(None);
    }
    let directories = match host.arch {
        TargetArch::X86_64 => [
            "/lib/x86_64-linux-gnu",
            "/usr/lib/x86_64-linux-gnu",
            "/lib64",
            "/usr/lib64",
            "/lib",
            "/usr/lib",
        ],
        TargetArch::Aarch64 => [
            "/lib/aarch64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
            "/lib64",
            "/usr/lib64",
            "/lib",
            "/usr/lib",
        ],
    };
    for directory in directories {
        match elf::SelectedLibrary::new(Path::new(directory).join(name)) {
            Ok(path) => return Ok(Some(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(operational(error)),
        }
    }
    Err(gate(
        span,
        format!(
            "required GNU system runtime {name:?} is absent from the explicit platform directories"
        ),
    ))
}
