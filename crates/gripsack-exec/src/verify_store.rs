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
    for n in gripsack_store::list_generations(home) {
        let manifest = match gripsack_store::read_manifest(home, n) {
            Ok(m) => m,
            Err(_) => continue, // gc's fail-closed rule is gc's, not verify's
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
            // every module kind (config-only modules have no lock pin)
            let mut handled = false;
            for entry in &state.entries {
                let src = path.join(&entry.from);
                if src.is_file()
                    && !src.is_symlink()
                    && let Ok(h) = gripsack_store::canonical_file_hash(&src)
                    && h != entry.hash
                {
                    return_corrupt(
                        &mut out,
                        repair,
                        path,
                        name,
                        &format!(
                            "corrupt: {} tampered (recorded {} ≠ {})",
                            src.display(),
                            &entry.hash[..16],
                            &h[..16]
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
                        path,
                        name,
                        &format!(
                            "corrupt: {} hashes {} but the manifest records {} — `grip store verify --repair` removes it",
                            path.display(),
                            &actual[..16],
                            &expected[..16]
                        ),
                    )?;
                }
            }
        }
    }
    Ok(out)
}

/// Report (or repair) a corrupt store path. Repair removes it — the
/// next apply re-fetches from the pin.
fn return_corrupt(
    out: &mut Vec<(String, ReportKind, String)>,
    repair: bool,
    path: &std::path::Path,
    name: &str,
    summary: &str,
) -> Result<(), ExecError> {
    if repair {
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
