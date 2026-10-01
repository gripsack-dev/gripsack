use super::{BlobDigest, FileEntry, FileKind, OciError, OciLimits, archive::{self, Region}};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, fs::File, io::{BufReader, Read, Seek, SeekFrom, Write}, path::Path};

pub(super) struct Budget {
    expanded: u64,
    entries: usize,
    metadata: u64,
}
impl Budget {
    pub fn new(limits: OciLimits) -> Self { Self { expanded:limits.expanded_bytes, entries:limits.entries, metadata:limits.metadata_bytes } }
    fn entry(&mut self) -> Result<(), OciError> {
        self.entries = self.entries.checked_sub(1).ok_or(OciError::Invalid("OCI layer entry budget exceeded"))?;
        Ok(())
    }
    fn metadata(&mut self, count: usize) -> Result<(), OciError> {
        self.metadata = self.metadata.checked_sub(count as u64).ok_or(OciError::Invalid("OCI filesystem metadata budget exceeded"))?;
        Ok(())
    }
}

pub(super) fn apply(
    file: &mut File,
    region: Region,
    expected_diff: BlobDigest,
    budget: &mut Budget,
    files: &mut BTreeMap<String, FileEntry>,
    metadata: &mut super::runtime::MetadataIndex,
) -> Result<(), OciError> {
    file.seek(SeekFrom::Start(region.offset))?;
    let compressed = Read::by_ref(file).take(region.size);
    let mut decoder = flate2::bufread::GzDecoder::new(BufReader::new(compressed));
    let header = decoder.header().ok_or(OciError::Invalid("OCI layer is not a complete gzip stream"))?;
    if header.mtime() != 0 || header.filename().is_some() || header.comment().is_some() || header.extra().is_some() {
        return Err(OciError::Invalid("OCI gzip metadata differs from the fixed export profile"));
    }
    let mut spool = tempfile::tempfile()?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = decoder.read(&mut buffer)?;
        if count == 0 { break; }
        budget.expanded = budget.expanded.checked_sub(count as u64).ok_or(OciError::Invalid("expanded OCI layer budget exceeded"))?;
        hash.update(&buffer[..count]);
        spool.write_all(&buffer[..count])?;
    }
    let compressed = decoder.into_inner();
    if !compressed.buffer().is_empty() || compressed.get_ref().limit() != 0 {
        return Err(OciError::Invalid("trailing data or extra members in OCI gzip layer"));
    }
    if BlobDigest::from_bytes(hash.finalize().into()) != expected_diff {
        return Err(OciError::Invalid("OCI uncompressed layer differs from its DiffID"));
    }
    // Admit extension sizes before tar's ordinary iterator allocates PAX or GNU
    // records. Expanded payload size has already been bounded on the spool.
    spool.rewind()?;
    let mut last_end = 0;
    for entry in tar::Archive::new(&mut spool).entries_with_seek()?.raw(true) {
        budget.entry()?;
        let entry = entry?;
        last_end = entry.raw_file_position()
            .checked_add(entry.size().div_ceil(512).checked_mul(512).ok_or(OciError::Invalid("OCI tar size overflow"))?)
            .ok_or(OciError::Invalid("OCI tar range overflow"))?;
        let kind = entry.header().entry_type().as_byte();
        if matches!(kind, b'g' | b'S') { return Err(OciError::Invalid("global PAX and sparse OCI layers are unsupported")); }
        if matches!(kind, b'x' | b'L' | b'K') && entry.size() > 64 * 1024 {
            return Err(OciError::Invalid("OCI tar extension exceeds its metadata bound"));
        }
    }
    archive::end_marker(&mut spool, last_end)?;
    spool.rewind()?;
    let executable_source = spool.try_clone()?;
    let mut additions = BTreeMap::new();
    let mut whiteouts = BTreeSet::new();
    let mut opaque = BTreeSet::new();
    let mut links = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for entry in tar::Archive::new(&mut spool).entries_with_seek()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type().as_byte();
        if matches!(kind, b'x' | b'L' | b'K') { continue; }
        let path = member_path(&entry.path()?)?;
        budget.metadata(path.len())?;
        if !seen.insert(path.clone()) { return Err(OciError::Invalid("duplicate path within one OCI layer")); }
        if entry.header().mtime()? != 0 { return Err(OciError::Invalid("OCI layer timestamp differs from the fixed epoch")); }
        let raw_uid = entry.header().uid()?;
        let raw_gid = entry.header().gid()?;
        let raw_size = entry.size();
        if let Some(extensions) = entry.pax_extensions()? {
            for extension in extensions {
                let extension = extension?;
                budget.metadata(extension.key_bytes().len() + extension.value_bytes().len())?;
                match extension.key_bytes() {
                    b"path" | b"linkpath" => {}
                    b"uid" if numeric_metadata(extension.value_bytes()) == Some(raw_uid) => {}
                    b"gid" if numeric_metadata(extension.value_bytes()) == Some(raw_gid) => {}
                    b"size" if numeric_metadata(extension.value_bytes()) == Some(raw_size) => {}
                    b"mtime" | b"atime" | b"ctime" if zero_time(extension.value_bytes()) => {}
                    _ => return Err(OciError::Invalid("unsupported OCI layer PAX metadata")),
                }
            }
        }
        let name = path.rsplit('/').next().unwrap_or("");
        let parent = path.rsplit_once('/').map_or("/", |(parent, _)| if parent.is_empty() { "/" } else { parent });
        if name == ".wh..wh..opq" {
            if !matches!(kind, 0 | b'0') || entry.size() != 0 { return Err(OciError::Invalid("invalid OCI opaque whiteout")); }
            opaque.insert(parent.to_owned());
            continue;
        }
        if let Some(hidden) = name.strip_prefix(".wh.") {
            if hidden.is_empty() || matches!(hidden, "." | "..") || !matches!(kind, 0 | b'0') || entry.size() != 0 { return Err(OciError::Invalid("invalid OCI whiteout")); }
            whiteouts.insert(format!("{}/{hidden}", parent.trim_end_matches('/')));
            continue;
        }
        let mode = entry.header().mode()?;
        if mode & !0o7777 != 0 { return Err(OciError::Invalid("unsupported OCI permission bits")); }
        let uid = u32::try_from(raw_uid).map_err(|_| OciError::Invalid("OCI uid exceeds its numeric domain"))?;
        let gid = u32::try_from(raw_gid).map_err(|_| OciError::Invalid("OCI gid exceeds its numeric domain"))?;
        let content = match kind {
            0 | b'0' | b'7' => {
                let digest = archive::hash(&mut entry)?;
                let metadata_bytes = super::runtime::record(
                    metadata, &executable_source, entry.raw_file_position(), entry.size(), digest,
                )?;
                budget.metadata(metadata_bytes)?;
                FileKind::File { size:entry.size(), digest }
            },
            b'5' if entry.size() == 0 => FileKind::Directory,
            b'2' if entry.size() == 0 => {
                let target = entry.link_name()?.ok_or(OciError::Invalid("OCI symlink lacks a target"))?;
                let target = target.to_str().ok_or(OciError::Invalid("OCI symlink target is not UTF-8"))?;
                if target.is_empty() || target.len() > 4096 || target.contains('\0') { return Err(OciError::Invalid("invalid OCI symlink target")); }
                budget.metadata(target.len())?;
                FileKind::Symlink { target:target.to_owned() }
            }
            b'1' if entry.size() == 0 => {
                let target = member_path(&entry.link_name()?.ok_or(OciError::Invalid("OCI hardlink lacks a target"))?)?;
                budget.metadata(target.len())?;
                links.insert(path, target);
                continue;
            }
            _ => return Err(OciError::Invalid("unsupported special entry in OCI layer")),
        };
        additions.insert(path, FileEntry { kind:content, mode, uid, gid });
    }
    // Whiteouts affect the lower snapshot, never same-layer replacements.
    for path in opaque { remove_children(files, &path); }
    for path in whiteouts { files.remove(&path); remove_children(files, &path); }
    for (path, entry) in additions {
        if !matches!(entry.kind, FileKind::Directory) { remove_children(files, &path); }
        files.insert(path, entry);
    }
    let mut resolved: BTreeMap<&str, FileEntry> = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    let mut trail = Vec::new();
    for path in links.keys() {
        if resolved.contains_key(path.as_str()) { continue; }
        let mut current = path.as_str();
        while !resolved.contains_key(current) {
            let Some(target) = links.get(current) else { break; };
            if !visiting.insert(current) { return Err(OciError::Invalid("cyclic OCI hardlink target")); }
            trail.push(current);
            current = target;
        }
        let entry = resolved.get(current).or_else(|| files.get(current))
            .filter(|entry| matches!(entry.kind, FileKind::File { .. }))
            .ok_or(OciError::Invalid("missing or non-file OCI hardlink target"))?.clone();
        for path in trail.drain(..).rev() {
            visiting.remove(path);
            resolved.insert(path, entry.clone());
        }
    }
    for (path, entry) in resolved {
        budget.metadata(path.len())?;
        remove_children(files, path);
        files.insert(path.to_owned(), entry);
    }
    let mut implicit = BTreeSet::new();
    for path in files.keys() {
        let mut parent = path.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if prefix.is_empty() { break; }
            if files.get(prefix).is_some_and(|entry| !matches!(entry.kind, FileKind::Directory)) {
                return Err(OciError::Invalid("OCI layer addresses a symlink or file as a parent"));
            }
            if !files.contains_key(prefix) { implicit.insert(prefix.to_owned()); }
            parent = prefix;
        }
    }
    for path in implicit {
        budget.entry()?;
        budget.metadata(path.len())?;
        files.insert(path, FileEntry { kind:FileKind::Directory, mode:0o755, uid:0, gid:0 });
    }
    Ok(())
}

fn member_path(path: &Path) -> Result<String, OciError> {
    let text = path.to_str().ok_or(OciError::Invalid("OCI layer path is not UTF-8"))?;
    let text = text.strip_prefix("./").unwrap_or(text).trim_end_matches('/');
    if matches!(text, "" | ".") { return Ok("/".into()); }
    if text.len() > 4096 || text.starts_with('/') || text.contains('\0') || text.split('/').any(|part| matches!(part, "" | "." | "..")) {
        return Err(OciError::Invalid("OCI layer path escapes or aliases its image root"));
    }
    Ok(format!("/{text}"))
}
fn zero_time(value: &[u8]) -> bool {
    value == b"0" || value.strip_prefix(b"0.").is_some_and(|fraction| !fraction.is_empty() && fraction.iter().all(|byte| *byte == b'0'))
}
fn numeric_metadata(value: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(value).ok()?;
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
    text.parse().ok()
}
fn remove_children(files: &mut BTreeMap<String, FileEntry>, path: &str) {
    let prefix = format!("{}/", path.trim_end_matches('/'));
    let upper = format!("{}0", path.trim_end_matches('/'));
    files.extract_if(prefix..upper, |_, _| true).for_each(drop);
}
