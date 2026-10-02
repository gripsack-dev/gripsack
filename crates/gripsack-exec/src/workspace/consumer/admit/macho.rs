//! Checked dyld lookup for retained Mach-O programs and extension bundles.
//! The copied main image loses its original origin. A per-launch DYLD plan is
//! admitted only when its basename overrides preserve every selected library.
//! This does not byte-pin dylibs or constrain arbitrary future dlopen calls.
use super::{BinaryAbi, EXECUTABLE_BYTES, ExecError, HostTarget, Package, Span, gate, operational};
use gripsack_policy::target::{OsRelease, TargetArch, TargetOs, release_at_most};
use gripsack_process::executable::{
    Endianness, ExecutableArch, ExecutableFormat, ExecutableMetadata, Interpreter, ObjectKind,
    WordClass, classify,
};

#[cfg(test)]
mod tests;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};

const MAX_OBJECTS: usize = 16_384;
const MAX_ENTRIES: usize = 200_000;

/// Apple-owned names must be real shared-cache members, not merely strings
/// under a broad system prefix. Non-Mac inspection only recognizes this small
/// explicit ABI list; it is not evidence of a runnable macOS installation.
pub(super) fn system_library(name: &str) -> bool {
    let path = Path::new(name);
    if !(path.starts_with("/usr/lib") || path.starts_with("/System/Library"))
        || name
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn _dyld_shared_cache_contains_path(path: *const libc::c_char) -> bool;
        }
        let Ok(name) = std::ffi::CString::new(name) else {
            return false;
        };
        // SAFETY: the query only reads this live NUL-terminated path.
        unsafe { _dyld_shared_cache_contains_path(name.as_ptr()) }
    }
    #[cfg(not(target_os = "macos"))]
    matches!(
        name,
        "/usr/lib/libSystem.B.dylib"
            | "/usr/lib/libobjc.A.dylib"
            | "/usr/lib/libc++.1.dylib"
            | "/usr/lib/libc++abi.dylib"
            | "/usr/lib/libresolv.9.dylib"
    )
}

pub(super) fn admit(
    metadata: &ExecutableMetadata,
    path: &Path,
    package: &Package,
    abi: Option<BinaryAbi>,
    host: &HostTarget,
    span: &Span,
) -> Result<Vec<PathBuf>, ExecError> {
    header(metadata, host, true, span)?;
    if abi != Some(BinaryAbi::Darwin) || host.abi != Some(BinaryAbi::Darwin) {
        return Err(gate(
            span,
            "Mach-O dynamic execution requires a declared Darwin ABI",
        ));
    }
    let mut roots = BTreeMap::new();
    let mut packages = vec![package];
    let mut seen = BTreeSet::new();
    while let Some(package) = packages.pop() {
        if !seen.insert(&package.root) {
            continue;
        }
        roots.insert(
            package
                .producer
                .payload
                .canonicalize()
                .map_err(operational)?,
            package.conda.is_some(),
        );
        packages.extend(package.runtime.iter().map(std::sync::Arc::as_ref));
    }
    let mut closure = Closure {
        roots,
        objects: BTreeMap::new(),
        host,
        span,
    };
    closure.load(path, true)?;
    // dlopen of Python extension modules is not represented by LC_LOAD_DYLIB.
    // Inventory the retained closure as well, rather than claiming the main
    // interpreter's startup graph proves compiled extensions compatible.
    closure.inventory()?;
    closure.plan(path)
}

fn header(
    metadata: &ExecutableMetadata,
    host: &HostTarget,
    launch: bool,
    span: &Span,
) -> Result<(), ExecError> {
    let arch = match host.arch {
        TargetArch::X86_64 => ExecutableArch::X86_64,
        TargetArch::Aarch64 => ExecutableArch::Aarch64,
    };
    if host.os != TargetOs::Macos
        || metadata.format != Some(ExecutableFormat::MachO)
        || metadata.class != Some(WordClass::SixtyFour)
        || metadata.arch != Some(arch)
        || metadata.endianness != Some(Endianness::Little)
    {
        return Err(gate(
            span,
            "Mach-O format/class/machine/byte order differs from the native target",
        ));
    }
    if metadata.object
        != Some(if launch {
            ObjectKind::Executable
        } else {
            ObjectKind::SharedObject
        })
    {
        return Err(gate(
            span,
            "Mach-O object has an incompatible program/library kind",
        ));
    }
    match &metadata.interpreter {
        Some(Interpreter::Loader(loader)) if launch && loader == "/usr/lib/dyld" => {}
        None if !launch => {}
        _ => {
            return Err(gate(
                span,
                "Mach-O object must use the platform dyld loader only",
            ));
        }
    }
    Ok(())
}

struct Closure<'a> {
    roots: BTreeMap<PathBuf, bool>,
    objects: BTreeMap<PathBuf, ExecutableMetadata>,
    host: &'a HostTarget,
    span: &'a Span,
}
impl Closure<'_> {
    fn owner(&self, path: &Path) -> Option<bool> {
        self.roots
            .iter()
            .find_map(|(root, conda)| path.starts_with(root).then_some(*conda))
    }
    fn load(&mut self, path: &Path, launch: bool) -> Result<(), ExecError> {
        if self.objects.contains_key(path) {
            return Ok(());
        }
        if self.objects.len() >= MAX_OBJECTS {
            return Err(gate(self.span, "Mach-O object inventory exceeds bound"));
        }
        let mut file = std::fs::File::open(path).map_err(operational)?;
        let status = file.metadata().map_err(operational)?;
        if !status.is_file() || status.len() > EXECUTABLE_BYTES {
            return Err(gate(
                self.span,
                "Mach-O object is not a bounded regular file",
            ));
        }
        let metadata = classify(&mut file)
            .map_err(|error| gate(self.span, format!("Mach-O {}: {error}", path.display())))?;
        header(&metadata, self.host, launch, self.span)?;
        controls(&mut file, launch, self.host, self.span)?;
        self.objects.insert(path.to_owned(), metadata);
        Ok(())
    }
    fn inventory(&mut self) -> Result<(), ExecError> {
        let mut pending: Vec<_> = self.roots.keys().cloned().collect();
        let mut count = 0;
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory).map_err(operational)? {
                let entry = entry.map_err(operational)?;
                count += 1;
                if count > MAX_ENTRIES {
                    return Err(gate(
                        self.span,
                        "Mach-O runtime inventory exceeds entry bound",
                    ));
                }
                let kind = entry.file_type().map_err(operational)?;
                if kind.is_dir() {
                    pending.push(entry.path());
                }
                if !kind.is_file() {
                    continue;
                }
                let path = entry.path();
                let mut file = std::fs::File::open(&path).map_err(operational)?;
                let mut magic = [0; 4];
                let count = file.read(&mut magic).map_err(operational)?;
                if count != 4
                    || !matches!(
                        magic,
                        [0xcf, 0xfa, 0xed, 0xfe]
                            | [0xce, 0xfa, 0xed, 0xfe]
                            | [0xfe, 0xed, 0xfa, 0xcf]
                            | [0xfe, 0xed, 0xfa, 0xce]
                    )
                {
                    if path.extension().is_some_and(|extension| {
                        matches!(extension.to_str(), Some("so" | "dylib" | "bundle"))
                    }) {
                        return Err(gate(
                            self.span,
                            format!(
                                "runtime extension {} is not a supported single-architecture Mach-O object",
                                path.display()
                            ),
                        ));
                    }
                    continue;
                }
                file.rewind().map_err(operational)?;
                let metadata = classify(&mut file).map_err(operational)?;
                // Other exported programs receive their own origin and launch
                // plan when selected; they are not dependencies of this one.
                if matches!(
                    metadata.object,
                    Some(ObjectKind::Executable | ObjectKind::Relocatable)
                ) {
                    continue;
                }
                self.load(&path.canonicalize().map_err(operational)?, false)?;
            }
        }
        Ok(())
    }
    fn expand(&self, text: &str, object: &Path, executable: &Path) -> Result<PathBuf, ExecError> {
        let fixed = Path::new(text).is_absolute();
        let path = if let Some(relative) = text.strip_prefix("@loader_path/") {
            object.parent().expect("absolute object").join(relative)
        } else if let Some(relative) = text.strip_prefix("@executable_path/") {
            executable
                .parent()
                .expect("absolute executable")
                .join(relative)
        } else if text == "@loader_path" {
            object.parent().expect("absolute object").to_owned()
        } else if text == "@executable_path" {
            executable.parent().expect("absolute executable").to_owned()
        } else if fixed {
            PathBuf::from(text)
        } else {
            return Err(gate(
                self.span,
                format!("unsupported or ambient Mach-O path {text:?}"),
            ));
        };
        if !fixed {
            let mut normalized = PathBuf::new();
            for component in path.components() {
                match component {
                    Component::ParentDir => {
                        if !normalized.pop() {
                            return Err(gate(self.span, "Mach-O path escapes root"));
                        }
                    }
                    Component::CurDir => {}
                    other => normalized.push(other),
                }
            }
            if self.owner(&normalized).is_none() {
                return Err(gate(
                    self.span,
                    "Mach-O origin path escapes retained runtime closure",
                ));
            }
        }
        // Resolve existing ancestors as well: a missing leaf must not hide a
        // symlinked directory that leads outside retained authority.
        let mut ancestor = path.as_path();
        loop {
            match ancestor.canonicalize() {
                Ok(real)
                    if self
                        .owner(&real)
                        .is_some_and(|materialized| !fixed || materialized) =>
                {
                    break;
                }
                Ok(_) => {
                    return Err(gate(
                        self.span,
                        "Mach-O path escapes retained authority or requires a validated final prefix",
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    ancestor = ancestor
                        .parent()
                        .ok_or_else(|| gate(self.span, "Mach-O path has no retained ancestor"))?;
                }
                Err(error) => return Err(operational(error)),
            }
        }
        // Keep the actual lookup spelling: lexically collapsing `link/..`
        // before resolving a symlink would select a different directory.
        Ok(path)
    }
    fn rpaths(
        &self,
        object: &Path,
        executable: &Path,
        inherited: &[PathBuf],
    ) -> Result<Vec<PathBuf>, ExecError> {
        let mut result = Vec::new();
        for path in &self.objects[object].runpaths {
            let text = path
                .to_str()
                .ok_or_else(|| gate(self.span, "Mach-O rpath is not UTF-8"))?;
            let path = self.expand(text, object, executable)?;
            if !result.contains(&path) {
                result.push(path);
            }
        }
        for path in inherited {
            if !result.contains(path) {
                result.push(path.clone());
            }
        }
        Ok(result)
    }
    fn resolve(
        &self,
        name: &str,
        object: &Path,
        executable: &Path,
        rpaths: &[PathBuf],
    ) -> Result<PathBuf, ExecError> {
        if system_library(name) {
            return Ok(PathBuf::from(name));
        }
        let candidates = if let Some(relative) = name.strip_prefix("@rpath/") {
            if relative.is_empty()
                || Path::new(relative)
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(gate(
                    self.span,
                    "Mach-O @rpath suffix escapes its lookup directory",
                ));
            }
            rpaths
                .iter()
                .map(|directory| directory.join(relative))
                .collect::<Vec<_>>()
        } else {
            vec![self.expand(name, object, executable)?]
        };
        let mut selected = None;
        for path in candidates {
            match path.canonicalize() {
                Ok(real) => {
                    if self.owner(&real).is_none() {
                        return Err(gate(
                            self.span,
                            "Mach-O library escapes retained runtime closure",
                        ));
                    }
                    if path
                        .parent()
                        .ok_or_else(|| gate(self.span, "Mach-O dependency has no parent"))?
                        .canonicalize()
                        .map_err(operational)?
                        != real.parent().expect("absolute library")
                    {
                        return Err(gate(
                            self.span,
                            "Mach-O dependency symlink changes its loader origin directory",
                        ));
                    }
                    if selected.as_ref().is_some_and(|previous| previous != &real) {
                        return Err(gate(
                            self.span,
                            format!("ambiguous Mach-O library lookup {name:?}"),
                        ));
                    }
                    selected = Some(real);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(operational(error)),
            }
        }
        selected.ok_or_else(|| gate(self.span, format!("Mach-O dependency {name:?} is absent from retained runtime closure and explicit Apple shared cache")))
    }
    fn plan(&mut self, executable: &Path) -> Result<Vec<PathBuf>, ExecError> {
        let main_rpaths = self.rpaths(executable, executable, &[])?;
        let mut pending: Vec<_> = self
            .objects
            .keys()
            .map(|path| (path.clone(), main_rpaths.clone(), BTreeSet::new()))
            .collect();
        let mut contexts = BTreeSet::new();
        let mut edges = BTreeMap::new();
        let mut directories = Vec::new();
        while let Some((object, inherited, mut ancestors)) = pending.pop() {
            if !ancestors.insert(object.clone()) {
                continue;
            }
            let rpaths = self.rpaths(&object, executable, &inherited)?;
            if !contexts.insert((object.clone(), rpaths.clone())) {
                continue;
            }
            if contexts.len() > MAX_OBJECTS * 4 {
                return Err(gate(self.span, "Mach-O dependency contexts exceed bound"));
            }
            for index in 0..self.objects[&object].needed_libraries.len() {
                let name = self.objects[&object].needed_libraries[index]
                    .to_str()
                    .ok_or_else(|| gate(self.span, "Mach-O install name is not UTF-8"))?;
                let selected = self.resolve(name, &object, executable, &rpaths)?;
                if let Some(previous) =
                    edges.insert((object.clone(), name.to_owned()), selected.clone())
                    && previous != selected
                {
                    return Err(gate(
                        self.span,
                        "Mach-O dependency selection changes with rpath ancestry",
                    ));
                }
                if system_library(name) {
                    continue;
                }
                if name.contains(".framework/") {
                    return Err(gate(
                        self.span,
                        "private Mach-O frameworks cannot preserve sealed-image lookup; use dylibs",
                    ));
                }
                let directory = selected.parent().expect("absolute library");
                if !directories.iter().any(|path| path == directory) {
                    directories.push(directory.to_owned());
                }
                self.load(&selected, false)?;
                pending.push((selected, rpaths.clone(), ancestors.clone()));
            }
        }
        // DYLD_LIBRARY_PATH precedes install-name lookup, including system
        // names. Every override must be exactly the object already admitted.
        for ((_, name), selected) in &edges {
            let base = Path::new(name)
                .file_name()
                .ok_or_else(|| gate(self.span, "Mach-O install name has no basename"))?;
            let mut override_path = None;
            for directory in &directories {
                match directory.join(base).canonicalize() {
                    Ok(path) => {
                        override_path = Some(path);
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(operational(error)),
                }
            }
            if override_path.as_ref().is_some_and(|path| path != selected)
                || (!system_library(name) && override_path.as_ref() != Some(selected))
            {
                return Err(gate(
                    self.span,
                    format!("sealed Mach-O execution would change library selection for {name:?}"),
                ));
            }
        }
        Ok(directories)
    }
}

/// Reject dyld modes that bypass the checked overlay, unsupported required
/// commands, specialized CPU subtypes, and signatures that enable library
/// validation/hardened runtime. Signed bytes remain unchanged by selection.
fn controls(
    file: &mut std::fs::File,
    launch: bool,
    host: &HostTarget,
    span: &Span,
) -> Result<(), ExecError> {
    file.rewind().map_err(operational)?;
    let mut header = [0u8; 32];
    file.read_exact(&mut header).map_err(operational)?;
    let file_length = file.metadata().map_err(operational)?.len();
    let word = |bytes: &[u8], at: usize| {
        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("bounded word"))
    };
    let cpu = word(&header, 4);
    let subtype = word(&header, 8);
    if (cpu == 0x0100_0007 && subtype != 3) || (cpu == 0x0100_000c && subtype != 0) {
        return Err(gate(
            span,
            "specialized Mach-O CPU subtype is not admitted on the generic target",
        ));
    }
    if word(&header, 24) & 0x100 != 0 {
        return Err(gate(
            span,
            "Mach-O flat namespace cannot preserve checked library selection",
        ));
    }
    if !matches!(word(&header, 12), 2 | 6 | 8) {
        return Err(gate(span, "unsupported Mach-O program/library file type"));
    }
    let length = word(&header, 20) as usize;
    if length > 64 * 1024 {
        return Err(gate(span, "Mach-O load command bytes exceed bound"));
    }
    let mut commands = vec![0; length];
    file.read_exact(&mut commands).map_err(operational)?;
    let mut at = 0;
    while at < commands.len() {
        // classify already checked command sizes against the declared region.
        let kind = word(&commands, at);
        let size = word(&commands, at + 4) as usize;
        let command = &commands[at..at + size];
        if kind & 0x8000_0000 != 0
            && !matches!(
                kind,
                0x8000_0018
                    | 0x8000_001c
                    | 0x8000_001f
                    | 0x8000_0022
                    | 0x8000_0023
                    | 0x8000_0028
                    | 0x8000_0033
                    | 0x8000_0034
            )
        {
            return Err(gate(span, "unsupported required Mach-O load command"));
        }
        if matches!(kind, 0x6 | 0x7 | 0xf | 0x10 | 0x17 | 0x20 | 0x27 | 0x2c) {
            return Err(gate(
                span,
                "Mach-O embedded loader environment or alternate runtime lookup is not admitted",
            ));
        }
        if matches!(kind, 0x25 | 0x2f | 0x30) {
            return Err(gate(
                span,
                "Mach-O object targets an Apple platform other than macOS",
            ));
        }
        if kind == 0x32 && (size < 24 || word(command, 8) != 1) {
            return Err(gate(
                span,
                "Mach-O build-version command does not target macOS",
            ));
        }
        if kind == 0x32 && (size - 24) / 8 != word(command, 20) as usize {
            return Err(gate(span, "Mach-O build tools exceed their command region"));
        }
        if (kind == 0x8000_0028 && size != 24)
            || (matches!(kind, 0x22 | 0x8000_0022) && size != 48)
            || (matches!(kind, 0x8000_0033 | 0x8000_0034) && size != 16)
        {
            return Err(gate(span, "invalid Mach-O loader command size"));
        }
        if matches!(kind, 0x24 | 0x32) {
            if (kind == 0x24 && size != 16) || (kind == 0x32 && size < 24) {
                return Err(gate(span, "truncated Mach-O minimum OS command"));
            }
            let encoded = word(command, if kind == 0x24 { 8 } else { 12 });
            let required = OsRelease {
                major: (encoded >> 16) as u16,
                minor: ((encoded >> 8) & 0xff) as u16,
                patch: (encoded & 0xff) as u16,
            };
            if host
                .requirement
                .minimum_os
                .is_none_or(|available| !release_at_most(required, available))
            {
                return Err(gate(
                    span,
                    format!(
                        "Mach-O minimum macOS {}.{}.{} exceeds the measured host capability",
                        required.major, required.minor, required.patch
                    ),
                ));
            }
        }
        if kind == 0x19 {
            if size < 72 {
                return Err(gate(span, "truncated Mach-O segment command"));
            }
            if (size - 72) / 80 != word(command, 64) as usize || (size - 72) % 80 != 0 {
                return Err(gate(
                    span,
                    "Mach-O sections exceed their segment command region",
                ));
            }
            let offset = u64::from_le_bytes(command[40..48].try_into().expect("segment offset"));
            let length = u64::from_le_bytes(command[48..56].try_into().expect("segment length"));
            if offset
                .checked_add(length)
                .is_none_or(|end| end > file_length)
            {
                return Err(gate(span, "Mach-O segment exceeds object bytes"));
            }
            if command[8..24].split(|byte| *byte == 0).next() == Some(b"__RESTRICT".as_slice()) {
                return Err(gate(
                    span,
                    "restricted Mach-O image ignores the checked DYLD plan",
                ));
            }
        }
        if kind == 0x1d {
            if size != 16 {
                return Err(gate(span, "invalid Mach-O signature command"));
            }
            let offset = u64::from(word(command, 8));
            let length = word(command, 12) as usize;
            if !(12..=16 * 1024 * 1024).contains(&length) {
                return Err(gate(span, "Mach-O signature exceeds bound"));
            }
            file.seek(SeekFrom::Start(offset)).map_err(operational)?;
            let mut signature = vec![0; length];
            file.read_exact(&mut signature).map_err(operational)?;
            signature_controls(&signature, launch, span)?;
        }
        at += size;
    }
    Ok(())
}
fn signature_controls(bytes: &[u8], launch: bool, span: &Span) -> Result<(), ExecError> {
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|bytes| u32::from_be_bytes(bytes.try_into().expect("word")))
            .ok_or_else(|| gate(span, "truncated Mach-O signature"))
    };
    let length = word(4)? as usize;
    if word(0)? != 0xfade0cc0 || !(12..=bytes.len()).contains(&length) {
        return Err(gate(span, "invalid Mach-O signature superblob"));
    }
    let count = word(8)? as usize;
    if count > (length - 12) / 8 {
        return Err(gate(span, "invalid Mach-O signature index"));
    }
    for index in 0..count {
        let offset = word(16 + index * 8)? as usize;
        let blob_length = word(offset + 4)? as usize;
        if offset < 12 + count * 8
            || blob_length < 8
            || offset
                .checked_add(blob_length)
                .is_none_or(|end| end > length)
        {
            return Err(gate(
                span,
                "Mach-O signature blob escapes its declared region",
            ));
        }
        if word(offset)? == 0xfade0c02 {
            if blob_length < 16 {
                return Err(gate(span, "truncated Mach-O code directory"));
            }
            let flags = word(offset + 12)?;
            if launch && flags & (0x10000 | 0x2000 | 0x800) != 0 {
                return Err(gate(
                    span,
                    "hardened/library-validated Mach-O requires a signed origin-preserving launcher; DYLD translation is unavailable",
                ));
            }
        }
    }
    Ok(())
}
