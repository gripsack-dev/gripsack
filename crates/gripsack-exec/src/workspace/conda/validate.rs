//! Independent materialized-tree validation (A3): after the optional
//! helper stages a closure, the core re-derives EVERY expectation from
//! the frozen lock records and the retained original archives — exact
//! file set, byte identity (including its own prefix-patch computation
//! for placeholder files), normalized conda-meta receipts, link-script
//! refusal and bytecode policy. A forged helper success, an extra file
//! or a single drifted byte is a hard refusal here.

use super::archive::{CondaArchive, EntryKind};
use crate::ExecError;
use gripsack_ir::workspace_v6::lock::{LockedCondaEnvironment, LockedCondaPackage};
use gripsack_process::executable;
use gripsack_store as store;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

/// paths.json record (bounded subset; unknown fields tolerated for
/// forward-compatible repodata, every consumer-relevant field checked).
#[derive(Debug, Deserialize)]
struct PathsJson {
    paths: Vec<PathEntry>,
}
#[derive(Debug, Deserialize)]
struct PathEntry {
    #[serde(rename = "_path")]
    path: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    prefix_placeholder: Option<String>,
    #[serde(default)]
    file_mode: Option<String>,
}

/// One expected payload file, derived from the original archive.
#[derive(Debug, Clone)]
struct ExpectedFile {
    /// Owning package archive digest (indexes the reader map).
    archive: String,
    sha256: Option<String>,
    placeholder: Option<String>,
    binary_mode: bool,
    symlink: Option<String>,
}


/// Measured runtime facts a consumer admission needs (A3/parent
/// boundary): ambient sonames the closure cannot satisfy itself and the
/// absolute loaders its executables name.
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

/// The validated tree: measured identity plus measured ambient needs.
pub(crate) struct ValidatedTree {
    pub tree: store::hash::PayloadHash,
    pub system: SystemRuntime,
    pub files: u64,
    pub bytes: u64,
}

fn failure(output: &str, detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        module: output.into(),
        step: "conda".into(),
        detail: detail.into(),
    }
}

/// Link scripts and activation hooks the materializer refuses, by exact
/// archive path. Packages carrying them are rejected BY NAME before any
/// byte of their payload is admitted.
const LINK_SCRIPTS: &[&str] = &[
    "info/recipe/pre-link.sh",
    "info/recipe/post-link.sh",
    "info/recipe/pre-unlink.sh",
    "info/recipe/pre-link.bat",
    "info/recipe/post-link.bat",
    "info/recipe/pre-unlink.bat",
];
const ACTIVATION_PREFIXES: &[&str] = &["etc/conda/activate.d/", "etc/conda/deactivate.d/"];

/// One package's expected inventory from its retained archive, plus the
/// refusal decision for unsupported packages.
fn inventory(
    output: &str,
    record: &LockedCondaPackage,
    archive: &CondaArchive,
) -> Result<BTreeMap<String, ExpectedFile>, ExecError> {
    for entry in archive.info_entries() {
        if LINK_SCRIPTS.contains(&entry.path.as_str()) {
            return Err(failure(
                output,
                format!(
                    "package {} carries an unsupported link script ({})",
                    record.name, entry.path
                ),
            ));
        }
    }
    for entry in archive.payload_entries() {
        if ACTIVATION_PREFIXES
            .iter()
            .any(|prefix| entry.path.starts_with(prefix))
        {
            return Err(failure(
                output,
                format!(
                    "package {} carries activation scripts with unsupported semantics ({})",
                    record.name, entry.path
                ),
            ));
        }
    }
    let paths = archive
        .info_document("paths.json")
        .map_err(|error| failure(output, format!("{}: {error}", record.name)))?
        .ok_or_else(|| failure(output, format!("{} has no info/paths.json", record.name)))?;
    let parsed: PathsJson = serde_json::from_slice(&paths)
        .map_err(|error| failure(output, format!("{} paths.json: {error}", record.name)))?;
    if parsed.paths.len() > archive.payload_entries().len().max(1) * 4 {
        return Err(failure(
            output,
            format!("{} paths.json disagrees with its archive listing", record.name),
        ));
    }
    let mut expected = BTreeMap::new();
    for entry in parsed.paths {
        if entry.path.ends_with(".pyc") || entry.path.ends_with(".pyo") {
            continue;
        }
        if entry.path.is_empty()
            || entry.path.starts_with('/')
            || entry.path.split('/').any(|segment| segment == "..")
        {
            return Err(failure(
                output,
                format!("{} paths.json carries an unsafe path", record.name),
            ));
        }
        let symlink = archive
            .payload_entries()
            .iter()
            .find(|candidate| candidate.path == entry.path)
            .and_then(|candidate| match &candidate.kind {
                EntryKind::Symlink { target } => Some(target.clone()),
                _ => None,
            });
        let record_entry = ExpectedFile {
            archive: record.sha256.clone(),
            sha256: entry.sha256,
            placeholder: entry.prefix_placeholder,
            binary_mode: entry.file_mode.as_deref() == Some("binary"),
            symlink,
        };
        if expected.insert(entry.path.clone(), record_entry).is_some() {
            return Err(failure(
                output,
                format!("{} paths.json repeats {:?}", record.name, entry.path),
            ));
        }
    }
    Ok(expected)
}

/// The core's own prefix-patch computation. Text files: literal
/// replacement. Binary files: the new prefix must fit the placeholder,
/// padded with NUL bytes — the conda binary rule.
fn patched(
    original: &[u8],
    placeholder: &str,
    prefix: &str,
    binary: bool,
) -> Result<Vec<u8>, String> {
    let needle = placeholder.as_bytes();
    if needle.is_empty() {
        return Err("empty placeholder".into());
    }
    if binary && prefix.len() > needle.len() {
        return Err(format!(
            "prefix ({} bytes) exceeds its binary placeholder ({} bytes)",
            prefix.len(),
            needle.len()
        ));
    }
    let replacement = if binary {
        let mut padded = prefix.as_bytes().to_vec();
        padded.resize(needle.len(), 0);
        padded
    } else {
        prefix.as_bytes().to_vec()
    };
    let mut output = Vec::with_capacity(original.len() + replacement.len());
    let mut cursor = 0;
    let mut occurrences = 0usize;
    while cursor < original.len() {
        if original[cursor..].starts_with(needle) {
            output.extend_from_slice(&replacement);
            cursor += needle.len();
            occurrences += 1;
        } else {
            output.push(original[cursor]);
            cursor += 1;
        }
    }
    if occurrences == 0 {
        return Err("placeholder never occurs in the payload file".into());
    }
    Ok(output)
}

fn sha256_hex(bytes: &[u8]) -> String {
    gripsack_process::Sha256Digest::of(bytes).to_string()
}

fn receipt_name(relative: &str) -> Option<&str> {
    relative
        .strip_prefix("conda-meta/")
        .filter(|name| name.ends_with(".json") && !name.contains('/'))
}

/// Validate one staged tree against the frozen closure and its retained
/// original archives. `final_prefix` is the exact string the bytes were
/// patched for. Returns the measured tree identity plus the ambient
/// system runtime the tree still needs.
pub(crate) fn validate_tree(
    output: &str,
    locked: &LockedCondaEnvironment,
    archives: &BTreeMap<String, PathBuf>,
    tree_root: &Path,
    final_prefix: &str,
) -> Result<ValidatedTree, ExecError> {
    // 1. Expected inventory per package, with unsupported-package refusal.
    let mut readers: BTreeMap<String, CondaArchive> = BTreeMap::new();
    let mut expected: BTreeMap<String, ExpectedFile> = BTreeMap::new();
    for record in &locked.packages {
        let archive_path = archives
            .get(&record.sha256)
            .ok_or_else(|| failure(output, format!("{} archive is not retained", record.name)))?;
        if !readers.contains_key(&record.sha256) {
            let reader = CondaArchive::open(archive_path).map_err(|error| {
                failure(
                    output,
                    format!("{} archive cannot be read: {error}", record.name),
                )
            })?;
            readers.insert(record.sha256.clone(), reader);
        }
        let reader = &readers[&record.sha256];
        for (path, file) in inventory(output, record, reader)? {
            if expected.insert(path.clone(), file).is_some() {
                return Err(failure(
                    output,
                    format!("path {path:?} is claimed by two packages"),
                ));
            }
        }
    }
    // 2. Walk the staged tree: no extras, exact bytes per expectation.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut receipts: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut sonames_needed: BTreeSet<String> = BTreeSet::new();
    let mut loaders: BTreeSet<String> = BTreeSet::new();
    let mut files = 0u64;
    let mut bytes = 0u64;
    let entry_cap = expected.len() as u64 + locked.packages.len() as u64 + 1024;
    let mut walked = 0u64;
    let mut stack = vec![PathBuf::from(tree_root)];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            walked += 1;
            if walked > entry_cap {
                return Err(failure(
                    output,
                    "staged tree exceeds its lock-derived entry bound",
                ));
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(tree_root)
                .map_err(|_| failure(output, "staged path escapes its root"))?
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if receipt_name(&relative).is_some() && metadata.is_file() {
                receipts.insert(relative, path);
                continue;
            }
            if relative.ends_with(".pyc") || relative.contains("__pycache__") {
                return Err(failure(
                    output,
                    format!("bytecode policy Suppress admits no compiled python ({relative:?})"),
                ));
            }
            let Some(want) = expected.get(&relative) else {
                return Err(failure(
                    output,
                    format!("staged tree carries unaccounted file {relative:?}"),
                ));
            };
            seen.insert(
                expected
                    .get_key_value(&relative)
                    .expect("present")
                    .0
                    .as_str(),
            );
            if let Some(link) = &want.symlink {
                if !metadata.file_type().is_symlink() {
                    return Err(failure(output, format!("{relative:?} must be a symlink")));
                }
                let target = std::fs::read_link(&path)?;
                let target_text = target.to_string_lossy().into_owned();
                // Admissible: the archive's recorded target verbatim, or
                // its placeholder-patched spelling. Targets escaping the
                // prefix are refused outright.
                let patched_target = want
                    .placeholder
                    .as_ref()
                    .map(|placeholder| link.replace(placeholder.as_str(), final_prefix));
                if target_text != *link && Some(&target_text) != patched_target.as_ref() {
                    return Err(failure(
                        output,
                        format!("{relative:?} symlink target differs from its archive"),
                    ));
                }
                let normalized = if target.is_absolute() {
                    target.clone()
                } else {
                    path.parent().unwrap_or(tree_root).join(&target)
                };
                let mut resolved = PathBuf::new();
                for component in normalized.components() {
                    match component {
                        std::path::Component::ParentDir => {
                            resolved.pop();
                        }
                        std::path::Component::CurDir => {}
                        other => resolved.push(other.as_os_str()),
                    }
                }
                let prefix_root = if target.is_absolute() {
                    PathBuf::from(final_prefix)
                } else {
                    tree_root.to_path_buf()
                };
                if !resolved.starts_with(&prefix_root) {
                    return Err(failure(
                        output,
                        format!("{relative:?} symlink target escapes its prefix"),
                    ));
                }
                continue;
            }
            if !metadata.is_file() {
                return Err(failure(
                    output,
                    format!("{relative:?} must be a regular file"),
                ));
            }
            files += 1;
            bytes += metadata.len();
            let actual = std::fs::read(&path)?;
            if let Some(placeholder) = &want.placeholder {
                let original = readers[&want.archive]
                    .payload_bytes(&relative)
                    .map_err(|error| failure(output, format!("{relative:?}: {error}")))?
                    .ok_or_else(|| {
                        failure(
                            output,
                            format!("{relative:?} original bytes are unavailable for patch check"),
                        )
                    })?;
                let wanted = patched(&original, placeholder, final_prefix, want.binary_mode)
                    .map_err(|detail| failure(output, format!("{relative:?}: {detail}")))?;
                if actual != wanted {
                    return Err(failure(
                        output,
                        format!("{relative:?} differs from the independently patched original"),
                    ));
                }
            } else if let Some(expected_sha) = &want.sha256 {
                if sha256_hex(&actual) != *expected_sha {
                    return Err(failure(
                        output,
                        format!("{relative:?} differs from its archive byte identity"),
                    ));
                }
            }
            // Ambient runtime measurement: ELF needs the closure cannot
            // satisfy and absolute loaders are receipt facts.
            if let Ok(mut file) = std::fs::File::open(&path) {
                let mut head = [0u8; 4];
                if file.read_exact(&mut head).is_ok() && &head == b"\x7fELF" {
                    file.rewind()?;
                    if let Ok(metadata) = executable::classify(&mut file) {
                        for needed in &metadata.needed_libraries {
                            sonames_needed.insert(needed.to_string_lossy().into_owned());
                        }
                        if let Some(executable::Interpreter::Loader(loader)) = &metadata.interpreter
                        {
                            loaders.insert(loader.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
    }
    // 3. No missing payload files.
    for path in expected.keys() {
        if !seen.contains(path.as_str()) {
            return Err(failure(
                output,
                format!("staged tree is missing expected file {path:?}"),
            ));
        }
    }
    // 4. Receipts bind every frozen field, not just name/version/hash. Derive
    // installed file lists from our archive inventory, never from helper output.
    let expected_receipts: BTreeMap<_, _> = locked.packages.iter().map(|record| (
        format!("conda-meta/{}-{}-{}.json", record.name, record.version, record.build),
        record,
    )).collect();
    if !receipts.keys().eq(expected_receipts.keys()) {
        return Err(failure(output, "conda-meta receipt set differs from the frozen closure"));
    }
    let mut package_files: BTreeMap<&str, BTreeSet<&str>> = locked.packages.iter()
        .map(|record| (record.sha256.as_str(), BTreeSet::new())).collect();
    for (path, file) in &expected {
        package_files.get_mut(file.archive.as_str())
            .expect("inventory names a frozen archive").insert(path.as_str());
    }
    for (relative, path) in receipts {
        let record = expected_receipts[&relative];
        gripsack_conda::receipt::verify(
            std::fs::File::open(path)?, record, &package_files[record.sha256.as_str()],
        ).map_err(|error| failure(output,
            format!("conda-meta receipt {relative:?} does not match its frozen record: {error}")))?;
    }
    // 5. Ambient sonames the closure cannot satisfy itself.
    let provided: BTreeSet<String> = expected
        .keys()
        .filter_map(|path| path.rsplit('/').next().map(str::to_owned))
        .collect();
    let system = SystemRuntime {
        libraries: sonames_needed
            .into_iter()
            .filter(|soname| !provided.contains(soname))
            .collect(),
        loaders: loaders.into_iter().collect(),
    };
    let tree = store::canonical_tree_hash(tree_root)?;
    Ok(ValidatedTree {
        tree,
        system,
        files,
        bytes,
    })
}
