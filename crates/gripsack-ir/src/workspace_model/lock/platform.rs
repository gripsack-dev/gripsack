use super::WorkspaceLockError;
use crate::workspace::{OsVersion, PlatformAbi, PlatformArch, PlatformOs, WorkspacePlatform};

/// Omitted patch and explicit zero denote the same platform partition.
pub fn platform_key(platform: &WorkspacePlatform) -> String {
    let os = match platform.os {
        PlatformOs::Linux => "linux",
        PlatformOs::Macos => "macos",
    };
    let arch = match platform.arch {
        PlatformArch::X86_64 => "x86_64",
        PlatformArch::Aarch64 => "aarch64",
    };
    let mut key = format!("{os}-{arch}");
    if let Some(abi) = platform.abi {
        key.push_str(match abi {
            PlatformAbi::Gnu => "-gnu",
            PlatformAbi::Musl => "-musl",
            PlatformAbi::Darwin => "-darwin",
        });
    }
    if let Some(version) = platform.minimum_os {
        use std::fmt::Write;
        write!(
            key,
            "@{}.{}.{}",
            version.major,
            version.minor,
            version.patch.unwrap_or(0)
        )
        .expect("String writes are infallible");
    }
    key
}

pub fn parse_platform_key(text: &str) -> Result<WorkspacePlatform, WorkspaceLockError> {
    let invalid = || WorkspaceLockError::InvalidPin(format!("noncanonical platform key {text:?}"));
    let (base, version) = text
        .split_once('@')
        .map_or((text, None), |(base, version)| (base, Some(version)));
    let mut fields = base.split('-');
    let os = match fields.next() {
        Some("linux") => PlatformOs::Linux,
        Some("macos") => PlatformOs::Macos,
        _ => return Err(invalid()),
    };
    let arch = match fields.next() {
        Some("x86_64") => PlatformArch::X86_64,
        Some("aarch64") => PlatformArch::Aarch64,
        _ => return Err(invalid()),
    };
    let abi = match fields.next() {
        None => None,
        Some("gnu") => Some(PlatformAbi::Gnu),
        Some("musl") => Some(PlatformAbi::Musl),
        Some("darwin") => Some(PlatformAbi::Darwin),
        _ => return Err(invalid()),
    };
    if fields.next().is_some() {
        return Err(invalid());
    }
    let minimum_os = if let Some(version) = version {
        let mut fields = version.split('.');
        let mut component = || {
            fields
                .next()
                .ok_or_else(invalid)?
                .parse::<u16>()
                .map_err(|_| invalid())
        };
        let value = OsVersion {
            major: component()?,
            minor: component()?,
            patch: Some(component()?),
        };
        if fields.next().is_some() {
            return Err(invalid());
        }
        Some(value)
    } else {
        None
    };
    let platform = WorkspacePlatform {
        os,
        arch,
        abi,
        minimum_os,
    };
    if !platform.valid_abi() || platform_key(&platform) != text {
        return Err(invalid());
    }
    Ok(platform)
}
