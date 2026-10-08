//! Image facts come from independently verified rootfs bytes, never the host or
//! the virtual-package values recorded by a solver. Kernel/GPU/CPU remain runtime
//! obligations and are reported, not manufactured as successful measurements.
use super::{failure, plan::Placement};
use crate::ExecError;
use gripsack_buildkit::oci::{OciLimits, ValidatedImage};
use gripsack_ir::workspace_model::{ImageOutput, lock::LockedVirtualPackage};
use std::{collections::BTreeSet, fs::File};

pub(super) fn validate(
    image: &ImageOutput,
    placements: &[Placement<'_, '_>],
    actual: &ValidatedImage,
    archive: &mut File,
    limits: OciLimits,
) -> Result<Vec<String>, ExecError> {
    let mut external = BTreeSet::new();
    if let Some(version) = image.target.minimum_os {
        external.insert(format!(
            "runtime Linux kernel >= {}.{}.{}",
            version.major,
            version.minor,
            version.patch.unwrap_or(0)
        ));
    }
    if !placements.iter().any(|placement| placement.conda.is_some()) {
        return Ok(external.into_iter().collect());
    }
    let mut version = None;
    for path in actual
        .files()
        .keys()
        .filter(|path| path.ends_with("/libc.so.6"))
    {
        let bytes = actual
            .read_file(archive, path, 16 * 1024 * 1024, limits)
            .map_err(|error| failure(image, error))?;
        let measured = glibc_release(&bytes).ok_or_else(|| {
            failure(
                image,
                format!("cannot establish GNU libc version from verified image file {path:?}"),
            )
        })?;
        if version.as_ref().is_some_and(|version| version != &measured) {
            return Err(failure(
                image,
                "Conda image contains conflicting GNU libc releases",
            ));
        }
        version = Some(measured);
    }
    let unix = LockedVirtualPackage {
        name: "__unix".into(),
        version: "0".into(),
        build: "0".into(),
    };
    let measured;
    let facts = if let Some(version) = version {
        measured = [
            unix,
            LockedVirtualPackage {
                name: "__glibc".into(),
                version,
                build: "0".into(),
            },
        ];
        measured.as_slice()
    } else {
        std::slice::from_ref(&unix)
    };
    for placement in placements {
        let Some(receipt) = placement.conda else {
            continue;
        };
        for requirement in
            gripsack_conda::virtuals::evaluate_image_constraints(&receipt.closure.packages, facts)
                .map_err(|error| failure(image, error))?
        {
            external.insert(format!("{}: {requirement}", placement.name));
        }
        let requirements = &receipt.closure.system_requirements;
        for required in &requirements.virtual_packages {
            if matches!(
                required.name.as_str(),
                "__glibc" | "__unix" | "__osx" | "__win"
            ) {
                let measured = gripsack_ir::workspace_model::lock::LockedCondaSystemRequirements {
                    virtual_packages: vec![required.clone()],
                    archspec: None,
                };
                gripsack_conda::virtuals::evaluate_system_requirements(&measured, facts)
                    .map_err(|error| failure(image, error))?;
            } else {
                external.insert(format!(
                    "{}: runtime {} >= {} build {:?}",
                    placement.name, required.name, required.minimum_version, required.build
                ));
            }
        }
        if let Some(architecture) = &requirements.archspec {
            external.insert(format!(
                "{}: runtime CPU supports {architecture}",
                placement.name
            ));
        }
    }
    Ok(external.into_iter().collect())
}

fn glibc_release(bytes: &[u8]) -> Option<String> {
    const MARKER: &[u8] = b"release version ";
    if !bytes
        .windows(b"GNU C Library".len())
        .any(|part| part == b"GNU C Library")
    {
        return None;
    }
    let start = bytes
        .windows(MARKER.len())
        .position(|part| part == MARKER)?
        + MARKER.len();
    let rest = &bytes[start..];
    let end = rest
        .iter()
        .position(|byte| !byte.is_ascii_digit() && *byte != b'.')?;
    let version = std::str::from_utf8(&rest[..end])
        .ok()?
        .trim_end_matches('.');
    let mut fields = version.split('.');
    if fields.next()?.is_empty()
        || fields.next()?.is_empty()
        || fields.any(|field| field.is_empty())
    {
        return None;
    }
    Some(version.to_owned())
}
