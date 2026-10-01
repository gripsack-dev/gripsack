//! Object-local ELF lookup, including inherited RPATH (but never inherited
//! RUNPATH). All candidates come from the independently verified image inventory.
#[cfg(test)]
mod tests;
use super::{Namespace, reject, text};
use crate::{oci::{FileKind, OciError, resolve}, plan::Architecture};
use gripsack_process::executable::{Endianness, ExecutableArch, ExecutableFormat, ExecutableMetadata, Interpreter, ObjectKind, WordClass};
use std::collections::{BTreeMap, BTreeSet};

struct Object<'a> {
    path:&'a str,
    layout:&'a ExecutableMetadata,
    rpaths:Vec<String>,
    runpaths:Vec<String>,
    launch:bool,
}
struct Frame<'a> {
    object:&'a Object<'a>,
    parent:Option<usize>,
}

pub(super) fn validate(namespace: &Namespace<'_>) -> Result<(),OciError> {
    let mut objects = BTreeMap::new();
    for (path, entry) in namespace.files {
        let FileKind::File { digest, .. } = entry.kind else { continue; };
        let Some(Ok(layout)) = namespace.metadata.get(&digest) else { continue; };
        if layout.format != Some(ExecutableFormat::Elf) {
            if entry.mode & 0o111 != 0 && layout.format == Some(ExecutableFormat::MachO) {
                return Err(reject(path,"Mach-O cannot run in a Linux image"));
            }
            continue;
        }
        if !matches!(layout.object,Some(ObjectKind::Executable | ObjectKind::SharedObject)) { continue; }
        let launch = entry.mode & 0o111 != 0 && layout.has_entry_point;
        entry_header(namespace,path,layout,launch)?;
        let mut rpaths = Vec::new();
        let mut runpaths = Vec::new();
        namespace.search_paths(path,&layout.rpaths,&mut rpaths)?;
        namespace.search_paths(path,&layout.runpaths,&mut runpaths)?;
        objects.insert(path.as_str(),Object { path,layout,rpaths,runpaths,launch });
    }
    let mut library_dirs = Vec::new();
    if let Some(value) = namespace.environment("LD_LIBRARY_PATH").filter(|value| !value.is_empty()) {
        for directory in value.split(':') { library_dirs.push(namespace.directory(directory,"LD_LIBRARY_PATH")?); }
    }
    let mut reached = BTreeSet::new();
    for object in objects.values().filter(|object| object.launch) {
        closure(namespace,&objects,&library_dirs,object,&mut reached)?;
    }
    // Extension modules can be loaded through dlopen rather than DT_NEEDED.
    // Unreached objects must therefore carry a complete standalone closure.
    for object in objects.values() {
        if !reached.contains(object.path) {
            closure(namespace,&objects,&library_dirs,object,&mut reached)?;
        }
    }
    Ok(())
}
fn closure<'a>(namespace: &Namespace<'_>,objects: &'a BTreeMap<&str,Object<'a>>,library_dirs: &[String],root: &'a Object<'a>,reached: &mut BTreeSet<&'a str>) -> Result<(),OciError> {
    reached.insert(root.path);
    if root.layout.needed_libraries.is_empty() { return Ok(()); }
    let mut frames = vec![Frame { object:root,parent:None }];
    let mut visited = BTreeSet::from([root.path]);
    let mut index = 0;
    while index < frames.len() {
        reached.insert(frames[index].object.path);
        for needed in &frames[index].object.layout.needed_libraries {
            let requester = frames[index].object.path;
            let needed = text(needed,requester)?;
            let (path,entry) = dependency(namespace,&frames,index,library_dirs,needed)?;
            let metadata = namespace.layout(path,entry)?;
            if metadata.object != Some(ObjectKind::SharedObject) {
                return Err(reject(path,"runtime library is not a compatible shared object"));
            }
            let object = objects.get(path).ok_or_else(|| reject(path,"runtime library is not an admitted ELF object"))?;
            if visited.insert(path) { frames.push(Frame { object,parent:Some(index) }); }
        }
        index += 1;
    }
    Ok(())
}
fn dependency<'a>(namespace: &Namespace<'a>,frames: &[Frame<'_>],index: usize,library_dirs: &[String],needed: &str) -> Result<(&'a str,&'a crate::oci::FileEntry),OciError> {
    let object = frames[index].object;
    if needed.starts_with('/') { return resolve(namespace.files,needed); }
    if needed.is_empty() || needed.contains('/') { return Err(reject(object.path,"DT_NEEDED is not a library name or absolute image path")); }
    if object.runpaths.is_empty() {
        let mut ancestor = Some(index);
        while let Some(index) = ancestor {
            let frame = &frames[index];
            if frame.object.runpaths.is_empty()
                && let Some(selected) = lookup(namespace,&frame.object.rpaths,needed) { return Ok(selected); }
            ancestor = frame.parent;
        }
    }
    if let Some(selected) = lookup(namespace,library_dirs,needed) { return Ok(selected); }
    if let Some(selected) = lookup(namespace,&object.runpaths,needed) { return Ok(selected); }
    let defaults: &[&str] = match namespace.platform.architecture {
        Architecture::Amd64 => &["/lib/x86_64-linux-gnu","/usr/lib/x86_64-linux-gnu","/lib64","/usr/lib64","/lib","/usr/lib"],
        Architecture::Arm64 => &["/lib/aarch64-linux-gnu","/usr/lib/aarch64-linux-gnu","/lib64","/usr/lib64","/lib","/usr/lib"],
    };
    for directory in defaults {
        if let Ok(selected) = resolve(namespace.files,&format!("{directory}/{needed}")) { return Ok(selected); }
    }
    Err(OciError::Runtime { path:object.path.into(),detail:format!("unresolved runtime library {needed:?}") })
}
fn lookup<'a>(namespace: &Namespace<'a>,directories: &[String],needed: &str) -> Option<(&'a str,&'a crate::oci::FileEntry)> {
    directories.iter().find_map(|directory| resolve(namespace.files,&format!("{directory}/{needed}")).ok())
}

pub(super) fn entry_header(namespace: &Namespace<'_>,path: &str,layout: &ExecutableMetadata,launch: bool) -> Result<(),OciError> {
    let architecture = match namespace.platform.architecture { Architecture::Amd64 => ExecutableArch::X86_64, Architecture::Arm64 => ExecutableArch::Aarch64 };
    if layout.format != Some(ExecutableFormat::Elf) || layout.class != Some(WordClass::SixtyFour)
        || layout.endianness != Some(Endianness::Little) || layout.arch != Some(architecture)
        || !matches!(layout.object,Some(ObjectKind::Executable | ObjectKind::SharedObject)) {
        return Err(reject(path,"ELF format/class/machine/byte order is incompatible with the image"));
    }
    if launch && !layout.has_entry_point { return Err(reject(path,"ELF object has no executable entry point")); }
    if let Some(Interpreter::Loader(loader)) = &layout.interpreter {
        let loader = text(loader,path)?;
        if !loader.starts_with('/') { return Err(reject(path,"ELF loader is not absolute")); }
        let (loader_path,entry) = resolve(namespace.files,loader).map_err(|error| OciError::Runtime {
            path:path.into(),detail:format!("ELF interpreter {loader:?}: {error}"),
        })?;
        let loader = namespace.layout(loader_path,entry)?;
        if loader.format != Some(ExecutableFormat::Elf) || loader.arch != Some(architecture)
            || loader.class != Some(WordClass::SixtyFour) || loader.endianness != Some(Endianness::Little)
            || !matches!(loader.object,Some(ObjectKind::Executable | ObjectKind::SharedObject))
            || !loader.has_entry_point || entry.mode & 0o111 == 0 {
            return Err(reject(path,"ELF interpreter is absent or incompatible"));
        }
    }
    Ok(())
}
