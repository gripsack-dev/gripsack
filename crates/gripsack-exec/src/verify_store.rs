//! `grip store verify [--repair]` (0008 §3 — the promise shipped):
//! re-hash every store path and report corruption. The re-hash walk
//! uses the same canonical tree hash that pinned the payload at fetch
//! (0.15.2's cross-implementation parity), so a mismatch is proof, not
//! suspicion.

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
    let locks: std::collections::BTreeMap<String, crate::lockfile::LockEntry> =
        crate::lockfile::read(&ctx.repo, &ctx.host)
            .map(|l| l.modules)
            .unwrap_or_default();
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
            let actual = gripsack_store::canonical_tree_hash(path)?;
            let pinned = locks
                .get(name)
                .and_then(|e| e.resolved.as_ref())
                .and_then(|r| r.sha256.clone());
            if let Some(expected) = pinned
                && actual != expected
            {
                return_corrupt(
                    &mut out,
                    repair,
                    path,
                    name,
                    &format!(
                        "corrupt: {} hashes {} but the lock pins {} — `grip store verify --repair` removes it",
                        path.display(),
                        &actual[..16],
                        &expected[..16]
                    ),
                )?;
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
