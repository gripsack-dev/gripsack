//! Archive/lock-derived installed paths, relocation and generated Python wrappers.
use super::super::archive::{ArchiveEntry, CondaArchive, EntryKind, safe_path};
use gripsack_ir::workspace_v6::lock::{LockedCondaEnvironment, LockedCondaPackage, LockedNoArch};
use gripsack_process::Sha256Digest;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug)]
pub(super) enum Content {
    Original {
        path: String,
        sha256: Sha256Digest,
        size: u64,
        patch: Option<PrefixPatch>,
    },
    Generated(Vec<u8>),
    Symlink(String),
    Directory,
}
#[derive(Debug)]
pub(super) struct PrefixPatch {
    pub placeholder: String,
    pub binary: bool,
}
#[derive(Debug)]
pub(super) struct ExpectedFile {
    pub archive: String,
    pub mode: u32,
    pub content: Content,
}
#[derive(Deserialize)]
struct PathsJson {
    paths_version: u64,
    paths: Vec<PathEntry>,
}
#[derive(Deserialize)]
struct PathEntry {
    #[serde(rename = "_path")]
    path: String,
    path_type: String,
    sha256: Option<String>,
    size_in_bytes: Option<u64>,
    prefix_placeholder: Option<String>,
    file_mode: Option<String>,
}
#[derive(Deserialize)]
struct LinkJson {
    package_metadata_version: u64,
    noarch: NoArchLinks,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum NoArchLinks {
    Generic,
    Python {
        #[serde(default)]
        entry_points: Vec<String>,
    },
}

pub(super) struct PythonLayout {
    executable: String,
    site_packages: String,
}
impl PythonLayout {
    pub fn from_lock(locked: &LockedCondaEnvironment) -> Result<Option<Self>, String> {
        let Some(python) = locked
            .packages
            .iter()
            .find(|record| record.name == "python")
        else {
            return Ok(None);
        };
        let mut parts = python.version.split('.');
        let major = parts.next().unwrap_or("");
        let minor = parts.next().unwrap_or("");
        if [major, minor]
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err("frozen Python version has no numeric major/minor".into());
        }
        let site_packages = python
            .python_site_packages_path
            .clone()
            .unwrap_or_else(|| format!("lib/python{major}.{minor}/site-packages"));
        if !safe_path(&site_packages) {
            return Err("unsafe frozen Python site-packages path".into());
        }
        Ok(Some(Self {
            executable: format!("bin/python{major}.{minor}"),
            site_packages,
        }))
    }

    fn relocate(&self, path: &str) -> String {
        if let Some(rest) = path.strip_prefix("site-packages/") {
            format!("{}/{rest}", self.site_packages)
        } else if let Some(rest) = path.strip_prefix("python-scripts/") {
            format!("bin/{rest}")
        } else {
            path.into()
        }
    }

    fn wrapper(&self, entry: &str, prefix: &str) -> Result<(String, Vec<u8>), String> {
        let (command, callable) = entry.split_once('=').ok_or("entry point has no '='")?;
        let command = command.trim();
        if !safe_path(command) || command.contains('/') {
            return Err("unsafe entry point command".into());
        }
        let (module, function) = callable
            .trim()
            .split_once(':')
            .ok_or("entry point has no ':'")?;
        let (module, function) = (module.trim(), function.trim());
        let identifier = |part: &str| {
            let mut chars = part.chars();
            chars
                .next()
                .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
                && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        };
        if !module.split('.').all(identifier) || !function.split('.').all(identifier) {
            return Err("unsupported Python entry point identifier".into());
        }
        let executable = format!("{prefix}/{}", self.executable);
        let shebang = if executable.len() > 125 || executable.contains(' ') {
            format!("#!/bin/sh\n'''exec' \"{executable}\" \"$0\" \"$@\" #'''")
        } else {
            format!("#!{executable}")
        };
        let import = function
            .split('.')
            .next()
            .ok_or("missing Python entry point function")?;
        let bytes = format!("{shebang}\n# -*- coding: utf-8 -*-\nimport re\nimport sys\n\nfrom {module} import {import}\n\nif __name__ == '__main__':\n\tsys.argv[0] = re.sub(r'(-script\\.pyw?|\\.exe)?$', '', sys.argv[0])\n\tsys.exit({function}())\n").into_bytes();
        Ok((format!("bin/{command}"), bytes))
    }
}

fn document(archive: &CondaArchive, name: &str) -> Result<Option<Vec<u8>>, String> {
    archive
        .info_document(name)
        .map_err(|error| error.to_string())
}

fn script(path: &str, name: &str) -> bool {
    [
        "etc/conda/activate.d",
        "etc/conda/deactivate.d",
        "etc/activate.d",
        "etc/deactivate.d",
    ]
    .iter()
    .any(|prefix| {
        path == *prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    }) || ["pre-link", "post-link", "pre-unlink"].iter().any(|phase| {
        path == format!("info/recipe/{phase}.sh")
            || path == format!("info/recipe/{phase}.bat")
            || path == format!("bin/.{name}-{phase}.sh")
            || path == format!("Scripts/.{name}-{phase}.bat")
    })
}

fn paths(archive: &CondaArchive) -> Result<Vec<PathEntry>, String> {
    if let Some(bytes) = document(archive, "paths.json")? {
        let parsed: PathsJson =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if parsed.paths_version != 1 {
            return Err("unsupported paths.json version".into());
        }
        return Ok(parsed.paths);
    }
    // Legacy tar.bz2 packages distribute paths.json across files/has_prefix.
    let bytes = document(archive, "files")?.ok_or("missing paths.json and legacy info/files")?;
    let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
    let mut result = BTreeMap::new();
    for path in text.lines() {
        let entry = archive
            .payload
            .get(path)
            .ok_or("legacy info/files names an absent payload")?;
        let kind = match entry.kind {
            EntryKind::File { .. } => "hardlink",
            EntryKind::Symlink { .. } => "softlink",
            EntryKind::Directory => "directory",
        };
        if result
            .insert(
                path.to_owned(),
                PathEntry {
                    path: path.into(),
                    path_type: kind.into(),
                    sha256: None,
                    size_in_bytes: None,
                    prefix_placeholder: None,
                    file_mode: None,
                },
            )
            .is_some()
        {
            return Err("legacy info/files repeats a path".into());
        }
    }
    if let Some(bytes) = document(archive, "has_prefix")? {
        let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
        for line in text.lines() {
            let mut rest = line.trim();
            let mut fields = Vec::new();
            while !rest.is_empty() {
                let (field, remaining) = if let Some(quoted) = rest.strip_prefix('"') {
                    let end = quoted.find('"').ok_or("unterminated legacy prefix quote")?;
                    (&quoted[..end], &quoted[end + 1..])
                } else {
                    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                    (&rest[..end], &rest[end..])
                };
                fields.push(field);
                rest = remaining.trim_start();
            }
            let (placeholder, mode, path) = match fields.as_slice() {
                [path] => ("/opt/anaconda1anaconda2anaconda3", "text", *path),
                [placeholder, mode, path] => (*placeholder, *mode, *path),
                _ => return Err("unsupported legacy prefix declaration".into()),
            };
            let entry = result
                .get_mut(path)
                .ok_or("has_prefix names an absent path")?;
            if entry.prefix_placeholder.is_some() {
                return Err("duplicate legacy prefix declaration".into());
            }
            entry.prefix_placeholder = Some(placeholder.into());
            entry.file_mode = Some(mode.into());
        }
    }
    Ok(result.into_values().collect())
}

pub(super) fn inventory(
    record: &LockedCondaPackage,
    archive: &CondaArchive,
    python: Option<&PythonLayout>,
    final_prefix: &str,
) -> Result<BTreeMap<String, ExpectedFile>, String> {
    for path in archive.info.keys().chain(archive.payload.keys()) {
        if script(path, &record.name) {
            return Err(format!("unsupported link/activation script {path:?}"));
        }
    }
    let index = document(archive, "index.json")?.ok_or("missing info/index.json")?;
    let index: serde_json::Value =
        serde_json::from_slice(&index).map_err(|error| error.to_string())?;
    if index["name"].as_str() != Some(&record.name)
        || index["version"].as_str() != Some(&record.version)
        || index["build"].as_str() != Some(&record.build)
        || index["build_number"].as_u64() != Some(record.build_number)
    {
        return Err("archive index identity differs from frozen record".into());
    }
    let is_python = matches!(record.noarch, LockedNoArch::Python);
    let layout = if is_python {
        Some(python.ok_or("noarch Python package has no locked Python interpreter")?)
    } else {
        None
    };
    let mut result = BTreeMap::new();
    let mut originals = std::collections::BTreeSet::new();
    for entry in paths(archive)? {
        if !safe_path(&entry.path) || entry.path.starts_with("conda-meta/") {
            return Err("unsafe/reserved payload path".into());
        }
        if !originals.insert(entry.path.clone()) {
            return Err("paths metadata repeats a path".into());
        }
        let ArchiveEntry { kind, mode } = archive
            .payload
            .get(&entry.path)
            .ok_or("path metadata names an absent payload")?;
        let patch = match (entry.prefix_placeholder, entry.file_mode.as_deref()) {
            (None, None) => None,
            (Some(placeholder), Some(mode @ ("text" | "binary")))
                if !placeholder.is_empty() && !placeholder.contains('\0') =>
            {
                Some(PrefixPatch {
                    placeholder,
                    binary: mode == "binary",
                })
            }
            _ => return Err("unsupported prefix patch semantics".into()),
        };
        let content = match (kind, entry.path_type.as_str()) {
            (EntryKind::File { size, sha256, .. }, "hardlink") => {
                let declared = entry
                    .sha256
                    .as_deref()
                    .map(Sha256Digest::parse)
                    .transpose()
                    .map_err(|error| error.to_string())?;
                if declared.is_some_and(|hash| hash != *sha256)
                    || entry.size_in_bytes.is_some_and(|length| length != *size)
                {
                    return Err(format!(
                        "original payload {:?} disagrees with paths metadata",
                        entry.path
                    ));
                }
                Content::Original {
                    path: entry.path.clone(),
                    sha256: *sha256,
                    size: *size,
                    patch,
                }
            }
            (EntryKind::Symlink { target }, "softlink") if patch.is_none() => {
                Content::Symlink(target.clone())
            }
            (EntryKind::Directory, "directory") if patch.is_none() => Content::Directory,
            _ => return Err("unsupported path kind or symlink patch semantics".into()),
        };
        if entry.path.ends_with(".pyc") || entry.path.ends_with(".pyo") {
            continue;
        }
        let path = layout.map_or_else(|| entry.path.clone(), |layout| layout.relocate(&entry.path));
        if result
            .insert(
                path,
                ExpectedFile {
                    archive: record.sha256.clone(),
                    mode: *mode,
                    content,
                },
            )
            .is_some()
        {
            return Err("relocated paths clobber each other".into());
        }
    }
    for (path, entry) in &archive.payload {
        if !matches!(entry.kind, EntryKind::Directory) && !originals.contains(path) {
            return Err(format!(
                "archive payload {path:?} is absent from its paths metadata"
            ));
        }
    }
    if let Some(bytes) = document(archive, "link.json")? {
        let links: LinkJson = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if links.package_metadata_version != 1 {
            return Err("unsupported link metadata version".into());
        }
        match links.noarch {
            NoArchLinks::Python { entry_points } => {
                let layout =
                    layout.ok_or("Python entry points disagree with frozen noarch kind")?;
                if entry_points.len() > archive.payload.len().max(1) * 4 {
                    return Err("entry point count exceeds archive-derived bound".into());
                }
                for entry in entry_points {
                    let (path, bytes) = layout.wrapper(&entry, final_prefix)?;
                    if result
                        .insert(
                            path,
                            ExpectedFile {
                                archive: record.sha256.clone(),
                                mode: 0o775,
                                content: Content::Generated(bytes),
                            },
                        )
                        .is_some()
                    {
                        return Err("generated Python entry point clobbers a payload".into());
                    }
                }
            }
            NoArchLinks::Generic if !is_python => {}
            _ => return Err("noarch link metadata disagrees with frozen kind".into()),
        }
    }
    Ok(result)
}
