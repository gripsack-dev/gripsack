//! Inventory: what adopting a path would take (0015 §7 S2/S8).
//!
//! The walk never follows directory symlinks — a link inside the
//! adopted tree must not pull an arbitrary tree into the user's repo.
//! Directory links and broken file links are reported, not silently
//! absorbed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Keep the native path returned by the directory walk for the actual copy.
pub struct InventoriedFile {
    pub source: PathBuf,
    pub size: u64,
}

/// A skipped source and the reason it was not adopted.
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

pub struct Inventory<'a> {
    base: &'a Path,
    pub files: Vec<InventoriedFile>,
    pub skipped: Vec<Skipped>,
    pub total_bytes: u64,
}

/// Above this, adopting is probably pulling caches into the repo —
/// size is evidence, not a heuristic: warn and name the largest.
pub const SIZE_WARN_BYTES: u64 = 25 * 1024 * 1024;

pub fn inspect(dest: &Path, metadata: fs::Metadata) -> Result<Inventory<'_>, String> {
    let mut inv = Inventory {
        base: if metadata.is_dir() {
            dest
        } else {
            dest.parent().unwrap_or(dest)
        },
        files: Vec::new(),
        skipped: Vec::new(),
        total_bytes: 0,
    };
    inv.visit(dest.to_owned(), metadata)?;
    inv.files
        .sort_unstable_by(|a, b| a.source.as_os_str().cmp(b.source.as_os_str()));
    Ok(inv)
}

impl Inventory<'_> {
    pub fn relative<'p>(&self, path: &'p Path) -> &'p Path {
        path.strip_prefix(self.base)
            .expect("inventoried path is under its root")
    }

    /// For emitted IR and reports only. Never reopen a source from this spelling.
    pub fn selector<'f>(&self, file: &'f InventoriedFile) -> &'f str {
        self.relative(&file.source)
            .to_str()
            .expect("UTF-8 checked at inventory")
    }

    fn visit(&mut self, path: PathBuf, metadata: fs::Metadata) -> Result<(), String> {
        if metadata.is_dir() {
            let entries =
                fs::read_dir(&path).map_err(|e| format!("cannot read directory {path:?}: {e}"))?;
            for entry in entries {
                let child = entry
                    .map_err(|e| format!("cannot read entry in {path:?}: {e}"))?
                    .path();
                let metadata = child
                    .symlink_metadata()
                    .map_err(|e| format!("cannot inspect {child:?}: {e}"))?;
                self.visit(child, metadata)?;
            }
            return Ok(());
        }

        let reason = if metadata.is_symlink() {
            // Read a file link's contents, never descend through a directory link.
            let kind = match fs::metadata(&path) {
                Ok(m) if m.is_file() => return self.add_file(path, m.len()),
                Ok(m) if m.is_dir() => "directory symlink (not followed)",
                Ok(_) => "symlink to a non-regular file",
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) =>
                {
                    "broken symlink"
                }
                Err(e) => return Err(format!("cannot inspect target of {path:?}: {e}")),
            };
            let target =
                fs::read_link(&path).map_err(|e| format!("cannot read link {path:?}: {e}"))?;
            format!("{kind} → {target:?}")
        } else if metadata.is_file() {
            return self.add_file(path, metadata.len());
        } else {
            "not a regular file (fifo, device, or socket)".into()
        };
        self.skipped.push(Skipped { path, reason });
        Ok(())
    }

    fn add_file(&mut self, source: PathBuf, size: u64) -> Result<(), String> {
        let rel = self.relative(&source);
        let spelling = rel
            .to_str()
            .ok_or_else(|| format!("{rel:?} is not a UTF-8 workspace path"))?;
        // Linux treats backslash as a filename byte; IR10 repoFile forbids it.
        if spelling.contains('\\') {
            return Err(format!(
                "literal backslash in {rel:?} cannot be represented by repoFile"
            ));
        }
        self.total_bytes += size;
        self.files.push(InventoriedFile { source, size });
        Ok(())
    }
}

/// The N largest entries, for the size warning.
pub fn largest<'i>(inv: &'i Inventory<'_>, n: usize) -> Vec<(&'i str, u64)> {
    let mut files: Vec<_> = inv.files.iter().collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.size));
    files
        .into_iter()
        .take(n)
        .map(|f| (inv.selector(f), f.size))
        .collect()
}

pub fn fmt_kib(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} kB", bytes as f64 / 1024.0)
    }
}
