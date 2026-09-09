//! `grip store verify [--repair]` (0008 §3, corrected by 0014 §1a):
//! re-hash every store path and report corruption. Per-entry manifest
//! hashes cover every module kind; content-addressed modules also
//! carry `tree256` in the manifest, so whole-tree verification needs
//! no lockfile lookup (and never depends on the ambient hostname).

use crate::ctx::ExecError;
use crate::report::ReportKind;

/// Every store path, re-hashed against the lockfile's recorded pins.
/// Corrupt paths are reported; with `repair` they're removed (the next
/// apply re-fetches — publish_dir's refusal becomes a republish).
/// Requires a [`LifecycleSession`] (0045 F4): repair deletes store
/// paths, so it runs under the same serialization as apply/gc —
/// expressed in the signature, not left to the caller.
pub fn verify_store(
    session: &crate::LifecycleSession,
    repair: bool,
) -> Result<Vec<(String, ReportKind, String)>, ExecError> {
    let mut out = Vec::new();
    let home = session.home();
    let current = gripsack_store::current_generation(home)?;
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
                // Only the active generation claims the live destination.
                // Drift is not store corruption: --repair must never remove a
                // valid store artifact because its deployed file was chmodded.
                if current == Some(n)
                    && let Some(expected) = entry.file_mode
                    && matches!(
                        entry.mode,
                        gripsack_ir::Ownership::Template | gripsack_ir::Ownership::Merge
                    )
                    && let Ok(Some(crate::deploy::Observation::File { mode, .. })) =
                        crate::deploy::observe_readonly(&entry.key())
                    && mode != expected
                {
                    out.push((
                        name.clone(),
                        ReportKind::Warned,
                        format!(
                            "drift: {} mode 0{mode:o} differs from recorded 0{expected:o}",
                            entry.to
                        ),
                    ));
                }
                // a preserved-drift entry records an OBSERVATION, not
                // a store deployment (0029 §2) — there is no store
                // content claim to verify
                if entry.preserved_drift {
                    continue;
                }
                let src = path.join(&entry.from);
                let metadata = match std::fs::symlink_metadata(&src) {
                    Ok(metadata) if metadata.is_file() => metadata,
                    _ => continue,
                };
                #[cfg(unix)]
                let source_exec = {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o111 != 0
                };
                #[cfg(not(unix))]
                let source_exec = {
                    let _ = metadata;
                    false
                };
                let source_exec_changed = matches!(
                    entry.mode,
                    gripsack_ir::Ownership::TrackedCopy | gripsack_ir::Ownership::Template
                ) && entry
                    .source_executable
                    .is_some_and(|expected| expected != source_exec);
                // each arm recomputes in the entry's manifest domain
                // (0043): merge blocks are bytes-only; whole-file outputs
                // include permissions. Historical receipts retain their domain.
                // owned links record the payload's store identity
                let actual = match entry.mode {
                    gripsack_ir::Ownership::Merge => std::fs::read(&src).ok().map(|b| {
                        let text = String::from_utf8_lossy(&b);
                        let legacy = gripsack_store::canonical_bytes_hash(
                            text.trim_end_matches('\n').as_bytes(),
                        );
                        if legacy.as_str() == entry.hash.as_str() {
                            legacy.to_string()
                        } else {
                            crate::managed_blocks::content_hash(&text).to_string()
                        }
                    }),
                    gripsack_ir::Ownership::Template => std::fs::read(&src)
                        .ok()
                        .and_then(|b| {
                            crate::template::render_template(
                                &b,
                                &entry.vars,
                                &entry.from.to_string_lossy(),
                            )
                            .ok()
                        })
                        .map(|rendered| {
                            let legacy = gripsack_store::canonical_bytes_hash(&rendered);
                            if legacy.as_str() == entry.hash.as_str() {
                                legacy.to_string()
                            } else {
                                let mode = entry.file_mode.unwrap_or(if source_exec {
                                    0o755
                                } else {
                                    0o644
                                });
                                gripsack_store::canonical_bytes_identity(&rendered, mode)
                                    .to_string()
                            }
                        }),
                    gripsack_ir::Ownership::Owned => gripsack_store::canonical_file_hash(&src)
                        .ok()
                        .map(|h| h.to_string()),
                    gripsack_ir::Ownership::TrackedCopy => std::fs::read(&src).ok().map(|bytes| {
                        // Pre-0041 copy receipts hashed the nominal source mode,
                        // even for private takeovers. The new source field marks
                        // receipts whose hash includes the actual landed mode.
                        let mode = entry
                            .source_executable
                            .and(entry.file_mode)
                            .unwrap_or(if source_exec { 0o755 } else { 0o644 });
                        gripsack_store::canonical_bytes_identity(&bytes, mode).to_string()
                    }),
                };
                if let Some(h) = &actual
                    && (source_exec_changed || h.as_str() != entry.hash.as_str())
                {
                    let detail = if source_exec_changed {
                        format!("corrupt: {} source executable bit changed", src.display())
                    } else {
                        format!(
                            "corrupt: {} tampered (recorded {} ≠ {})",
                            src.display(),
                            hex_head(entry.hash.as_str()),
                            hex_head(h)
                        )
                    };
                    return_corrupt(&mut out, repair, home, path, name, &detail)?;
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
                if actual.as_str() != expected.as_str() {
                    return_corrupt(
                        &mut out,
                        repair,
                        home,
                        path,
                        name,
                        &format!(
                            "corrupt: {} hashes {} but the manifest records {} — `grip store verify --repair` removes it",
                            path.display(),
                            hex_head(actual.as_str()),
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
