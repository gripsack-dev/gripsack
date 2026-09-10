//! One lossless marker scan feeds reconciliation, drift checks, pruning and reports.
mod parse;
use gripsack_store as store;
pub use parse::MergeParseError;
use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

pub struct ManagedBlock<'a> {
    pub range: Range<usize>,
    pub content: &'a str,
    pub recorded_hash: &'a str,
    pub mode: Option<u32>,
    pub content_hash: store::hash::BytesHash,
}

impl ManagedBlock<'_> {
    pub fn edited(&self) -> bool {
        !self
            .recorded_hash
            .eq_ignore_ascii_case(&self.content_hash.as_str()[..16])
    }
}

pub struct ManagedBlockSet<'a> {
    text: &'a str,
    blocks: Vec<ManagedBlock<'a>>,
}

pub fn normalized_content(content: &str) -> Cow<'_, str> {
    let content = content.trim_end_matches(['\r', '\n']);
    if content.contains("\r\n") {
        Cow::Owned(content.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(content)
    }
}

pub fn content_hash(content: &str) -> store::hash::BytesHash {
    store::canonical_bytes_hash(normalized_content(content).as_bytes())
}

impl<'a> ManagedBlockSet<'a> {
    pub fn parse(text: &'a str, module: &str) -> Result<Self, MergeParseError> {
        let blocks = parse::scan(text, module)?
            .into_iter()
            .map(|block| ManagedBlock {
                content_hash: content_hash(block.content),
                range: block.range,
                content: block.content,
                recorded_hash: block.recorded_hash,
                mode: block.mode,
            })
            .collect();
        Ok(Self { text, blocks })
    }

    pub fn blocks(&self) -> &[ManagedBlock<'a>] {
        &self.blocks
    }
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn mode_conflicts(&self, live: u32, previous: Option<&store::DeployedEntry>) -> bool {
        self.blocks
            .iter()
            .any(|block| block.mode.is_some_and(|mode| mode != live))
            || previous
                .filter(|entry| !entry.preserved_drift)
                .and_then(|entry| entry.file_mode)
                .is_some_and(|mode| mode != live)
    }

    pub fn satisfied(&self, desired: &store::hash::BytesHash, mode: u32) -> bool {
        matches!(self.blocks.as_slice(), [block] if block.mode == Some(mode)
            && block.content_hash == *desired && !block.edited())
    }

    pub fn intact(&self, entry: &store::DeployedEntry, mode: u32) -> bool {
        !entry.preserved_drift
            && matches!(self.blocks.as_slice(), [block]
            if entry.file_mode.or(block.mode) == Some(mode)
                && block.mode.is_none_or(|recorded| recorded == mode)
                && block.content_hash.as_str() == entry.hash.as_str() && !block.edited())
    }

    /// No bytes outside complete owned spans are discarded or normalized.
    pub fn remove(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        Some(self.splice(""))
    }

    pub fn upsert(
        &self,
        module: &str,
        dest: &Path,
        marker: Option<&str>,
        payload: &str,
        mode: u32,
    ) -> Result<String, MergeParseError> {
        parse::validate_payload(payload)?;
        let (prefix, suffix) = comment_style(dest, marker);
        let newline = if self.text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let content = normalized_content(payload);
        let hash = content_hash(&content);
        let mut generated = format!(
            "{prefix} >>> gripsack module={module} sha={} mode=0{mode:o} >>>{suffix}{newline}{prefix} !! managed by gripsack — edit the module, not this block !!{suffix}{newline}",
            &hash.as_str()[..16]
        );
        for line in content.lines() {
            generated.push_str(line);
            generated.push_str(newline);
        }
        generated.push_str(&format!(
            "{prefix} <<< gripsack module={module} <<<{suffix}{newline}"
        ));
        if self.is_empty() {
            let mut output =
                String::with_capacity(self.text.len() + generated.len() + newline.len());
            output.push_str(self.text);
            if !output.is_empty() && !output.ends_with('\n') {
                output.push_str(newline)
            }
            output.push_str(&generated);
            Ok(output)
        } else {
            Ok(self.splice(&generated))
        }
    }

    fn splice(&self, replacement: &str) -> String {
        // the verified kernel (0047): byte-exact contract over spans —
        // ranges arrive line-aligned from the parser, so the output is
        // valid UTF-8; from_utf8 failing would be a parser bug, not a
        // user error
        let spans: Vec<gripsack_policy::merge::Span> = self
            .blocks
            .iter()
            .map(|block| gripsack_policy::merge::Span {
                start: block.range.start,
                end: block.range.end,
            })
            .collect();
        let bytes =
            gripsack_policy::merge::splice_bytes(self.text.as_bytes(), &spans, replacement.as_bytes());
        String::from_utf8(bytes).expect("block ranges are line-aligned and replacement is UTF-8")
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

fn comment_style<'a>(dest: &Path, marker: Option<&'a str>) -> (&'a str, &'static str) {
    if let Some(marker) = marker {
        return (marker, "");
    }
    match dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
    {
        ".vimrc" | "vimrc" | "init.vim" => return ("\"", ""),
        ".bashrc" | "bashrc" | ".zshrc" | "zshrc" | ".profile" | "profile" | ".bash_profile"
        | "bash_profile" | ".gitconfig" | "gitconfig" | "tmux.conf" | ".tmux.conf"
        | "ssh_config" => return ("#", ""),
        _ => {}
    }
    match dest
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "js" | "ts" | "jsx" | "tsx" | "jsonc" | "css" | "scss" | "rs" | "go" | "c" | "h"
        | "cpp" | "hpp" | "java" | "kt" | "swift" => ("//", ""),
        "lua" | "sql" => ("--", ""),
        "vim" => ("\"", ""),
        "html" | "xml" | "svg" | "md" => ("<!--", " -->"),
        _ => ("#", ""),
    }
}

#[cfg(test)]
mod tests;
