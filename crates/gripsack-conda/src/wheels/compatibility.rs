//! Shared frozen CPython/Linux wheel compatibility; no ambient interpreter or
//! historical solve fact can grant a newer native ABI than the declared floor.

use std::path::PathBuf;
use std::str::FromStr;

use gripsack_ir::workspace_model::lock::{LockedCondaEnvironment, LockedWheel};
use pep508_rs::pep440_rs::{Version, VersionSpecifiers};
use uv_distribution_filename::WheelFilename;
use uv_platform_tags::{Arch, Os, Platform, Tags, TagsOptions};

use super::{WheelError, invalid};

/// Interpreter facts derived only from the admitted Conda closure.
pub struct PythonTarget {
    pub version: Version,
    pub short_version: (u8, u8),
    pub site_packages: PathBuf,
    has_libc_floor: bool,
    tags: Tags,
}

impl PythonTarget {
    pub fn from_lock(closure: &LockedCondaEnvironment) -> Result<Self, WheelError> {
        if closure.platform != "linux-64" {
            return Err(invalid("wheels require the admitted Linux x86_64 target"));
        }
        let python = closure
            .packages
            .iter()
            .find(|package| package.name == "python")
            .ok_or_else(|| invalid("wheels require a locked Conda Python interpreter"))?;
        if python.build.contains("pypy")
            || closure
                .packages
                .iter()
                .any(|package| package.name == "pypy")
        {
            return Err(invalid(
                "wheel ABI admission requires locked CPython, not PyPy",
            ));
        }
        let version = Version::from_str(&python.version).map_err(invalid)?;
        let release = version.release();
        let (Some(&major), Some(&minor)) = (release.first(), release.get(1)) else {
            return Err(invalid(
                "locked Python must identify major and minor versions",
            ));
        };
        let short_version = (
            u8::try_from(major).map_err(invalid)?,
            u8::try_from(minor).map_err(invalid)?,
        );
        if major != 3 {
            return Err(invalid("wheel admission requires Python 3"));
        }
        let abi_record = closure
            .packages
            .iter()
            .find(|package| package.name == "python_abi");
        let abi_build = abi_record.map_or(python.build.as_str(), |package| package.build.as_str());
        let abi = abi_build.split('_').find(|part| {
            part.starts_with("cp") && part.as_bytes().get(2).is_some_and(u8::is_ascii_digit)
        });
        let (gil_disabled, debug_enabled) = match abi {
            Some(abi) => {
                let expected = format!("cp{major}{minor}");
                let flags = abi
                    .strip_prefix(&expected)
                    .ok_or_else(|| invalid("locked Python and python_abi versions disagree"))?;
                if !flags.bytes().all(|flag| matches!(flag, b't' | b'd' | b'm')) {
                    return Err(invalid("unrecognized locked CPython ABI flags"));
                }
                (flags.contains('t'), flags.contains('d'))
            }
            None if abi_record.is_some() => {
                return Err(invalid("python_abi does not identify a CPython ABI"));
            }
            None => {
                if python.build.contains("freethread") || python.build.contains("debug") {
                    return Err(invalid(
                        "nonstandard CPython requires an explicit python_abi record",
                    ));
                }
                (false, false)
            }
        };
        let glibc = closure
            .system_requirements
            .virtual_packages
            .iter()
            .find(|package| package.name == "__glibc")
            .map(|package| package.minimum_version.as_str());
        let (glibc_major, glibc_minor) = match glibc {
            Some(value) => {
                let value = Version::from_str(value).map_err(invalid)?;
                let release = value.release();
                let (Some(&major), Some(&minor)) = (release.first(), release.get(1)) else {
                    return Err(invalid(
                        "glibc wheel target requires major and minor versions",
                    ));
                };
                (
                    u16::try_from(major).map_err(invalid)?,
                    u16::try_from(minor).map_err(invalid)?,
                )
            }
            None => (2, 0),
        };
        let tags = Tags::from_env(
            Platform::new(
                Os::Manylinux {
                    major: glibc_major,
                    minor: glibc_minor,
                },
                Arch::X86_64,
            ),
            short_version,
            "cpython",
            short_version,
            TagsOptions {
                manylinux_compatible: glibc.is_some(),
                gil_disabled,
                debug_enabled,
                is_cross: true,
            },
        )
        .map_err(invalid)?;
        let site_packages = python
            .python_site_packages_path
            .clone()
            .unwrap_or_else(|| format!("lib/python{major}.{minor}/site-packages"));
        if site_packages
            .split('/')
            .any(|part| matches!(part, "" | "." | ".."))
            || site_packages.contains('\\')
        {
            return Err(invalid("unsafe Python site-packages path"));
        }
        Ok(Self {
            version,
            short_version,
            site_packages: PathBuf::from(site_packages),
            tags,
            has_libc_floor: glibc.is_some(),
        })
    }

    pub fn admit(&self, wheel: &LockedWheel) -> Result<WheelFilename, WheelError> {
        let filename = WheelFilename::from_str(&wheel.filename).map_err(invalid)?;
        if filename.name.as_ref() != wheel.name || filename.version.to_string() != wheel.version {
            return Err(invalid(format!(
                "wheel '{}' filename identity differs from lock",
                wheel.filename
            )));
        }
        if !self.has_libc_floor
            && filename
                .platform_tags()
                .iter()
                .any(|tag| tag.to_string() != "any")
        {
            return Err(invalid(
                "binary Linux wheels require an explicit __glibc system requirement",
            ));
        }
        if !filename.is_compatible(&self.tags) {
            return Err(invalid(format!(
                "wheel '{}' is incompatible with locked Python/Linux ABI",
                wheel.filename
            )));
        }
        if let Some(requires_python) = &wheel.requires_python {
            let specifiers = VersionSpecifiers::from_str(requires_python).map_err(invalid)?;
            if !specifiers.contains(&self.version) {
                return Err(invalid(format!(
                    "wheel '{}' requires Python {requires_python}, locked {}",
                    wheel.name, self.version
                )));
            }
        }
        for dependency in &wheel.requires_dist {
            pep508_rs::Requirement::<pep508_rs::VerbatimUrl>::from_str(dependency)
                .map_err(invalid)?;
        }
        Ok(filename)
    }
}

/// Importers can reject incompatible frozen metadata without downloading or
/// solving. Actual distribution ownership is checked against archive metadata
/// before materialization, not inferred from similar Conda package names.
pub fn validate_lock(closure: &LockedCondaEnvironment) -> Result<(), WheelError> {
    if closure.wheels.is_empty() {
        return Ok(());
    }
    let target = PythonTarget::from_lock(closure)?;
    for wheel in &closure.wheels {
        target.admit(wheel)?;
    }
    Ok(())
}
