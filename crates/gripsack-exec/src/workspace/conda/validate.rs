//! Independent A3 admission: retained archive bytes + full frozen records are
//! authority. Helper inventories, hashes and successful exits are not proof.
#[path = "inventory.rs"]
mod inventory;
#[path = "links.rs"]
mod links;
#[path = "signing.rs"]
mod signing;
#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;

use super::archive::{CondaArchive, MAX_ENTRY_BYTES};
use crate::ExecError;
use gripsack_ir::workspace_model::lock::LockedCondaEnvironment;
use gripsack_process::executable;
use gripsack_store as store;
use inventory::{Content, ExpectedFile, PythonLayout};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

// The closure bound includes all expanded tar headers/info/payload. It is
// independent of helper declarations and limits aggregate private spool usage.
const MAX_CLOSURE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_CLOSURE_ENTRIES: usize = 1_048_576;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SystemRuntime {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loaders: Vec<String>,
}
impl SystemRuntime {
    pub(crate) fn is_empty(&self) -> bool {
        self.libraries.is_empty() && self.loaders.is_empty()
    }
}
pub(crate) struct ValidatedTree {
    pub tree: store::hash::PayloadHash,
    pub system: SystemRuntime,
}

/// Publication deliberately strips write bits from regular files (0016 §D3).
/// Callers supply the lifecycle phase; a writable retained file must not gain
/// admission merely because it would have been legal in private staging.
#[derive(Clone, Copy)]
pub(crate) enum FileModes {
    Staged,
    Published,
}
impl FileModes {
    fn expected(self, mode: u32, regular: bool) -> u32 {
        match self {
            Self::Published if regular => mode & !0o222,
            _ => mode,
        }
    }
}
fn failure(output: &str, detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        module: output.into(),
        step: "conda".into(),
        detail: detail.into(),
    }
}

fn compare_file(
    want: &ExpectedFile,
    actual: &mut std::fs::File,
    archive: &CondaArchive,
    relative: &str,
    prefix: &str,
    platform: &str,
) -> Result<(), String> {
    let length = actual.metadata().map_err(|error| error.to_string())?.len();
    // Text relocation may grow a file, but never lets a staged length allocate
    // memory. Original payload and comparator are bounded independently.
    if length > MAX_ENTRY_BYTES {
        return Err("staged file exceeds its byte bound".into());
    }
    match &want.content {
        Content::Original {
            path,
            sha256,
            size,
            patch: None,
        } => {
            if length != *size {
                return Err(format!("{path:?} differs from original byte length"));
            }
            let digest =
                super::archive::hash_reader(actual, *size).map_err(|error| error.to_string())?;
            if digest != *sha256 {
                return Err("differs from original archive byte identity".into());
            }
        }
        Content::Original {
            path,
            patch: Some(patch),
            ..
        } => {
            let original = archive
                .payload_bytes(path)
                .map_err(|error| error.to_string())?;
            if patch.placeholder != prefix
                && signing::required(&original, want.mode, patch, platform)
            {
                signing::verify(&original, actual, relative, patch, prefix, platform)?;
            } else {
                super::prefix::verify(
                    &original,
                    actual,
                    &patch.placeholder,
                    prefix,
                    patch.binary,
                    platform,
                )?;
            }
        }
        Content::Generated(expected) => {
            if length != expected.len() as u64 {
                return Err("generated entry point has wrong length".into());
            }
            super::prefix::Comparison(actual)
                .write_all(expected)
                .map_err(|error| error.to_string())?;
        }
        _ => return Err("non-file inventory used for regular file comparison".into()),
    }
    Ok(())
}

pub(crate) fn validate_tree(
    output: &str,
    locked: &LockedCondaEnvironment,
    archives: &BTreeMap<String, PathBuf>,
    tree_root: &Path,
    final_prefix: &str,
    modes: FileModes,
) -> Result<ValidatedTree, ExecError> {
    if !matches!(
        locked.platform.as_str(),
        "linux-64" | "linux-aarch64" | "osx-64" | "osx-arm64"
    ) {
        return Err(failure(output, "unsupported Conda installation platform"));
    }
    if !final_prefix.starts_with('/')
        || final_prefix.ends_with('/')
        || final_prefix.len() > super::archive::MAX_PATH_BYTES
        || final_prefix.contains(['\0', '\n', '\r', '\\', '"', '\'', '$', '`'])
        || final_prefix
            .split('/')
            .skip(1)
            .any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(failure(
            output,
            "final prefix is not a canonical shell-safe absolute path",
        ));
    }
    let python = PythonLayout::from_lock(locked).map_err(|detail| failure(output, detail))?;
    let mut readers = BTreeMap::new();
    let mut expected = BTreeMap::new();
    let mut expanded = 0;
    let mut package_files: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for record in &locked.packages {
        let archive_path = archives
            .get(&record.sha256)
            .ok_or_else(|| failure(output, format!("{} archive is not retained", record.name)))?;
        if !readers.contains_key(&record.sha256) {
            let reader = CondaArchive::open(archive_path, &record.sha256)
                .map_err(|error| failure(output, format!("{}: {error}", record.name)))?;
            expanded += reader.expanded_bytes;
            if expanded > MAX_CLOSURE_BYTES {
                return Err(failure(
                    output,
                    "archive closure exceeds expanded byte bound",
                ));
            }
            readers.insert(record.sha256.clone(), reader);
        }
        let inventory = inventory::inventory(
            record,
            &readers[&record.sha256],
            python.as_ref(),
            final_prefix,
        )
        .map_err(|detail| failure(output, format!("{}: {detail}", record.name)))?;
        package_files.insert(&record.sha256, inventory.keys().cloned().collect());
        for (path, file) in inventory {
            if expected.insert(path.clone(), file).is_some() {
                return Err(failure(
                    output,
                    format!("path {path:?} is claimed by two packages"),
                ));
            }
        }
        if expected.len() > MAX_CLOSURE_ENTRIES {
            return Err(failure(output, "closure inventory exceeds entry bound"));
        }
    }
    let directories = links::directories(&expected).map_err(|detail| failure(output, detail))?;
    links::verify(&expected, &directories, final_prefix)
        .map_err(|detail| failure(output, detail))?;
    let expected_receipts: BTreeMap<_, _> = locked
        .packages
        .iter()
        .map(|record| {
            (
                format!(
                    "conda-meta/{}-{}-{}.json",
                    record.name, record.version, record.build
                ),
                record,
            )
        })
        .collect();
    let root = gripsack_fs::open(tree_root)?;
    let mut seen = BTreeSet::new();
    let mut receipts = BTreeSet::new();
    let mut sonames_needed = BTreeSet::new();
    let mut loaders = BTreeSet::new();
    let (mut staged_bytes, mut walked) = (0u64, 0usize);
    let entry_cap = expected.len() + directories.len() + expected_receipts.len();
    // Every directory is pinned before walking; leaf reads never follow a
    // final symlink and FIFO/special files cannot block admission.
    let mut stack = vec![(root, String::new())];
    while let Some((directory, parent)) = stack.pop() {
        for entry in directory.entries()? {
            let entry = entry?;
            walked += 1;
            if walked > entry_cap {
                return Err(failure(
                    output,
                    "staged tree exceeds its lock-derived entry bound",
                ));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| failure(output, "non-UTF-8 staged path"))?;
            let relative = if parent.is_empty() {
                name.clone()
            } else {
                format!("{parent}/{name}")
            };
            let metadata = directory.symlink_metadata(&name)?;
            if metadata.is_file() {
                if metadata.len() > MAX_CLOSURE_BYTES - staged_bytes {
                    return Err(failure(
                        output,
                        "staged closure exceeds its aggregate byte bound",
                    ));
                }
                staged_bytes += metadata.len();
            }
            if metadata.is_dir() {
                if !directories.contains(&relative) {
                    return Err(failure(
                        output,
                        format!("unaccounted directory {relative:?}"),
                    ));
                }
                if let Some(want) = expected.get(&relative) {
                    if !matches!(want.content, Content::Directory) {
                        return Err(failure(output, format!("{relative:?} has wrong type")));
                    }
                    check_mode(
                        output,
                        &relative,
                        &metadata,
                        modes.expected(want.mode, false),
                    )?;
                    seen.insert(relative.clone());
                }
                stack.push((
                    gripsack_fs::open_dir_nofollow(&directory, Path::new(&name))?,
                    relative,
                ));
                continue;
            }
            if let Some(record) = expected_receipts.get(&relative) {
                let file = gripsack_fs::open_file_nofollow(&directory, Path::new(&name))?;
                let metadata = file.metadata()?;
                check_private_file(output, &relative, &metadata)?;
                check_mode(
                    output,
                    &relative,
                    &metadata,
                    modes.expected(gripsack_conda::receipt::FILE_MODE, true),
                )?;
                gripsack_conda::receipt::verify(
                    file,
                    record,
                    &package_files[record.sha256.as_str()],
                    final_prefix,
                )
                .map_err(|error| failure(output, format!("receipt {relative:?}: {error}")))?;
                receipts.insert(relative);
                continue;
            }
            let want = expected
                .get(&relative)
                .ok_or_else(|| failure(output, format!("unaccounted staged file {relative:?}")))?;
            seen.insert(relative.clone());
            if let Content::Symlink(target) = &want.content {
                if !metadata.file_type().is_symlink()
                    || directory.read_link_contents(&name)?.to_str() != Some(target)
                {
                    return Err(failure(
                        output,
                        format!("symlink {relative:?} differs from original target"),
                    ));
                }
                continue;
            }
            let file = gripsack_fs::open_file_nofollow(&directory, Path::new(&name))?;
            let metadata = file.metadata()?;
            check_private_file(output, &relative, &metadata)?;
            check_mode(
                output,
                &relative,
                &metadata,
                modes.expected(want.mode, true),
            )?;
            let mut file = file.into_std();
            compare_file(
                want,
                &mut file,
                &readers[&want.archive],
                &relative,
                final_prefix,
                &locked.platform,
            )
            .map_err(|detail| failure(output, format!("{relative:?}: {detail}")))?;
            file.rewind()?;
            let mut head = [0; 4];
            let read = file.read(&mut head)?;
            let elf = read == 4 && &head == b"\x7fELF";
            let macho =
                read == 4 && locked.platform.starts_with("osx-") && signing::is_macho(&head);
            if elf || macho {
                file.rewind()?;
                let runtime = executable::classify(&mut file).map_err(|error| {
                    failure(output, format!("invalid executable {relative:?}: {error}"))
                })?;
                for library in runtime.needed_libraries {
                    let path = Path::new(&library);
                    // Mach relative install names are resolved by native
                    // consumer admission, not misreported as ambient needs.
                    if elf || (path.is_absolute() && !path.starts_with(final_prefix)) {
                        sonames_needed.insert(library.to_string_lossy().into_owned());
                    }
                }
                if let Some(executable::Interpreter::Loader(loader)) = runtime.interpreter {
                    loaders.insert(loader.to_string_lossy().into_owned());
                }
            }
        }
    }
    if !seen.iter().eq(expected.keys()) {
        return Err(failure(
            output,
            "staged tree is missing expected archive/generated paths",
        ));
    }
    if !receipts.iter().eq(expected_receipts.keys()) {
        return Err(failure(
            output,
            "conda-meta receipt set differs from frozen closure",
        ));
    }
    let provided: BTreeSet<_> = expected
        .keys()
        .filter_map(|path| path.rsplit('/').next())
        .collect();
    let system = SystemRuntime {
        libraries: sonames_needed
            .into_iter()
            .filter(|name| !provided.contains(name.as_str()))
            .collect(),
        loaders: loaders.into_iter().collect(),
    };
    Ok(ValidatedTree {
        tree: store::canonical_tree_hash(tree_root)?,
        system,
    })
}

fn check_mode(
    output: &str,
    path: &str,
    metadata: &gripsack_fs::cap_std::fs::Metadata,
    mode: u32,
) -> Result<(), ExecError> {
    use gripsack_fs::cap_std::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o7777 != mode {
        return Err(failure(
            output,
            format!("{path:?} permissions differ from archive/generated mode"),
        ));
    }
    Ok(())
}
fn check_private_file(
    output: &str,
    path: &str,
    metadata: &gripsack_fs::cap_std::fs::Metadata,
) -> Result<(), ExecError> {
    use gripsack_fs::cap_std::fs::MetadataExt;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(failure(
            output,
            format!("{path:?} is not a private regular file (shared hardlink forbidden)"),
        ));
    }
    Ok(())
}
