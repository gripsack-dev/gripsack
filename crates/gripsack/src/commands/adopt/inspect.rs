//! Inventory: what adopting a path would take (0015 §7 S2/S8).
//!
//! The walk never follows directory symlinks — a link inside the
//! adopted tree must not pull an arbitrary tree into the user's repo.
//! Directory links and broken file links are reported, not silently
//! absorbed.

use std::path::Path;

/// One adoptable file: path relative to the adopt root, size in bytes.
pub struct InventoriedFile {
    pub rel: String,
    pub size: u64,
}

/// A skipped entry: relative path and the reason it was skipped.
pub struct Skipped {
    pub rel: String,
    pub reason: String,
}

pub struct Inventory {
    pub files: Vec<InventoriedFile>,
    pub skipped: Vec<Skipped>,
    pub total_bytes: u64,
}

/// Above this, adopting is probably pulling caches into the repo —
/// size is evidence, not a heuristic: warn and name the largest.
pub const SIZE_WARN_BYTES: u64 = 25 * 1024 * 1024;

pub fn inspect(dest: &Path, is_dir: bool) -> Inventory {
    let mut inv = Inventory {
        files: Vec::new(),
        skipped: Vec::new(),
        total_bytes: 0,
    };
    if !is_dir {
        let rel = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::metadata(dest) {
            Ok(meta) => {
                inv.total_bytes += meta.len();
                inv.files.push(InventoriedFile {
                    rel,
                    size: meta.len(),
                });
            }
            Err(_) => inv.skipped.push(Skipped {
                rel,
                reason: "broken symlink".into(),
            }),
        }
        return inv;
    }
    walk(dest, dest, &mut inv);
    inv.files.sort_by_key(|f| f.rel.clone());
    inv
}

fn walk(root: &Path, dir: &Path, inv: &mut Inventory) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .expect("walked path is under root")
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&path)
                .map(|t| t.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "?".into());
            if std::fs::metadata(&path)
                .map(|m| m.is_dir())
                .unwrap_or(false)
            {
                inv.skipped.push(Skipped {
                    rel,
                    reason: format!("directory symlink → {target} (not followed)"),
                });
            } else if std::fs::metadata(&path).is_err() {
                inv.skipped.push(Skipped {
                    rel,
                    reason: format!("broken symlink → {target}"),
                });
            } else {
                // a live file symlink: the CONTENT is adopted
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                inv.total_bytes += size;
                inv.files.push(InventoriedFile { rel, size });
            }
        } else if meta.is_dir() {
            walk(root, &path, inv);
        } else if meta.is_file() {
            inv.total_bytes += meta.len();
            inv.files.push(InventoriedFile {
                rel,
                size: meta.len(),
            });
        }
    }
}

/// The N largest entries, for the size warning.
pub fn largest(inv: &Inventory, n: usize) -> Vec<(&str, u64)> {
    let mut files: Vec<_> = inv.files.iter().collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.size));
    files
        .into_iter()
        .take(n)
        .map(|f| (f.rel.as_str(), f.size))
        .collect()
}

pub fn fmt_kib(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} kB", bytes as f64 / 1024.0)
    }
}
