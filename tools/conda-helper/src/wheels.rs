//! Frozen wheels: all archive, dependency, and ownership admission precedes
//! prefix effects. The maintained uv installer owns packaging transformations;
//! independent core inventory owns their expected bytes and normalized receipt.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use gripsack_conda::protocol::{ArchiveRef, MaterializedWheel};
use gripsack_conda::wheels::{ExpectedWheelFile, PythonTarget, WheelArchive};
use gripsack_ir::workspace_model::lock::{LockedCondaEnvironment, LockedWheel};
use rattler::install::PythonInfo;
use rattler_conda_types::package::PathsJson;
use uv_distribution_filename::WheelFilename;
use uv_install_wheel::{InstallState, Layout, LinkMode};
use uv_pypi_types::Scheme;

use super::{HResult, PreparedPackage, fail};

fn invalid(message: impl std::fmt::Display) -> super::Failure {
    fail("invalid_wheel", message.to_string())
}

pub(super) fn validate_lock(closure: &LockedCondaEnvironment) -> HResult<()> {
    gripsack_conda::wheels::validate_lock(closure).map_err(invalid)
}

struct PreparedWheel<'a> {
    record: &'a LockedWheel,
    archive: WheelArchive,
    filename: WheelFilename,
    inventory: Vec<ExpectedWheelFile>,
}

pub(super) struct PreparedWheels<'a> {
    target: Option<PythonTarget>,
    wheels: Vec<PreparedWheel<'a>>,
}

/// Original Conda payload paths and their admitted noarch relocation policy.
pub(super) struct CondaInventory<'a, 'lock> {
    pub packages: &'a [PreparedPackage<'lock>],
    pub paths: &'a [(String, PathsJson)],
    pub python: Option<&'a PythonInfo>,
}

/// Prepare the entire batch before Conda's first link, including uv's temporary
/// `.data` footprint. A transient copy may not clobber another package either.
pub(super) fn prepare<'a>(
    closure: &'a LockedCondaEnvironment,
    archives: &BTreeMap<&str, &ArchiveRef>,
    conda: CondaInventory<'_, '_>,
    owners: &mut HashMap<String, String>,
    directory_paths: &BTreeSet<String>,
    final_prefix: &Path,
) -> HResult<PreparedWheels<'a>> {
    if closure.wheels.is_empty() {
        return Ok(PreparedWheels {
            target: None,
            wheels: Vec::new(),
        });
    }
    let target = PythonTarget::from_lock(closure).map_err(invalid)?;
    if !owners.contains_key("bin/python") || directory_paths.contains("bin/python") {
        return Err(invalid("locked Conda interpreter does not own bin/python"));
    }
    let metadata = conda_metadata(
        conda.packages,
        conda.paths,
        conda.python,
        &target.site_packages,
    )?;
    gripsack_conda::wheels::validate_dependencies(closure, &metadata).map_err(invalid)?;
    let python_version = target.version.to_string();
    let mut wheels = Vec::with_capacity(closure.wheels.len());
    for record in &closure.wheels {
        let filename = target.admit(record).map_err(invalid)?;
        let archive = archives
            .get(record.sha256.as_str())
            .ok_or_else(|| invalid(format!("missing retained wheel '{}'", record.filename)))?;
        let archive = WheelArchive::admit(Path::new(&archive.path), record).map_err(invalid)?;
        let inventory = archive
            .installation_inventory_at(&python_version, final_prefix, &target.site_packages)
            .map_err(invalid)?;
        let owner = format!("wheel:{}", record.name);
        for file in &inventory {
            if file.path().parent() == Some(Path::new("bin")) {
                let command = file
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| invalid("invalid wheel launcher name"))?;
                if let Some(reserved) =
                    uv_install_wheel::reserved_script_name(&command.to_ascii_lowercase())
                {
                    return Err(invalid(format!(
                        "wheel launcher '{command}' uses reserved interpreter name '{reserved}'"
                    )));
                }
            }
            claim_path(owners, file.path(), &owner)?;
        }
        claim_path(
            owners,
            Path::new(&gripsack_conda::receipt::wheel_path(record)),
            &owner,
        )?;
        wheels.push(PreparedWheel {
            record,
            archive,
            filename,
            inventory,
        });
    }
    let mut footprint = owners.clone();
    for wheel in &wheels {
        let owner = format!("wheel:{}", wheel.record.name);
        let root = wheel.archive.extracted_dir();
        let mut directories = vec![root.to_path_buf()];
        while let Some(directory) = directories.pop() {
            for entry in fs::read_dir(directory).map_err(invalid)? {
                let entry = entry.map_err(invalid)?;
                let file_type = entry.file_type().map_err(invalid)?;
                if file_type.is_dir() {
                    directories.push(entry.path());
                } else if file_type.is_file() {
                    let path = entry.path();
                    let relative = path.strip_prefix(root).map_err(invalid)?;
                    let temporary = target.site_packages.join(relative);
                    let temporary = temporary
                        .to_str()
                        .ok_or_else(|| invalid("non-UTF8 wheel path"))?;
                    match footprint.get(temporary) {
                        Some(previous) if previous == &owner => {}
                        Some(previous) => {
                            return Err(fail(
                                "clobber",
                                format!("uv temporary path '{temporary}' overlaps '{previous}'"),
                            ));
                        }
                        None => {
                            footprint.insert(temporary.to_owned(), owner.clone());
                        }
                    }
                } else {
                    return Err(invalid(
                        "admitted wheel extraction contains a non-regular entry",
                    ));
                }
            }
        }
    }
    check_ancestors(&footprint, directory_paths)?;
    Ok(PreparedWheels {
        target: Some(target),
        wheels,
    })
}

fn claim_path(owners: &mut HashMap<String, String>, path: &Path, owner: &str) -> HResult<()> {
    let path = path
        .to_str()
        .ok_or_else(|| invalid("non-UTF8 wheel destination"))?;
    if let Some(previous) = owners.get(path) {
        return Err(fail(
            "clobber",
            format!("'{owner}' and '{previous}' both install '{path}'"),
        ));
    }
    owners.insert(path.to_owned(), owner.to_owned());
    Ok(())
}

fn check_ancestors(
    owners: &HashMap<String, String>,
    directories: &BTreeSet<String>,
) -> HResult<()> {
    for (path, owner) in owners {
        let mut ancestor = path.as_str();
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if !directories.contains(parent)
                && let Some(previous) = owners.get(parent)
            {
                return Err(fail(
                    "clobber",
                    format!("'{owner}' path '{path}' traverses '{parent}' owned by '{previous}'"),
                ));
            }
            ancestor = parent;
        }
    }
    Ok(())
}

fn conda_metadata(
    packages: &[PreparedPackage<'_>],
    package_paths: &[(String, PathsJson)],
    python_info: Option<&PythonInfo>,
    site_packages: &Path,
) -> HResult<Vec<(String, Vec<u8>)>> {
    let mut metadata = Vec::new();
    for (package, (_, paths)) in packages.iter().zip(package_paths) {
        for entry in &paths.paths {
            if entry.path_type == rattler_conda_types::package::PathType::Directory {
                continue;
            }
            let target = if package.index.noarch.is_python() {
                python_info
                    .ok_or_else(|| invalid("Conda Python metadata has no interpreter"))?
                    .get_python_noarch_target_path(&entry.relative_path)
                    .into_owned()
            } else {
                entry.relative_path.clone()
            };
            let Ok(relative) = target.strip_prefix(site_packages) else {
                continue;
            };
            let parts: Vec<_> = relative.iter().collect();
            let is_metadata = match parts.as_slice() {
                [directory, file] => {
                    directory
                        .to_str()
                        .is_some_and(|name| name.ends_with(".dist-info"))
                        && *file == "METADATA"
                        || directory
                            .to_str()
                            .is_some_and(|name| name.ends_with(".egg-info"))
                            && *file == "PKG-INFO"
                }
                [file] => file
                    .to_str()
                    .is_some_and(|name| name.ends_with(".egg-info")),
                _ => false,
            };
            if !is_metadata {
                continue;
            }
            if entry.prefix_placeholder.is_some() {
                return Err(invalid(
                    "Python distribution metadata must not require prefix rewriting",
                ));
            }
            let source = package.directory.join(&entry.relative_path);
            let mut cursor = source.as_path();
            while cursor != package.directory {
                let observed = fs::symlink_metadata(cursor).map_err(invalid)?;
                if observed.file_type().is_symlink() {
                    return Err(invalid(
                        "Conda distribution metadata traverses a symbolic link",
                    ));
                }
                cursor = cursor
                    .parent()
                    .ok_or_else(|| invalid("metadata escapes extracted package"))?;
            }
            let file = fs::File::open(&source).map_err(invalid)?;
            if !file.metadata().map_err(invalid)?.is_file() {
                return Err(invalid("Conda distribution metadata is not a regular file"));
            }
            let mut bytes = Vec::new();
            file.take(gripsack_conda::protocol::MAX_REQUEST_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(invalid)?;
            if bytes.len() as u64 > gripsack_conda::protocol::MAX_REQUEST_BYTES {
                return Err(invalid(
                    "Conda distribution metadata exceeds admission budget",
                ));
            }
            metadata.push((
                target
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF8 metadata path"))?
                    .to_owned(),
                bytes,
            ));
        }
    }
    Ok(metadata)
}

impl PreparedWheels<'_> {
    /// Only an admitted batch can reach this effect boundary. All paths are
    /// staging paths, except the interpreter encoded in generated launchers.
    pub(super) fn install(
        self,
        staging: &Path,
        final_prefix: &Path,
    ) -> HResult<Vec<MaterializedWheel>> {
        let Some(target) = self.target else {
            return Ok(Vec::new());
        };
        let site_packages = staging.join(&target.site_packages);
        let layout = Layout {
            sys_executable: final_prefix.join("bin/python"),
            python_version: target.short_version,
            os_name: "posix".into(),
            scheme: Scheme {
                purelib: site_packages.clone(),
                platlib: site_packages,
                scripts: staging.join("bin"),
                data: staging.to_path_buf(),
                include: staging.join("include"),
            },
        };
        let state = InstallState::default();
        let mut receipts = Vec::with_capacity(self.wheels.len());
        for wheel in self.wheels {
            for file in &wheel.inventory {
                create_parents(staging, file.path())?;
            }
            uv_install_wheel::install_wheel::<(), ()>(
                &layout,
                false,
                wheel.archive.extracted_dir(),
                &wheel.filename,
                None,
                None,
                None,
                None,
                false,
                LinkMode::Copy,
                &state,
            )
            .map_err(invalid)?;
            // UV honors the parent's umask for generated files. Normalization is
            // explicit and limited to this wheel's previously admitted file set.
            let mut files = BTreeSet::new();
            for file in &wheel.inventory {
                let path = staging.join(file.path());
                let metadata = fs::symlink_metadata(&path).map_err(invalid)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(invalid("installer produced a non-regular wheel file"));
                }
                fs::set_permissions(path, fs::Permissions::from_mode(file.mode()))
                    .map_err(invalid)?;
                files.insert(
                    file.path()
                        .to_str()
                        .ok_or_else(|| invalid("non-UTF8 wheel receipt path"))?
                        .to_owned(),
                );
            }
            let wheel_meta = gripsack_conda::receipt::wheel_path(wheel.record);
            create_parents(staging, Path::new(&wheel_meta))?;
            let mut receipt = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(staging.join(&wheel_meta))
                .map_err(invalid)?;
            receipt
                .set_permissions(fs::Permissions::from_mode(
                    gripsack_conda::receipt::FILE_MODE,
                ))
                .map_err(invalid)?;
            gripsack_conda::receipt::write_wheel(
                &mut receipt,
                wheel.record,
                &files,
                final_prefix
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF8 final prefix"))?,
            )
            .map_err(invalid)?;
            receipt.flush().map_err(invalid)?;
            receipts.push(MaterializedWheel {
                name: wheel.record.name.clone(),
                wheel_meta,
            });
        }
        Ok(receipts)
    }
}

fn create_parents(staging: &Path, relative: &Path) -> HResult<()> {
    let mut path = staging.to_path_buf();
    let parent = relative
        .parent()
        .ok_or_else(|| invalid("wheel file has no parent"))?;
    for component in parent.components() {
        path.push(component);
        match fs::create_dir(&path) {
            Ok(()) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).map_err(invalid)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&path).map_err(invalid)?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(invalid(
                        "wheel destination parent is not an admitted directory",
                    ));
                }
            }
            Err(error) => return Err(invalid(error)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
