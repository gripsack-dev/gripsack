//! Runtime metadata is resolved only inside the verified image namespace. Host
//! loaders, libraries, PATH, working directories and build prefixes are not input.
mod elf;
use super::{BlobDigest, FileEntry, FileKind, OciError, file_region::FileRegion, resolve};
use crate::plan::{ImageConfig, Platform};
use gripsack_process::executable::{
    self, ExecutableFormat, ExecutableMetadata, Interpreter, LayoutError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs::File,
};

pub(super) type MetadataIndex = BTreeMap<BlobDigest, Result<Box<ExecutableMetadata>, LayoutError>>;

pub(super) fn record(
    index: &mut MetadataIndex,
    file: &File,
    offset: u64,
    size: u64,
    digest: BlobDigest,
) -> Result<usize, OciError> {
    if index.contains_key(&digest) {
        return Ok(0);
    }
    let mut source = FileRegion::new(file, offset, size)?;
    let metadata = executable::classify(&mut source).map(Box::new);
    if let Err(LayoutError::Io(kind)) = &metadata {
        return Err(OciError::Io(std::io::Error::from(*kind)));
    }
    // A source template or runtime data file can start with an executable
    // signature. Preserve a parse error until that file is actually selected.
    let mut bytes =
        std::mem::size_of::<(BlobDigest, Result<Box<ExecutableMetadata>, LayoutError>)>();
    if let Ok(metadata) = &metadata {
        bytes += std::mem::size_of::<ExecutableMetadata>();
        for value in metadata
            .needed_libraries
            .iter()
            .chain(&metadata.rpaths)
            .chain(&metadata.runpaths)
        {
            bytes += value.as_encoded_bytes().len();
        }
        if let Some(interpreter) = &metadata.interpreter {
            match interpreter {
                Interpreter::Loader(path) => bytes += path.as_encoded_bytes().len(),
                Interpreter::Shebang { program, argument } => {
                    bytes += program.as_encoded_bytes().len();
                    bytes += argument
                        .as_ref()
                        .map_or(0, |value| value.as_encoded_bytes().len());
                }
            }
        }
    }
    index.insert(digest, metadata);
    Ok(bytes)
}

pub(super) fn validate(
    files: &BTreeMap<String, FileEntry>,
    metadata: &MetadataIndex,
    platform: Platform,
    config: &ImageConfig,
) -> Result<(), OciError> {
    let namespace = Namespace {
        files,
        metadata,
        platform,
        config,
    };
    let mut commands = Vec::new();
    if let Some(program) = config.entrypoint.first().or_else(|| config.args.first()) {
        commands.push(program.as_str());
    }
    elf::validate(&namespace)?;
    let mut visited = BTreeSet::new();
    while let Some(program) = commands.pop() {
        let (path, entry) = resolve(files, program)?;
        if !visited.insert(path) {
            return Err(reject(path, "recursive interpreter chain"));
        }
        if visited.len() > 5 {
            return Err(reject(
                path,
                "interpreter chain exceeds the Linux recursion bound",
            ));
        }
        if entry.mode & 0o111 == 0 {
            return Err(reject(path, "runtime command is not executable"));
        }
        let layout = namespace.layout(path, entry)?;
        match layout.format {
            Some(ExecutableFormat::Elf) => elf::entry_header(&namespace, path, layout, true)?,
            Some(ExecutableFormat::Script) => {
                let Some(Interpreter::Shebang { program, argument }) = &layout.interpreter else {
                    return Err(reject(path, "script has no interpreter"));
                };
                let interpreter = text(program, path)?;
                if !interpreter.starts_with('/') {
                    return Err(reject(path, "script interpreter is not absolute"));
                }
                let (interpreter_path, _) = resolve(files, interpreter)?;
                commands.push(interpreter_path);
                if interpreter == "/usr/bin/env" || interpreter == "/bin/env" {
                    let argument = argument
                        .as_ref()
                        .ok_or_else(|| reject(path, "env shebang lacks an explicit program"))?;
                    let argument = text(argument, path)?;
                    if argument.starts_with('-') || argument.contains(char::is_whitespace) {
                        return Err(reject(
                            path,
                            "env shebang flags or split words are unsupported; name a pinned interpreter directly",
                        ));
                    }
                    let program = namespace.find_program(argument, path)?;
                    commands.push(program);
                }
            }
            _ => return Err(reject(path, "unsupported image executable format")),
        }
    }
    Ok(())
}

struct Namespace<'a> {
    files: &'a BTreeMap<String, FileEntry>,
    metadata: &'a MetadataIndex,
    platform: Platform,
    config: &'a ImageConfig,
}
impl<'a> Namespace<'a> {
    fn layout(&self, path: &str, entry: &FileEntry) -> Result<&'a ExecutableMetadata, OciError> {
        let FileKind::File { digest, .. } = entry.kind else {
            return Err(reject(path, "runtime target is not a regular file"));
        };
        match self.metadata.get(&digest) {
            Some(Ok(layout)) => Ok(layout),
            Some(Err(error)) => Err(OciError::Runtime {
                path: path.into(),
                detail: format!("runtime target has invalid executable metadata: {error}"),
            }),
            None => Err(reject(
                path,
                "runtime target has no admitted executable metadata",
            )),
        }
    }
    fn search_paths(
        &self,
        program: &str,
        paths: &[std::ffi::OsString],
        output: &mut Vec<String>,
    ) -> Result<(), OciError> {
        let parent = program.rsplit_once('/').map_or("/", |(parent, _)| parent);
        for path in paths {
            for path in text(path, program)?.split(':') {
                let expanded = if path == "$ORIGIN" || path == "${ORIGIN}" {
                    parent.to_owned()
                } else if let Some(relative) = path
                    .strip_prefix("$ORIGIN/")
                    .or_else(|| path.strip_prefix("${ORIGIN}/"))
                {
                    normalize(&format!("{parent}/{relative}"))?
                } else if path.starts_with('/') && !path.contains('$') {
                    path.to_owned()
                } else {
                    return Err(reject(
                        program,
                        "unsupported or host-relative ELF search path",
                    ));
                };
                if let Some(directory) = self.directory(&expanded, program)? {
                    output.push(directory);
                }
            }
        }
        Ok(())
    }
    fn directory(&self, directory: &str, requester: &str) -> Result<Option<String>, OciError> {
        if !directory.starts_with('/') {
            return Err(reject(
                requester,
                "runtime search directory must be absolute and nonempty",
            ));
        }
        let normalized = normalize(directory)?;
        let (path, entry) = match resolve(self.files, &normalized) {
            Ok(entry) => entry,
            Err(OciError::MissingPath(_)) => return Ok(None),
            Err(error) => {
                return Err(OciError::Runtime {
                    path: requester.into(),
                    detail: format!("runtime search directory {normalized:?}: {error}"),
                });
            }
        };
        if !matches!(entry.kind, FileKind::Directory) {
            return Err(reject(
                requester,
                "runtime search path is not an image directory",
            ));
        }
        Ok(Some(path.to_owned()))
    }
    fn environment(&self, name: &str) -> Option<&str> {
        self.config.env.iter().find_map(|entry| {
            entry
                .split_once('=')
                .filter(|(key, _)| *key == name)
                .map(|(_, value)| value)
        })
    }
    fn find_program(&self, program: &str, requester: &str) -> Result<&'a str, OciError> {
        if program.contains('/') {
            return Err(reject(requester, "env interpreter must be one PATH name"));
        }
        let search = self
            .environment("PATH")
            .ok_or_else(|| reject(requester, "env shebang requires an explicit image PATH"))?;
        for directory in search.split(':') {
            let Some(directory) = self.directory(directory, requester)? else {
                continue;
            };
            if let Ok((path, entry)) = resolve(self.files, &format!("{directory}/{program}"))
                && matches!(entry.kind, FileKind::File { .. })
                && entry.mode & 0o111 != 0
            {
                return Ok(path);
            }
        }
        Err(reject(
            requester,
            "env interpreter is absent from image PATH",
        ))
    }
}
fn normalize(path: &str) -> Result<String, OciError> {
    let mut pieces = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if pieces.pop().is_none() {
                    return Err(reject(path, "runtime path escapes the image root"));
                }
            }
            value => pieces.push(value),
        }
    }
    Ok(format!("/{}", pieces.join("/")))
}
fn text<'a>(value: &'a OsStr, requester: &str) -> Result<&'a str, OciError> {
    value
        .to_str()
        .ok_or_else(|| reject(requester, "runtime metadata is not UTF-8"))
}
fn reject(path: &str, detail: &'static str) -> OciError {
    OciError::Runtime {
        path: path.into(),
        detail: detail.into(),
    }
}
