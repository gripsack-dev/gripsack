//! Mach-O admission does not mask signatures or accept a helper's signed
//! region. On macOS reconstruct the patched original privately, independently
//! ad-hoc sign it with the native tool, then compare the entire resulting file.
//! Linux cannot qualify this native path and reports the missing capability.
use super::inventory::PrefixPatch;
use std::fs::File;
use std::io::{Read, Write};

pub(super) fn is_macho(source: &[u8]) -> bool {
    source.get(..4).is_some_and(|magic| {
        matches!(
            magic,
            [0xca, 0xfe, 0xba, 0xbe]
                | [0xbe, 0xba, 0xfe, 0xca]
                | [0xfe, 0xed, 0xfa, 0xce]
                | [0xce, 0xfa, 0xed, 0xfe]
                | [0xfe, 0xed, 0xfa, 0xcf]
                | [0xcf, 0xfa, 0xed, 0xfe]
        )
    })
}

pub(super) fn required(source: &[u8], mode: u32, patch: &PrefixPatch, platform: &str) -> bool {
    platform.starts_with("osx-")
        && patch.binary
        && (is_macho(source) || mode & 0o111 != 0)
        && memchr::memmem::find(source, patch.placeholder.as_bytes()).is_some()
}

pub(super) fn verify(
    original: &[u8],
    actual: &mut File,
    relative: &str,
    patch: &PrefixPatch,
    prefix: &str,
    platform: &str,
) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("native macOS /usr/bin/codesign is required to independently validate relocated Mach-O signing; this host cannot qualify that capability".into());
    }
    let scratch = tempfile::tempdir().map_err(|error| error.to_string())?;
    let name = std::path::Path::new(relative)
        .file_name()
        .ok_or("missing signed payload name")?;
    let path = scratch.path().join(name);
    {
        let file = File::create(&path).map_err(|error| error.to_string())?;
        let mut output = std::io::BufWriter::new(file);
        super::super::prefix::emit(
            original,
            &mut output,
            &patch.placeholder,
            prefix,
            true,
            platform,
        )?;
        output.flush().map_err(|error| error.to_string())?;
    }
    sign(&path, scratch.path())?;
    let mut reconstructed = File::open(&path).map_err(|error| error.to_string())?;
    let mut compare = super::super::prefix::Comparison(actual);
    std::io::copy(&mut reconstructed, &mut compare).map_err(|error| error.to_string())?;
    let mut extra = [0];
    if compare
        .0
        .read(&mut extra)
        .map_err(|error| error.to_string())?
        != 0
    {
        return Err("signed payload has extra bytes beyond independently signed original".into());
    }
    Ok(())
}

fn sign(path: &std::path::Path, directory: &std::path::Path) -> Result<(), String> {
    use gripsack_process::{
        Control, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessRole, SelectedProgram,
    };
    // Codesigning one bounded payload gets a finite process/output budget. No
    // unbounded native subprocess or inherited operator shell is used here.
    let timeout = std::time::Duration::from_secs(60);
    let deadline = std::time::Instant::now() + timeout;
    let environment = OperatorEnvironment::capture().map_err(|error| error.to_string())?;
    let program = SelectedProgram::select(
        &environment,
        std::path::Path::new("/usr/bin/codesign"),
        None,
        deadline,
    )
    .map_err(|error| format!("native signing capability unavailable: {error}"))?;
    let invocation = Invocation::admit(
        &environment,
        ProcessRole::Plugin,
        &program,
        directory,
        Limits {
            timeout,
            operation_deadline: Some(deadline),
            ..Limits::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let arguments = [
        std::ffi::OsStr::new("--sign"),
        std::ffi::OsStr::new("-"),
        std::ffi::OsStr::new("--force"),
        std::ffi::OsStr::new("--preserve-metadata=entitlements"),
        path.as_os_str(),
    ];
    let outcome = invocation
        .run(&arguments, NativeInput::Bytes(&[]), None, |_| {
            Control::Continue
        })
        .map_err(|error| error.to_string())?;
    if !outcome.success {
        return Err(format!(
            "independent native codesign failed: {}",
            String::from_utf8_lossy(&outcome.stderr)
        ));
    }
    Ok(())
}
