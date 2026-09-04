//! `grip store verify [--repair]` (0008 §3, corrected by 0014 §1a):
//! re-hash every store path and report corruption. Per-entry manifest
//! hashes cover every module kind; content-addressed modules also
//! carry `tree256` in the manifest, so whole-tree verification needs
//! no lockfile lookup (and never depends on the ambient hostname).

use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;

/// Every store path, re-hashed against the lockfile's recorded pins.
/// Corrupt paths are reported; with `repair` they're removed (the next
/// apply re-fetches — publish_dir's refusal becomes a republish).
pub fn verify_store(
    ctx: &Ctx,
    repair: bool,
) -> Result<Vec<(String, ReportKind, String)>, ExecError> {
    let mut out = Vec::new();
    let home = &ctx.home;
    // enumeration errors are real (0027 §2) — verify must not read
    // "cannot list generations" as "nothing to verify"
    for n in gripsack_store::list_generations(home)? {
        let manifest = match gripsack_store::read_manifest(home, n) {
            Ok(m) => m,
            Err(e) => {
                // a corrupt generation must surface, not read as "ok" —
                // gc fails closed; verify at least says what it skipped
                out.push((
                    "*".into(),
                    ReportKind::Warned,
                    format!("generation {n} manifest unreadable: {e}"),
                ));
                continue;
            }
        };
        for (name, state) in &manifest.modules {
            let path = &state.store_path;
            if !path.exists() {
                out.push((
                    name.clone(),
                    ReportKind::Warned,
                    format!("store path missing: {}", path.display()),
                ));
                continue;
            }
            // per-entry first: the manifest records every payload file's
            // hash at deploy — tampering with any file shows here, for
            // every module kind (config-only modules have no lock pin).
            // The recorded hash is what DEPLOY produced: merge entries
            // record the trimmed block's bytes hash, templates the
            // rendered output's — recomputing the same value keeps the
            // check honest (a raw file hash can never match those).
            let mut handled = false;
            for entry in &state.entries {
                let src = path.join(&entry.from);
                if !src.is_file() || src.is_symlink() {
                    continue;
                }
                let actual = match entry.mode {
                    gripsack_ir::Ownership::Merge => std::fs::read(&src).ok().map(|b| {
                        gripsack_store::canonical_bytes_hash(
                            String::from_utf8_lossy(&b)
                                .trim_end_matches('\n')
                                .as_bytes(),
                        )
                    }),
                    gripsack_ir::Ownership::Template => std::fs::read(&src)
                        .ok()
                        .and_then(|b| {
                            crate::template::render_template(&b, &entry.vars, &entry.from).ok()
                        })
                        .map(|r| gripsack_store::canonical_bytes_hash(&r)),
                    _ => gripsack_store::canonical_file_hash(&src).ok(),
                };
                if let Some(h) = &actual
                    && *h != entry.hash
                {
                    return_corrupt(
                        &mut out,
                        repair,
                        home,
                        path,
                        name,
                        &format!(
                            "corrupt: {} tampered (recorded {} ≠ {})",
                            src.display(),
                            hex_head(&entry.hash),
                            hex_head(h)
                        ),
                    )?;
                    handled = true;
                    break;
                }
            }
            if handled {
                continue; // removed (or reported) — nothing more to hash
            }
            // whole-tree: content-addressed modules (0014) record their
            // tree256 in the manifest — the expectation travels with
            // the generation, not with any host's lockfile
            if let Some(expected) = &state.tree256 {
                let actual = gripsack_store::canonical_tree_hash(path)?;
                if actual != *expected {
                    return_corrupt(
                        &mut out,
                        repair,
                        home,
                        path,
                        name,
                        &format!(
                            "corrupt: {} hashes {} but the manifest records {} — `grip store verify --repair` removes it",
                            path.display(),
                            hex_head(&actual),
                            hex_head(expected)
                        ),
                    )?;
                }
            }
        }
    }
    Ok(out)
}
/// Report (or repair) a corrupt store path. Repair removes it — the
/// next apply re-fetches from the pin. The path comes from a manifest,
/// which is disk state, not memory: repair refuses to remove anything
/// that does not sit inside `$home/store` — a tampered manifest must
/// not turn `--repair` into an arbitrary directory delete.
fn return_corrupt(
    out: &mut Vec<(String, ReportKind, String)>,
    repair: bool,
    home: &std::path::Path,
    path: &std::path::Path,
    name: &str,
    summary: &str,
) -> Result<(), ExecError> {
    if repair {
        let contained = home
            .join(gripsack_store::STORE_DIR)
            .canonicalize()
            .ok()
            .and_then(|root| path.canonicalize().ok().map(|p| p.starts_with(&root)))
            .unwrap_or(false);
        if !contained {
            out.push((
                name.to_string(),
                ReportKind::Warned,
                format!(
                    "refusing to repair {}: outside {}",
                    path.display(),
                    home.join(gripsack_store::STORE_DIR).display()
                ),
            ));
            return Ok(());
        }
        std::fs::remove_dir_all(path)?;
        out.push((
            name.to_string(),
            ReportKind::Configured,
            format!("removed corrupt {} (re-fetched next apply)", path.display()),
        ));
    } else {
        out.push((name.to_string(), ReportKind::Warned, summary.to_string()));
    }
    Ok(())
}

/// First 16 chars of a hash string — manifests are disk state, and a
/// tampered short hash must not panic the slice.
fn hex_head(hash: &str) -> &str {
    match hash.get(..16) {
        Some(head) => head,
        None => hash,
    }
}
