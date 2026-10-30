//! Independent admission of original wheel bytes; extraction is never an installer.
//! Limits apply before allocation and the private extraction is retained by its owner.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use gripsack_ir::workspace_model::{WheelSource, lock::LockedWheel};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use uv_distribution_filename::WheelFilename;
use uv_pep440::Version;
use uv_pypi_types::Metadata23;
mod inventory;
pub use inventory::ExpectedWheelFile;

mod compatibility;
pub use compatibility::{PythonTarget, validate_lock};
mod dependencies;
pub use dependencies::{DistributionMetadata, validate_dependencies};
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 1024 * 1024 * 1024;
const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

#[derive(Debug, thiserror::Error)]
#[error("wheel admission: {0}")]
pub struct WheelError(String);
fn invalid(message: impl ToString) -> WheelError {
    WheelError(message.to_string())
}
impl From<std::io::Error> for WheelError {
    fn from(e: std::io::Error) -> Self {
        invalid(e)
    }
}

#[derive(Debug)]
struct Member {
    bytes: Arc<Vec<u8>>,
    mode: u32,
}

/// A fully checked immutable inventory and a private, safely extracted copy.
#[derive(Debug)]
pub struct WheelArchive {
    extracted: tempfile::TempDir,
    files: BTreeMap<String, Member>,
    dist: String,
    expanded: u64,
    archive_size: u64,
    entries: usize,
    metadata: Metadata23,
}
impl WheelArchive {
    pub fn admit(path: &Path, record: &LockedWheel) -> Result<Self, WheelError> {
        Self::admit_bounded(path, record, MAX_EXPANDED, MAX_ENTRIES)
    }

    /// Bound decompression and entry admission by the caller's remaining closure.
    /// The wheel's independent format ceilings remain in force.
    pub fn admit_bounded(
        path: &Path,
        record: &LockedWheel,
        bytes: u64,
        entries: usize,
    ) -> Result<Self, WheelError> {
        record.validate().map_err(invalid)?;
        let archive = Self::load(
            path,
            &record.filename,
            &record.sha256,
            record.size,
            bytes,
            entries,
        )?;
        archive.check_identity(&record.name, &record.version)?;
        if archive.metadata.requires_python != record.requires_python
            || archive.metadata.requires_dist != record.requires_dist
        {
            return Err(invalid("locked dependency metadata differs from METADATA"));
        }
        Ok(archive)
    }
    pub fn extracted_dir(&self) -> &Path {
        self.extracted.path()
    }
    pub fn expanded_bytes(&self) -> u64 {
        self.expanded
    }
    pub fn entry_count(&self) -> usize {
        self.entries
    }
    fn check_identity(&self, name: &str, version: &str) -> Result<(), WheelError> {
        if normalize(name) != normalize(&self.metadata.name)
            || Version::from_str(version).map_err(invalid)?
                != Version::from_str(&self.metadata.version).map_err(invalid)?
        {
            return Err(invalid("wheel name/version differs from locked identity"));
        }
        Ok(())
    }
    fn load(
        path: &Path,
        filename: &str,
        sha256: &str,
        size: Option<u64>,
        byte_limit: u64,
        entry_limit: usize,
    ) -> Result<Self, WheelError> {
        safe_path(filename)?;
        if filename.contains('/') {
            return Err(invalid("wheel filename contains a directory"));
        }
        let filename = WheelFilename::from_str(filename).map_err(invalid)?;
        let mut input = fs::File::open(path)?;
        let length = input.metadata()?.len();
        if length > MAX_ARCHIVE || size.is_some_and(|size| size != length) {
            return Err(invalid("archive size mismatch or bound exceeded"));
        }
        // Spool the exact bytes which are hashed; later source-path mutation cannot change admission.
        let mut spool = tempfile::tempfile()?;
        let mut digest = Sha256::new();
        let mut count = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > MAX_ARCHIVE {
                return Err(invalid("archive grew beyond bound"));
            }
            digest.update(&buffer[..n]);
            spool.write_all(&buffer[..n])?;
        }
        if count != length || hex::encode(digest.finalize()) != sha256 {
            return Err(invalid("original archive SHA256 mismatch"));
        }
        let mut zip = zip::ZipArchive::new(spool).map_err(invalid)?;
        if zip.len() > MAX_ENTRIES.min(entry_limit) {
            return Err(invalid("too many ZIP entries"));
        }
        let mut files = BTreeMap::new();
        let mut names = BTreeSet::new();
        let mut kinds = BTreeMap::new();
        let mut expanded = 0u64;
        let mut ranges = BTreeMap::<u64, u64>::new();
        for index in 0..zip.len() {
            let entry = zip.by_index(index).map_err(invalid)?;
            let start = entry.header_start();
            let end = entry
                .data_start()
                .checked_add(entry.compressed_size())
                .ok_or_else(|| invalid("ZIP offset overflow"))?;
            if end > length
                || ranges
                    .range(..=start)
                    .next_back()
                    .is_some_and(|(_, previous)| *previous > start)
                || ranges
                    .range(start..)
                    .next()
                    .is_some_and(|(next, _)| *next < end)
            {
                return Err(invalid("overlapping ZIP members"));
            }
            ranges.insert(start, end);
            let directory = entry.is_dir();
            let name = entry
                .name()
                .strip_suffix('/')
                .unwrap_or(entry.name())
                .to_owned();
            safe_path(&name)?;
            if entry.name_raw() != entry.name().as_bytes() {
                return Err(invalid("non-UTF8 ZIP name"));
            }
            if !names.insert(name.to_ascii_lowercase()) {
                return Err(invalid("duplicate/case-colliding ZIP entry"));
            }
            let mode = entry
                .unix_mode()
                .unwrap_or(if directory { 0o40755 } else { 0o100644 });
            let kind = mode & 0o170000;
            if (kind != 0 && kind != if directory { 0o40000 } else { 0o100000 })
                || mode & 0o7022 != 0
            {
                return Err(invalid("unsafe ZIP type or permissions"));
            }
            kinds.insert(name.clone(), directory);
            if directory {
                if entry.size() != 0 {
                    return Err(invalid("nonempty directory entry"));
                }
                continue;
            }
            reject_startup(&name)?;
            if entry.size() > MAX_FILE {
                return Err(invalid("ZIP member exceeds bound"));
            }
            expanded = expanded
                .checked_add(entry.size())
                .ok_or_else(|| invalid("expanded size overflow"))?;
            if expanded > MAX_EXPANDED.min(byte_limit) {
                return Err(invalid("expanded wheel exceeds bound"));
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            let expected = entry.size();
            entry.take(expected + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 != expected {
                return Err(invalid("ZIP member size mismatch"));
            }
            if name.to_ascii_lowercase().ends_with(".pth") {
                validate_pth(&bytes)?;
            }
            files.insert(
                name,
                Member {
                    bytes: Arc::new(bytes),
                    mode: if mode & 0o111 != 0 { 0o755 } else { 0o644 },
                },
            );
        }
        let mut components = BTreeMap::<String, String>::new();
        for name in kinds.keys() {
            let parts: Vec<_> = name.split('/').collect();
            for end in 1..=parts.len() {
                let prefix = parts[..end].join("/");
                if end < parts.len() && kinds.get(&prefix) == Some(&false) {
                    return Err(invalid("ZIP file/directory collision"));
                }
                if let Some(previous) =
                    components.insert(prefix.to_ascii_lowercase(), prefix.clone())
                    && previous != prefix
                {
                    return Err(invalid("case-colliding path component"));
                }
            }
        }
        let dist_dirs: BTreeSet<_> = files
            .keys()
            .filter_map(|p| p.split('/').next())
            .filter(|p| p.ends_with(".dist-info"))
            .collect();
        if dist_dirs.len() != 1 {
            return Err(invalid(
                "wheel must contain exactly one dist-info directory",
            ));
        }
        let dist = dist_dirs.into_iter().next().unwrap().to_owned();
        let expected_dist = format!(
            "{}-{}.dist-info",
            filename.name.as_dist_info_name(),
            filename.version
        );
        if dist != expected_dist {
            return Err(invalid("dist-info directory differs from filename"));
        }
        for leaf in ["METADATA", "WHEEL", "entry_points.txt"] {
            if files
                .get(&format!("{dist}/{leaf}"))
                .is_some_and(|m| m.bytes.len() > 4 * 1024 * 1024)
            {
                return Err(invalid("metadata exceeds bound"));
            }
        }
        let metadata_text =
            std::str::from_utf8(&required(&files, &format!("{dist}/METADATA"))?.bytes)
                .map_err(invalid)?;
        let metadata_headers = mail_headers(metadata_text)?;
        for key in ["Name", "Version", "Metadata-Version"] {
            if metadata_headers
                .get(key)
                .is_none_or(|values| values.len() != 1)
            {
                return Err(invalid("missing or duplicate identity metadata"));
            }
        }
        let mut metadata = Metadata23::parse(&required(&files, &format!("{dist}/METADATA"))?.bytes)
            .map_err(invalid)?;
        metadata.requires_dist.sort();
        metadata.requires_dist.dedup();
        if normalize(&metadata.name) != filename.name.to_string()
            || Version::from_str(&metadata.version).map_err(invalid)? != filename.version
        {
            return Err(invalid("filename differs from METADATA"));
        }
        let wheel = std::str::from_utf8(&required(&files, &format!("{dist}/WHEEL"))?.bytes)
            .map_err(invalid)?;
        let wheel_headers = mail_headers(wheel)?;
        if wheel_headers.get("Wheel-Version").map(Vec::as_slice) != Some(&["1.0".to_string()][..])
            || !matches!(wheel_headers.get("Root-Is-Purelib").map(Vec::as_slice), Some([v]) if v == "true" || v == "false")
        {
            return Err(invalid("unsupported WHEEL headers"));
        }
        validate_tags(&filename, &wheel_headers)?;
        let record_path = format!("{dist}/RECORD");
        validate_record(&files, &record_path)?;
        let data = dist.replace(".dist-info", ".data");
        for name in files.keys() {
            if let Some(root) = name.split('/').next().filter(|p| p.ends_with(".data"))
                && root != data
            {
                return Err(invalid("foreign wheel data directory"));
            }
        }
        let extracted = tempfile::tempdir()?;
        for (name, member) in &files {
            if suppressed(name) {
                continue;
            }
            let destination = extracted.path().join(name);
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::write(&destination, member.bytes.as_slice())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(destination, fs::Permissions::from_mode(member.mode))?;
            }
        }
        let mut filtered_record = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(Vec::new());
        for (name, member) in &files {
            if suppressed(name) {
                continue;
            }
            let (digest, size) = if name == &record_path {
                (String::new(), String::new())
            } else {
                (hash(&member.bytes), member.bytes.len().to_string())
            };
            filtered_record
                .serialize((name, digest, size))
                .map_err(invalid)?;
        }
        fs::write(
            extracted.path().join(&record_path),
            filtered_record.into_inner().map_err(invalid)?,
        )?;
        Ok(Self {
            extracted,
            files,
            dist,
            expanded,
            archive_size: length,
            entries: zip.len(),
            metadata,
        })
    }
}

/// Freeze native authoring metadata without solving, executing or installing Python code.
pub fn inspect(path: &Path, source: &WheelSource) -> Result<LockedWheel, WheelError> {
    source.validate().map_err(invalid)?;
    let url = url::Url::parse(&source.url).map_err(invalid)?;
    let filename = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .ok_or_else(|| invalid("wheel URL has no filename"))?
        .to_owned();
    let archive = WheelArchive::load(
        path,
        &filename,
        &source.sha256,
        None,
        MAX_EXPANDED,
        MAX_ENTRIES,
    )?;
    archive.check_identity(&source.name, &source.version)?;
    let locked = LockedWheel {
        name: source.name.clone(),
        version: source.version.clone(),
        filename,
        url: source.url.clone(),
        sha256: source.sha256.clone(),
        size: Some(archive.archive_size),
        requires_python: archive.metadata.requires_python.clone(),
        requires_dist: archive.metadata.requires_dist.clone(),
    };
    locked.validate().map_err(invalid)?;
    Ok(locked)
}
fn normalize(name: &str) -> String {
    name.to_ascii_lowercase()
        .split(['-', '_', '.'])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
fn safe_path(path: &str) -> Result<(), WheelError> {
    if path.len() > 4096
        || path.is_empty()
        || !path.is_ascii()
        || path
            .bytes()
            .any(|c| c < 32 || c == 127 || matches!(c, b'\\' | b':'))
        || path
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == ".." || c.ends_with([' ', '.']))
    {
        return Err(invalid("unsafe wheel path"));
    }
    Ok(())
}
fn reject_startup(path: &str) -> Result<(), WheelError> {
    let lower = path.to_ascii_lowercase();
    let basename = lower.rsplit('/').next().unwrap();
    if basename.starts_with("sitecustomize.")
        || basename.starts_with("usercustomize.")
        || lower
            .split('/')
            .any(|p| p == "sitecustomize" || p == "usercustomize")
        || lower.contains(".data/scripts/")
    {
        return Err(invalid(
            "publisher startup hooks or arbitrary scripts are forbidden",
        ));
    }
    Ok(())
}
fn validate_pth(bytes: &[u8]) -> Result<(), WheelError> {
    let text = std::str::from_utf8(bytes).map_err(invalid)?;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("import ") || line.starts_with("import\t") {
            return Err(invalid("executable publisher .pth startup hook"));
        }
        if line != "." {
            safe_path(line)?;
        }
    }
    Ok(())
}
fn required<'a>(files: &'a BTreeMap<String, Member>, name: &str) -> Result<&'a Member, WheelError> {
    files
        .get(name)
        .ok_or_else(|| invalid(format!("missing {name}")))
}
fn hash(bytes: &[u8]) -> String {
    format!("sha256={}", URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)))
}
fn validate_record(files: &BTreeMap<String, Member>, record: &str) -> Result<(), WheelError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(required(files, record)?.bytes.as_slice());
    let mut seen = BTreeSet::new();
    for row in reader.records() {
        let row = row.map_err(invalid)?;
        if row.len() != 3 {
            return Err(invalid("RECORD must have three columns"));
        }
        let name = &row[0];
        safe_path(name)?;
        if !seen.insert(name.to_owned()) {
            return Err(invalid("duplicate RECORD member"));
        }
        let member = required(files, name)?;
        if name == record {
            if !row[1].is_empty() || !row[2].is_empty() {
                return Err(invalid("RECORD self-entry must be unhashed"));
            }
        } else if row[1] != hash(&member.bytes) || row[2] != member.bytes.len().to_string() {
            return Err(invalid("RECORD hash/size mismatch"));
        }
    }
    if seen.len() != files.len() {
        return Err(invalid("RECORD inventory is incomplete"));
    }
    Ok(())
}
fn mail_headers(text: &str) -> Result<BTreeMap<String, Vec<String>>, WheelError> {
    // WHEEL uses simple RFC822 headers; the maintained parser handles folding and syntax.
    let (headers, _) = mailparse::parse_headers(text.as_bytes()).map_err(invalid)?;
    let mut result = BTreeMap::<String, Vec<String>>::new();
    for header in headers {
        result
            .entry(header.get_key())
            .or_default()
            .push(header.get_value());
    }
    Ok(result)
}
fn suppressed(name: &str) -> bool {
    name.ends_with(".pyc")
        || name.ends_with(".pyo")
        || name.split('/').any(|part| part == "__pycache__")
}
fn validate_tags(
    filename: &WheelFilename,
    headers: &BTreeMap<String, Vec<String>>,
) -> Result<(), WheelError> {
    let rendered = filename.to_string();
    let parts: Vec<_> = rendered.trim_end_matches(".whl").rsplitn(4, '-').collect();
    if parts.len() != 4 {
        return Err(invalid("invalid wheel tag fields"));
    }
    let mut expected = BTreeSet::new();
    for python in parts[2].split('.') {
        for abi in parts[1].split('.') {
            for platform in parts[0].split('.') {
                expected.insert(format!("{python}-{abi}-{platform}"));
            }
        }
    }
    let actual: BTreeSet<_> = headers.get("Tag").into_iter().flatten().cloned().collect();
    if actual != expected {
        return Err(invalid("WHEEL tags differ from filename"));
    }
    Ok(())
}
#[cfg(test)]
mod tests;
