//! Admit the complete selected ELF dependency graph before native execution.
//! Library lookup is confined to retained runtime packages. RPATH ancestry is
//! distinct from RUNPATH and from the launch's explicit library overlay.
use super::{
    BinaryAbi, EXECUTABLE_BYTES, ExecError, HostTarget, Package, Span, gate, operational,
    platform_loader,
};
use gripsack_policy::target::{TargetArch, TargetOs};
use gripsack_process::executable::{
    Endianness, ExecutableArch, ExecutableFormat, ExecutableMetadata, Interpreter, ObjectKind,
    WordClass, classify,
};
use std::{
    borrow::Cow,
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct Runtime {
    pub directories: Vec<PathBuf>,
    pub gnu_loader: Option<&'static str>,
}
pub(super) fn admit(
    metadata: &ExecutableMetadata,
    path: &Path,
    payload: &Path,
    package: &Package,
    declared_abi: Option<BinaryAbi>,
    host: &HostTarget,
    span: &Span,
) -> Result<Runtime, ExecError> {
    header(metadata, host, true, span)?;
    if metadata.interpreter.is_none()
        && metadata.needed_libraries.is_empty()
        && metadata.rpaths.is_empty()
        && metadata.runpaths.is_empty()
    {
        return Ok(Runtime::default());
    }
    let abi = declared_abi.ok_or_else(|| {
        gate(
            span,
            "dynamic executable has no declared ABI; an omitted ABI is not a wildcard",
        )
    })?;
    let loader = platform_loader(host.os, host.arch, abi)
        .ok_or_else(|| gate(span, format!("no admitted platform loader for {abi:?}")))?;
    match &metadata.interpreter {
        Some(Interpreter::Loader(named)) if named == loader => {}
        _ => {
            return Err(gate(
                span,
                format!("dynamic executable must name the platform loader {loader:?}"),
            ));
        }
    }
    let gnu_loader = if abi == BinaryAbi::Gnu {
        if !host.gnu_loader_controls {
            return Err(gate(
                span,
                "sealed GNU dynamic execution requires measured glibc >=2.33 loader controls",
            ));
        }
        Some(loader)
    } else {
        if !metadata.rpaths.is_empty() || !metadata.runpaths.is_empty() {
            return Err(gate(
                span,
                "this native loader cannot preserve a copied executable's origin paths",
            ));
        }
        None
    };
    let mut closure = Closure::new(package, host, span)?;
    let mut objects = vec![Object {
        metadata: Cow::Borrowed(metadata),
        rpaths: origin_paths(
            &metadata.rpaths,
            path,
            payload,
            package.conda.is_some(),
            span,
        )?,
        runpaths: origin_paths(
            &metadata.runpaths,
            path,
            payload,
            package.conda.is_some(),
            span,
        )?,
        parent: None,
    }];
    let mut visited = BTreeSet::new();
    let mut index = 0;
    while index < objects.len() {
        for needed in 0..objects[index].metadata.needed_libraries.len() {
            let name = objects[index].metadata.needed_libraries[needed]
                .to_str()
                .filter(|name| !name.is_empty() && !name.contains('/'))
                .ok_or_else(|| gate(span, "DT_NEEDED must name a runtime library, not a path"))?;
            let selected = resolve_library(&objects, index, &mut closure, name, None, span)?;
            if visited.contains(&selected) {
                continue;
            }
            let mut file = std::fs::File::open(&selected).map_err(operational)?;
            let status = file.metadata().map_err(operational)?;
            if !status.is_file() || status.len() > EXECUTABLE_BYTES {
                return Err(gate(
                    span,
                    format!(
                        "runtime library {} is not a bounded regular file",
                        selected.display()
                    ),
                ));
            }
            let metadata = classify(&mut file).map_err(|error| {
                gate(
                    span,
                    format!("runtime library {}: {error}", selected.display()),
                )
            })?;
            header(&metadata, host, false, span)?;
            if metadata.interpreter.as_ref().is_some_and(
                |interpreter| !matches!(interpreter, Interpreter::Loader(named) if named == loader),
            ) {
                return Err(gate(
                    span,
                    format!(
                        "runtime library {} names a foreign loader",
                        selected.display()
                    ),
                ));
            }
            let owner = closure
                .owner(&selected)
                .ok_or_else(|| gate(span, "runtime library escaped the selected closure"))?;
            let rpaths = origin_paths(
                &metadata.rpaths,
                &selected,
                owner,
                closure.materialized(&selected),
                span,
            )?;
            let runpaths = origin_paths(
                &metadata.runpaths,
                &selected,
                owner,
                closure.materialized(&selected),
                span,
            )?;
            objects.push(Object {
                metadata: Cow::Owned(metadata),
                rpaths,
                runpaths,
                parent: Some(index),
            });
            visited.insert(selected);
        }
        index += 1;
    }
    // Sealing changes the main object's origin. Expand its paths into a checked
    // loader plan and disable only that object's embedded RPATH/RUNPATH. Every
    // dependency must still resolve to exactly the original admitted file.
    let mut directories = Vec::new();
    if objects[0].runpaths.is_empty() {
        directories.extend(objects[0].rpaths.iter().cloned());
    }
    for path in closure.library_dirs.iter().chain(&objects[0].runpaths) {
        if !directories.contains(path) {
            directories.push(path.clone());
        }
    }
    // Include every resolved dependency directory ahead of any caller-supplied
    // search segment. Ambiguous flattening is refused by the equivalence pass.
    for path in &visited {
        let parent = path
            .parent()
            .ok_or_else(|| gate(span, "runtime library has no parent"))?;
        if !directories.iter().any(|path| path == parent) {
            directories.push(parent.to_owned());
        }
    }
    for (index, object) in objects.iter().enumerate() {
        for name in &object.metadata.needed_libraries {
            let name = name
                .to_str()
                .ok_or_else(|| gate(span, "library name is not UTF-8"))?;
            let original = resolve_library(&objects, index, &mut closure, name, None, span)?;
            let copied = resolve_library(
                &objects,
                index,
                &mut closure,
                name,
                Some(&directories),
                span,
            )?;
            if original != copied {
                return Err(gate(
                    span,
                    format!("sealed execution would change the resolution of library {name:?}"),
                ));
            }
        }
    }
    Ok(Runtime {
        directories,
        gnu_loader,
    })
}

pub(super) fn header(
    metadata: &ExecutableMetadata,
    host: &HostTarget,
    launch: bool,
    span: &Span,
) -> Result<(), ExecError> {
    let architecture = match host.arch {
        TargetArch::X86_64 => ExecutableArch::X86_64,
        TargetArch::Aarch64 => ExecutableArch::Aarch64,
    };
    if host.os != TargetOs::Linux
        || metadata.format != Some(ExecutableFormat::Elf)
        || metadata.class != Some(WordClass::SixtyFour)
        || metadata.arch != Some(architecture)
        || metadata.endianness != Some(Endianness::Little)
    {
        return Err(gate(
            span,
            "ELF format/class/machine/byte order differs from the native target",
        ));
    }
    if launch {
        if !matches!(
            metadata.object,
            Some(ObjectKind::Executable | ObjectKind::SharedObject)
        ) || !metadata.has_entry_point
        {
            return Err(gate(span, "ELF object is not an executable program"));
        }
    } else if metadata.object != Some(ObjectKind::SharedObject) {
        return Err(gate(span, "runtime library is not an ELF shared object"));
    }
    Ok(())
}

struct Object<'a> {
    metadata: Cow<'a, ExecutableMetadata>,
    rpaths: Vec<PathBuf>,
    runpaths: Vec<PathBuf>,
    parent: Option<usize>,
}
struct RuntimeRoot {
    path: PathBuf,
    materialized: bool,
}
struct Closure<'a> {
    roots: Vec<RuntimeRoot>,
    library_dirs: Vec<PathBuf>,
    system: std::collections::BTreeMap<String, PathBuf>,
    host: &'a HostTarget,
}
impl<'a> Closure<'a> {
    fn new(package: &Package, host: &'a HostTarget, span: &Span) -> Result<Self, ExecError> {
        let mut packages = vec![package];
        let mut roots: Vec<RuntimeRoot> = Vec::new();
        let mut visited = BTreeSet::new();
        let mut index = 0;
        let mut system = std::collections::BTreeMap::new();
        while index < packages.len() {
            let package = packages[index];
            index += 1;
            if !visited.insert(&package.root) {
                continue;
            }
            let root = package
                .producer
                .payload
                .canonicalize()
                .map_err(operational)?;
            if !roots.iter().any(|existing| existing.path == root) {
                roots.push(RuntimeRoot {
                    path: root,
                    materialized: package.conda.is_some(),
                });
            }
            if let Some(receipt) = &package.conda {
                for name in &receipt.system.libraries {
                    if !system.contains_key(name)
                        && let Some(path) = super::platform::library(name, host, span)?
                    {
                        system.insert(name.clone(), path);
                    }
                }
            }
            packages.extend(package.runtime.iter().map(std::sync::Arc::as_ref));
        }
        let mut closure = Self {
            roots,
            library_dirs: Vec::new(),
            system,
            host,
        };
        for root in &closure.roots {
            for name in ["lib", "lib64"] {
                let path = root.path.join(name);
                match std::fs::metadata(&path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(operational(error)),
                    Ok(metadata) if !metadata.is_dir() => continue,
                    Ok(_) => {}
                }
                let path = path.canonicalize().map_err(operational)?;
                if closure.owner(&path).is_none() {
                    return Err(gate(
                        span,
                        "runtime library directory escapes the selected closure",
                    ));
                }
                if !closure.library_dirs.contains(&path) {
                    closure.library_dirs.push(path);
                }
            }
        }
        Ok(closure)
    }
    fn owner(&self, path: &Path) -> Option<&Path> {
        self.roots
            .iter()
            .find(|root| path.starts_with(&root.path))
            .map(|root| root.path.as_path())
            .or_else(|| {
                self.system
                    .values()
                    .find(|library| library.as_path() == path)
                    .and_then(|path| path.parent())
            })
    }
    fn materialized(&self, path: &Path) -> bool {
        self.roots
            .iter()
            .any(|root| root.materialized && path.starts_with(&root.path))
    }
    fn lookup(
        &self,
        directories: &[PathBuf],
        name: &str,
        span: &Span,
    ) -> Result<Option<PathBuf>, ExecError> {
        for directory in directories {
            let candidate = directory.join(name);
            match candidate.canonicalize() {
                Ok(path) => {
                    if self.owner(&path).is_none() {
                        return Err(gate(
                            span,
                            format!(
                                "runtime library {name:?} redirects outside the selected closure"
                            ),
                        ));
                    }
                    return Ok(Some(path));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(gate(span, format!("runtime library {name:?}: {error}"))),
            }
        }
        Ok(None)
    }
}
fn resolve_library(
    objects: &[Object<'_>],
    index: usize,
    closure: &mut Closure<'_>,
    name: &str,
    launch_paths: Option<&[PathBuf]>,
    span: &Span,
) -> Result<PathBuf, ExecError> {
    let object = &objects[index];
    let suppress_main = launch_paths.is_some();
    if object.runpaths.is_empty() || (index == 0 && suppress_main) {
        let mut ancestor = Some(index);
        while let Some(index) = ancestor {
            let object = &objects[index];
            if !(index == 0 && suppress_main)
                && object.runpaths.is_empty()
                && let Some(path) = closure.lookup(&object.rpaths, name, span)?
            {
                return Ok(path);
            }
            ancestor = object.parent;
        }
    }
    if let Some(path) = closure.lookup(launch_paths.unwrap_or(&closure.library_dirs), name, span)? {
        return Ok(path);
    }
    if !(index == 0 && suppress_main)
        && let Some(path) = closure.lookup(&object.runpaths, name, span)?
    {
        return Ok(path);
    }
    if let Some(path) = closure.system.get(name) {
        return Ok(path.clone());
    }
    if closure.roots.iter().any(|root| root.materialized)
        && let Some(path) = super::platform::library(name, closure.host, span)?
    {
        closure.system.insert(name.into(), path.clone());
        return Ok(path);
    }
    Err(gate(
        span,
        format!("needed library {name:?} is absent from the retained runtime closure"),
    ))
}
fn origin_paths(
    entries: &[OsString],
    path: &Path,
    payload: &Path,
    materialized: bool,
    span: &Span,
) -> Result<Vec<PathBuf>, ExecError> {
    let parent = path
        .parent()
        .ok_or_else(|| gate(span, "ELF object has no parent directory"))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry
            .to_str()
            .ok_or_else(|| gate(span, "ELF search path is not UTF-8"))?;
        for entry in entry.split(':') {
            let suffix = if entry == "$ORIGIN" || entry == "${ORIGIN}" {
                Some("")
            } else {
                entry
                    .strip_prefix("$ORIGIN/")
                    .or_else(|| entry.strip_prefix("${ORIGIN}/"))
            };
            let candidate = match suffix {
                Some(suffix) => parent.join(suffix),
                None if materialized && Path::new(entry).is_absolute() => PathBuf::from(entry),
                None => {
                    return Err(gate(
                        span,
                        format!("ELF search path {entry:?} is not package-relative $ORIGIN"),
                    ));
                }
            };
            if let Some(directory) = search_directory(&candidate, payload, span)?
                && !paths.contains(&directory)
            {
                paths.push(directory);
            }
        }
    }
    Ok(paths)
}

/// Loaders skip absent search candidates. Prove the nearest existing ancestor
/// is still inside the package before doing so; absence never grants a future
/// search path outside the retained root or through an unresolved symlink.
fn search_directory(
    candidate: &Path,
    payload: &Path,
    span: &Span,
) -> Result<Option<PathBuf>, ExecError> {
    for ancestor in candidate.ancestors() {
        match ancestor.canonicalize() {
            Ok(directory) => {
                if !directory.starts_with(payload) || !directory.is_dir() {
                    return Err(gate(
                        span,
                        "ELF search path is not a directory inside its package",
                    ));
                }
                return Ok((ancestor == candidate).then_some(directory));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::symlink_metadata(ancestor) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(operational(error)),
                    Ok(_) => {
                        return Err(gate(
                            span,
                            "ELF search path contains an unresolved or concurrently changed entry",
                        ));
                    }
                }
            }
            Err(error) => {
                return Err(gate(
                    span,
                    format!("ELF search path {}: {error}", candidate.display()),
                ));
            }
        }
    }
    Err(gate(
        span,
        "ELF search path has no admitted package ancestor",
    ))
}
