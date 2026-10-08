//! Admit the complete selected ELF dependency graph before native execution.
//! Retained package roots and explicit host roots are separate authorities.
//! RPATH ancestry remains distinct from RUNPATH and the launch overlay.
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
    if !metadata.elf_loader_extensions.is_empty() {
        return Err(gate(
            span,
            "ELF audit/filter dependencies are outside the admitted DT_NEEDED closure",
        ));
    }
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
            &closure,
            span,
        )?,
        runpaths: origin_paths(
            &metadata.runpaths,
            path,
            payload,
            package.conda.is_some(),
            &closure,
            span,
        )?,
        parent: None,
    }];
    let mut visited: std::collections::BTreeMap<PathBuf, LoadedLibrary> =
        std::collections::BTreeMap::new();
    let mut index = 0;
    while index < objects.len() {
        for needed in 0..objects[index].metadata.needed_libraries.len() {
            let name = objects[index].metadata.needed_libraries[needed]
                .to_str()
                .filter(|name| !name.is_empty() && !name.contains(['/', '$']))
                .ok_or_else(|| {
                    gate(
                        span,
                        "DT_NEEDED must name a runtime library, not a path or loader token",
                    )
                })?;
            let selected = resolve_library(&objects, index, &mut closure, name, None, span)?;
            if let Some(loaded) = visited.get(&selected.canonical) {
                if loaded.origin_sensitive && selected.origin != loaded.origin {
                    return Err(gate(
                        span,
                        "runtime object is selected through conflicting loader origins",
                    ));
                }
                continue;
            }
            let mut file = std::fs::File::open(&selected.canonical).map_err(operational)?;
            let status = file.metadata().map_err(operational)?;
            if !status.is_file() || status.len() > EXECUTABLE_BYTES {
                return Err(gate(
                    span,
                    format!(
                        "runtime library {} is not a bounded regular file",
                        selected.spelling.display()
                    ),
                ));
            }
            let metadata = classify(&mut file).map_err(|error| {
                gate(
                    span,
                    format!("runtime library {}: {error}", selected.spelling.display()),
                )
            })?;
            header(&metadata, host, false, span)?;
            if !metadata.elf_loader_extensions.is_empty() {
                return Err(gate(
                    span,
                    "runtime ELF audit/filter dependencies are outside the admitted DT_NEEDED closure",
                ));
            }
            if metadata.interpreter.as_ref().is_some_and(
                |interpreter| !matches!(interpreter, Interpreter::Loader(named) if named == loader),
            ) {
                return Err(gate(
                    span,
                    format!(
                        "runtime library {} names a foreign loader",
                        selected.spelling.display()
                    ),
                ));
            }
            let owner = closure
                .owner(&selected.canonical)
                .ok_or_else(|| gate(span, "runtime library escaped the selected closure"))?;
            let rpaths = origin_paths(
                &metadata.rpaths,
                &selected.spelling,
                owner,
                closure.materialized(&selected.canonical),
                &closure,
                span,
            )?;
            let runpaths = origin_paths(
                &metadata.runpaths,
                &selected.spelling,
                owner,
                closure.materialized(&selected.canonical),
                &closure,
                span,
            )?;
            let origin_sensitive = metadata
                .rpaths
                .iter()
                .chain(&metadata.runpaths)
                .any(|entry| {
                    entry.to_str().is_some_and(|entry| {
                        entry.contains("$ORIGIN") || entry.contains("${ORIGIN}")
                    })
                });
            objects.push(Object {
                metadata: Cow::Owned(metadata),
                rpaths,
                runpaths,
                parent: Some(index),
            });
            visited.insert(
                selected.canonical,
                LoadedLibrary {
                    spelling: selected.spelling,
                    origin: selected.origin,
                    origin_sensitive,
                },
            );
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
    for loaded in visited.values() {
        let parent = loaded
            .spelling
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
            let origin_sensitive = visited
                .get(&original.canonical)
                .is_some_and(|loaded| loaded.origin_sensitive);
            if original.canonical != copied.canonical
                || (origin_sensitive && !original.same_origin(&copied))
            {
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

/// GNU expands a dependency's $ORIGIN from the pathname used to load it,
/// not from the symlink-resolved identity of the bytes. Keep both throughout
/// graph admission and launch equivalence.
#[derive(Clone)]
pub(super) struct SelectedLibrary {
    pub(super) spelling: PathBuf,
    pub(super) canonical: PathBuf,
    // Resolve the loading directory, never take the parent of canonical bytes.
    // This accepts equivalent directory aliases and Conda's lib/../lib paths.
    origin: PathBuf,
}
impl SelectedLibrary {
    pub(super) fn new(spelling: PathBuf) -> std::io::Result<Self> {
        let canonical = spelling.canonicalize()?;
        let origin = spelling
            .parent()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "runtime library has no loading directory",
                )
            })?
            .canonicalize()?;
        Ok(Self {
            spelling,
            canonical,
            origin,
        })
    }
    fn same_origin(&self, other: &Self) -> bool {
        self.canonical == other.canonical && self.origin == other.origin
    }
}

struct LoadedLibrary {
    spelling: PathBuf,
    origin: PathBuf,
    origin_sensitive: bool,
}

struct Object<'a> {
    metadata: Cow<'a, ExecutableMetadata>,
    rpaths: Vec<PathBuf>,
    runpaths: Vec<PathBuf>,
    parent: Option<usize>,
}
struct RetainedRuntimeRoot {
    path: PathBuf,
    materialized: bool,
}
/// A declaration grants this host directory, not the retained store namespace.
/// Preserve the spelling to validate aliases before canonical containment.
struct AdmittedHostRoot {
    declared: PathBuf,
    canonical: PathBuf,
}
struct Closure<'a> {
    roots: Vec<RetainedRuntimeRoot>,
    host_roots: Vec<AdmittedHostRoot>,
    library_dirs: Vec<PathBuf>,
    system: std::collections::BTreeMap<String, SelectedLibrary>,
    host: &'a HostTarget,
}
impl<'a> Closure<'a> {
    fn new(package: &Package, host: &'a HostTarget, span: &Span) -> Result<Self, ExecError> {
        let ordered_runtime = package.host_dependent();
        let mut packages = vec![package];
        let mut roots: Vec<RetainedRuntimeRoot> = Vec::new();
        let mut visited = BTreeSet::new();
        let mut index = 0;
        let mut system = std::collections::BTreeMap::new();
        let mut host_roots: Vec<AdmittedHostRoot> = Vec::new();
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
                roots.push(RetainedRuntimeRoot {
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
            if let Some(policy) = &package.host_runtime {
                if host.os != TargetOs::Linux || host.abi != Some(BinaryAbi::Gnu) {
                    return Err(gate(
                        span,
                        "host runtime directories require a native Linux GNU target",
                    ));
                }
                for directory in &policy.library_directories {
                    let declared = PathBuf::from(directory.as_str());
                    admit_search_directory(&declared, host.arch, span)?;
                    let canonical = declared.canonicalize().map_err(|error| {
                        gate(
                            span,
                            format!(
                                "declared host runtime directory {}: {error}",
                                declared.display()
                            ),
                        )
                    })?;
                    if !canonical.is_dir() {
                        return Err(gate(span, "declared host runtime root is not a directory"));
                    }
                    if canonical.parent().is_none() {
                        return Err(gate(
                            span,
                            "host runtime directory alias must not grant the filesystem root",
                        ));
                    }
                    admit_search_directory(&canonical, host.arch, span)?;
                    if let Some(existing) =
                        host_roots.iter().find(|root| root.canonical == canonical)
                    {
                        if existing.declared == declared {
                            continue;
                        }
                        return Err(gate(
                            span,
                            "declared host runtime directories contain canonical aliases",
                        ));
                    }
                    host_roots.push(AdmittedHostRoot {
                        declared,
                        canonical,
                    });
                }
            }
            let children = packages.len();
            packages.extend(package.runtime.iter().map(std::sync::Arc::as_ref));
            if ordered_runtime {
                // Runtime edges are identity sets and retained receipts restore
                // digest order. Use the same root-first BFS live and on replay.
                packages[children..].sort_unstable_by_key(|package| package.identity);
            }
        }
        let mut closure = Self {
            roots,
            host_roots,
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
        for root in &closure.host_roots {
            if !closure.library_dirs.contains(&root.canonical) {
                closure.library_dirs.push(root.canonical.clone());
            }
        }
        Ok(closure)
    }
    fn host_owner(&self, path: &Path) -> Option<&AdmittedHostRoot> {
        self.host_roots
            .iter()
            .filter(|root| path.starts_with(&root.canonical))
            .max_by_key(|root| root.canonical.components().count())
    }
    fn owner(&self, path: &Path) -> Option<&Path> {
        self.roots
            .iter()
            .find(|root| path.starts_with(&root.path))
            .map(|root| root.path.as_path())
            .or_else(|| self.host_owner(path).map(|root| root.canonical.as_path()))
            .or_else(|| {
                self.system
                    .values()
                    .find(|library| library.canonical == path)
                    .and_then(|library| library.canonical.parent())
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
    ) -> Result<Option<SelectedLibrary>, ExecError> {
        for directory in directories {
            if self.host.abi == Some(BinaryAbi::Gnu) {
                admit_search_directory(directory, self.host.arch, span)?;
            }
            let canonical_directory = directory.canonicalize().map_err(operational)?;
            let candidate = directory.join(name);
            match candidate.canonicalize() {
                Ok(path) => {
                    if let Some(root) = self.host_owner(&canonical_directory)
                        && !path.starts_with(&root.canonical)
                    {
                        return Err(gate(
                            span,
                            "host runtime library alias escapes its declared root",
                        ));
                    }
                    if self.owner(&path).is_none() {
                        return Err(gate(
                            span,
                            format!(
                                "runtime library {name:?} redirects outside the selected closure"
                            ),
                        ));
                    }
                    return Ok(Some(SelectedLibrary {
                        spelling: candidate,
                        canonical: path,
                        origin: canonical_directory,
                    }));
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
) -> Result<SelectedLibrary, ExecError> {
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
    if (closure.roots.iter().any(|root| root.materialized) || !closure.host_roots.is_empty())
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
    closure: &Closure<'_>,
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
            if entry.contains(';') || suffix.unwrap_or(entry).contains('$') {
                return Err(gate(
                    span,
                    "ELF search path contains an unsupported loader token or delimiter",
                ));
            }
            let candidate = match suffix {
                Some(suffix) => parent.join(suffix),
                None if (materialized || !closure.host_roots.is_empty())
                    && Path::new(entry).is_absolute() =>
                {
                    PathBuf::from(entry)
                }
                None => {
                    return Err(gate(
                        span,
                        format!("ELF search path {entry:?} is not package-relative $ORIGIN"),
                    ));
                }
            };
            let host_root = closure
                .host_roots
                .iter()
                .filter(|root| {
                    candidate.starts_with(&root.declared) || candidate.starts_with(&root.canonical)
                })
                .max_by_key(|root| root.canonical.components().count());
            let boundary = host_root.map_or(payload, |root| root.canonical.as_path());
            if let Some(directory) = search_directory(&candidate, boundary, span)?
                && !paths.contains(&directory)
            {
                admit_search_directory(&directory, closure.host.arch, span)?;
                paths.push(directory);
            }
        }
    }
    Ok(paths)
}

/// GNU expands tokens and both ':' and ';' in its search argument. In
/// addition, the pre-2.37 legacy hwcap lookup is independent of the modern
/// --glibc-hwcaps-mask flag. Refuse capability subtrees rather than admit a
/// flat graph that the real loader may silently override on another CPU.
fn admit_search_directory(
    directory: &Path,
    arch: TargetArch,
    span: &Span,
) -> Result<(), ExecError> {
    use std::os::unix::ffi::OsStrExt;
    if directory
        .as_os_str()
        .as_bytes()
        .iter()
        .any(|byte| matches!(*byte, b':' | b';' | b'$'))
    {
        return Err(gate(
            span,
            "runtime search directory contains a GNU loader delimiter or token",
        ));
    }
    let capabilities: &[&str] = match arch {
        TargetArch::X86_64 => &[
            "tls", "sse2", "x86_64", "avx512_1", "i586", "i686", "haswell", "xeon_phi",
        ],
        // With sanitized loader environment, AArch64's HWCAP_IMPORTANT is
        // ATOMICS, in addition to the kernel platform string and TLS.
        TargetArch::Aarch64 => &["tls", "aarch64", "atomics"],
    };
    for capability in capabilities {
        let candidate = directory.join(capability);
        match std::fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(operational(error)),
            Ok(metadata) => {
                // Some OS packages reserve empty capability directories.
                // They cannot override a lookup. A populated tree or alias
                // needs a different graph policy; do not flatten it silently.
                if metadata.is_dir() {
                    if std::fs::read_dir(&candidate)
                        .map_err(operational)?
                        .next()
                        .transpose()
                        .map_err(operational)?
                        .is_none()
                    {
                        continue;
                    }
                } else if !metadata.file_type().is_symlink() {
                    continue;
                }
                return Err(gate(
                    span,
                    format!(
                        "runtime search directory contains unadmitted legacy GNU capability entry {}",
                        candidate.display()
                    ),
                ));
            }
        }
    }
    Ok(())
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
                return Ok((ancestor == candidate).then(|| candidate.to_owned()));
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
