//! Managed blocks over the proved kernel (`gripsack_kernel::block`, plan/0056
//! K5): the kernel scans the hosting text, renders the block, splices it and
//! decides when gripsack may touch it; this module supplies what the kernel
//! cannot compute — the SHA-256 of the normalized content, the comment style
//! of a destination — and turns kernel offsets into `&str` views for reports.
use gripsack_host::store;
use gripsack_kernel::Digest;
use gripsack_kernel::block::{
    self, BlockDecision, BlockIntent, BlockRecord, Host, LineKind, Malformation, ScanFailure, Seen,
    SpliceRefusal, Style,
};
use std::ops::Range;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
#[error("malformed managed block at line {line}: {reason}")]
pub struct MergeParseError {
    pub line: usize,
    pub reason: &'static str,
}

impl From<ScanFailure> for MergeParseError {
    fn from(failure: ScanFailure) -> Self {
        Self {
            line: failure.line,
            reason: match failure.malformation {
                Malformation::Metadata => "invalid marker metadata or delimiter",
                Malformation::Nested => "nested or interleaved managed opener",
                Malformation::Orphan => "closing marker has no opener",
                Malformation::Interleaved => "closing marker names a different owner",
                Malformation::Unclosed => {
                    "opening marker has no closing marker; following text is unowned"
                }
            },
        }
    }
}

impl From<SpliceRefusal> for MergeParseError {
    fn from(refusal: SpliceRefusal) -> Self {
        match refusal {
            SpliceRefusal::Malformed(failure) => failure.into(),
            SpliceRefusal::Unordered => Self {
                line: 1,
                reason: "the scan returned blocks out of order",
            },
            SpliceRefusal::Unrecognized => Self {
                line: 1,
                reason: "the rendered block would not read back as written (an owner, comment \
                         prefix or payload line the marker grammar misreads)",
            },
        }
    }
}

/// Refuse a payload with a line the marker grammar reads as a marker.
pub(crate) fn validate_payload(payload: &str) -> Result<(), MergeParseError> {
    for (index, line) in payload.lines().enumerate() {
        match block::classify(line.as_bytes(), 0, line.len()) {
            LineKind::Content => {}
            LineKind::Malformed => {
                return Err(ScanFailure {
                    line: index + 1,
                    malformation: Malformation::Metadata,
                }
                .into());
            }
            LineKind::Open { .. } | LineKind::Close { .. } => {
                return Err(MergeParseError {
                    line: index + 1,
                    reason: "payload contains a managed marker",
                });
            }
        }
    }
    Ok(())
}

pub struct ManagedBlock<'a> {
    pub range: Range<usize>,
    pub content: &'a str,
    pub mode: Option<u32>,
    pub content_hash: store::hash::BytesHash,
    /// The marker's recorded sha digits name `content_hash`.
    pub marked: bool,
}

impl ManagedBlock<'_> {
    pub fn edited(&self) -> bool {
        !self.marked
    }

    fn seen(&self) -> Seen {
        Seen {
            content: digest(&self.content_hash),
            marked: self.marked,
            mode: self.mode,
        }
    }
}

pub struct ManagedBlockSet<'a> {
    text: &'a str,
    blocks: Vec<ManagedBlock<'a>>,
}

/// The SHA-256 a block records: of its content in the kernel's normal form.
pub fn content_hash(content: &str) -> store::hash::BytesHash {
    store::canonical_bytes_hash(&block::normalize(content.as_bytes()))
}

fn digest(hash: &store::hash::BytesHash) -> Digest {
    store::hash::kernel::digest_hex(hash.as_str()).expect("a bytes hash is sha256 hex")
}

/// The record a manifest entry keeps for its block.
pub fn record(entry: &store::DeployedEntry) -> BlockRecord {
    BlockRecord {
        hash: store::hash::kernel::digest_hex(entry.hash.as_str()),
        mode: entry.file_mode,
        preserved: entry.preserved_drift,
    }
}

/// The hosting object as the decision sees it.
pub fn host(observation: Option<&crate::deploy::Observation>) -> Host {
    match observation {
        None => Host::Absent,
        Some(crate::deploy::Observation::File { mode, .. }) => Host::File { mode: *mode },
        Some(crate::deploy::Observation::Symlink { .. }) => Host::Foreign,
    }
}

impl<'a> ManagedBlockSet<'a> {
    pub fn parse(text: &'a str, owner: &str) -> Result<Self, MergeParseError> {
        let found = block::scan(text.as_bytes(), owner.as_bytes())?;
        let mut blocks = Vec::with_capacity(found.len());
        for span in found {
            // kernel offsets sit at line boundaries or inside the ASCII sha
            // word, so every slice is on a char boundary
            let content = &text[span.content_start..span.content_end];
            let content_hash = content_hash(content);
            let mut sha = [0u8; block::MARK_DIGITS];
            sha.copy_from_slice(&text.as_bytes()[span.sha..span.sha + block::MARK_DIGITS]);
            let marked = block::marks(&sha, &digest(&content_hash));
            blocks.push(ManagedBlock {
                range: span.start..span.end,
                content,
                mode: span.mode,
                content_hash,
                marked,
            });
        }
        Ok(Self { text, blocks })
    }

    pub fn blocks(&self) -> &[ManagedBlock<'a>] {
        &self.blocks
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The proved decision (plan/0056 K1 for blocks) over these blocks.
    pub fn decide(
        &self,
        record: Option<BlockRecord>,
        host: &Host,
        intent: &BlockIntent,
    ) -> BlockDecision {
        let seen: Vec<Seen> = self.blocks.iter().map(ManagedBlock::seen).collect();
        block::decide_block(&record, host, &seen, intent)
    }

    /// The text with every block of `owner` removed, every other byte kept.
    pub fn remove(&self, owner: &str) -> Result<Option<String>, MergeParseError> {
        match block::remove(self.text.as_bytes(), owner.as_bytes())? {
            None => Ok(None),
            Some(bytes) => utf8(bytes).map(Some),
        }
    }

    pub fn upsert(
        &self,
        owner: &str,
        dest: &Path,
        marker: Option<&str>,
        payload: &str,
        mode: u32,
    ) -> Result<String, MergeParseError> {
        validate_payload(payload)?;
        let desired = digest(&content_hash(payload));
        let style = comment_style(dest, marker);
        let bytes = block::upsert(
            self.text.as_bytes(),
            owner.as_bytes(),
            &style,
            &desired,
            mode,
            payload.as_bytes(),
        )?;
        utf8(bytes)
    }

    pub fn report_note(&self) -> Option<String> {
        let mut notes = Vec::new();
        if self.blocks.iter().any(ManagedBlock::edited) {
            notes.push("hand-edited block regenerated".to_string());
        }
        let duplicates = self.blocks.len().saturating_sub(1);
        if duplicates != 0 {
            notes.push(format!(
                "removed {duplicates} duplicate block{}",
                if duplicates == 1 { "" } else { "s" }
            ));
        }
        (!notes.is_empty()).then(|| format!(" ({})", notes.join(", ")))
    }
}

/// The kernel splices UTF-8 text at line boundaries with UTF-8 blocks; the
/// conversion stays fallible as a boundary check.
fn utf8(bytes: Vec<u8>) -> Result<String, MergeParseError> {
    String::from_utf8(bytes).map_err(|_| MergeParseError {
        line: 1,
        reason: "splice output is not UTF-8",
    })
}

fn comment_style(dest: &Path, marker: Option<&str>) -> Style {
    let (prefix, html) = comment_prefix(dest, marker);
    Style {
        prefix: prefix.as_bytes().to_vec(),
        html,
    }
}

fn comment_prefix<'a>(dest: &Path, marker: Option<&'a str>) -> (&'a str, bool) {
    if let Some(marker) = marker {
        return (marker, false);
    }
    match dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
    {
        ".vimrc" | "vimrc" | "init.vim" => return ("\"", false),
        ".bashrc" | "bashrc" | ".zshrc" | "zshrc" | ".profile" | "profile" | ".bash_profile"
        | "bash_profile" | ".gitconfig" | "gitconfig" | "tmux.conf" | ".tmux.conf"
        | "ssh_config" => return ("#", false),
        _ => {}
    }
    match dest
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "js" | "ts" | "jsx" | "tsx" | "jsonc" | "css" | "scss" | "rs" | "go" | "c" | "h"
        | "cpp" | "hpp" | "java" | "kt" | "swift" => ("//", false),
        "lua" | "sql" => ("--", false),
        "vim" => ("\"", false),
        "html" | "xml" | "svg" | "md" => ("<!--", true),
        _ => ("#", false),
    }
}
