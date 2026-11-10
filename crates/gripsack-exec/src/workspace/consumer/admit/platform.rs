//! Narrow OS runtime authority for coherent Conda prefixes and packages with
//! explicit GNU host-runtime policy; never ld.so.cache, PATH or host packages.
use super::{BinaryAbi, ExecError, HostTarget, Span, elf, gate, operational, platform_loader};
use crate::workspace::conda::SystemRuntime;
use gripsack_ir::workspace_model::PlatformLibraries;
use gripsack_policy::target::{TargetArch, TargetOs};
use std::path::{Path, PathBuf};

/// Sonames conda-forge leaves to the host system rather than shipping in the
/// closure: its "Core Dependency Tree" (CDT) policy (conda-forge maintainer
/// knowledge base, <https://conda-forge.org/docs/maintainer/knowledge_base/>,
/// "Core Dependency Tree Packages") — X11 client libraries, OpenGL/EGL and
/// ALSA. conda-forge's `tk`, pulled in by every `python`, links
/// `libX11.so.6` this way (only a `run_constrained` on `xorg-libx11`).
/// Keep this list explicit and small: anything else stays in the closure.
const HOST_PLATFORM_LIBRARIES: &[&str] = &[
    // X11 client libraries.
    "libX11.so.6",
    "libX11-xcb.so.1",
    "libxcb.so.1",
    "libXau.so.6",
    "libXdmcp.so.6",
    "libXext.so.6",
    "libXrender.so.1",
    "libXft.so.2",
    "libXss.so.1",
    // OpenGL / EGL (the vendor-neutral dispatch libraries).
    "libGL.so.1",
    "libEGL.so.1",
    "libGLX.so.0",
    "libOpenGL.so.0",
    "libGLdispatch.so.0",
    // ALSA.
    "libasound.so.2",
];

/// The measured platform libraries a prefix takes from the host under its
/// policy; empty under `strict: true`, where admission refuses them instead.
pub(in crate::workspace) fn host_provided(
    system: &SystemRuntime,
    policy: PlatformLibraries,
) -> impl Iterator<Item = &str> {
    system
        .libraries
        .iter()
        .map(String::as_str)
        .filter(move |name| policy.is_host_provided() && HOST_PLATFORM_LIBRARIES.contains(name))
}

pub(super) fn admit(
    system: &SystemRuntime,
    policy: PlatformLibraries,
    host: &HostTarget,
    span: &Span,
) -> Result<(), ExecError> {
    for loader in &system.loaders {
        let expected = host
            .abi
            .and_then(|abi| platform_loader(host.os, host.arch, abi));
        if expected != Some(loader.as_str()) {
            return Err(gate(
                span,
                format!("Conda prefix requires incompatible platform loader {loader:?}"),
            ));
        }
    }
    for name in &system.libraries {
        let platform_library = HOST_PLATFORM_LIBRARIES.contains(&name.as_str());
        if platform_library && policy.is_host_provided() {
            // Loaded by the platform loader from the host when an optional
            // module (Tk, say) is imported; a missing one fails that import.
            continue;
        }
        let path = library(name, host, span)?.ok_or_else(|| {
            gate(
                span,
                if platform_library {
                    format!(
                        "Conda prefix requires host platform library {name:?} and declares \
                         strict: true; include that library in the frozen package closure \
                         or drop strict"
                    )
                } else {
                    format!(
                        "Conda prefix requires {name:?} outside the explicit OS runtime; \
                         include that library in the frozen package closure"
                    )
                },
            )
        })?;
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
    if host.os != TargetOs::Linux
        || host.arch != TargetArch::X86_64
        || host.abi != Some(BinaryAbi::Gnu)
    {
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
    let directories = [
        "/lib/x86_64-linux-gnu",
        "/usr/lib/x86_64-linux-gnu",
        "/lib64",
        "/usr/lib64",
        "/lib",
        "/usr/lib",
    ];
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
