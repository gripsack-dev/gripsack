//! Generation allocation and durable publication.
use super::{Generation, admission::validate, current, list};
use crate::GenerationId;
use std::{io, path::Path};
/// Write a generation's manifest atomically, relative to the home
/// capability (plan/0021): `generations/<N>/manifest.json` can never
/// be redirected by a swapped path component.
pub fn write_manifest(home: &gripsack_fs::Dir, generation: &Generation) -> io::Result<()> {
    // generations are immutable history (0026 §3): an existing
    // generation number is a hard invariant failure, never an
    // overwrite target (the pre-0.23 current+1 allocator could reuse
    // one after a rollback)
    let gen_rel = Path::new(crate::paths::GENERATIONS_DIR).join(generation.number.to_string());
    if home.symlink_metadata(&gen_rel).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "generation {} already exists — generations are immutable",
                generation.number
            ),
        ));
    }
    let json = serde_json::to_string_pretty(generation)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    gripsack_fs::atomic_write(
        home,
        &Path::new(crate::paths::GENERATIONS_DIR)
            .join(generation.number.to_string())
            .join("manifest.json"),
        json.as_bytes(),
    )
}

/// Durable high-water mark: `generations/high-water` holds the highest
/// generation number ever allocated (0027 §9). Without it, gc of the
/// tip moves the on-disk maximum backward and IDs get reused — logs
/// and journal remnants would then name two different states "2".
pub fn allocate(home_path: &Path, home: &gripsack_fs::Dir) -> io::Result<GenerationId> {
    let high_water = match home.read("generations/high-water") {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let n = text.trim().parse::<GenerationId>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "generations/high-water is corrupt — refusing to allocate",
                )
            })?;
            Some(n)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let next = [
        current(home_path)?,
        list(home_path)?.into_iter().max(),
        high_water,
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or_else(|| GenerationId::new(0))
    .checked_next()
    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "generation identity exhausted"))?;
    Ok(next)
}

/// Publish a complete generation as ONE object (0027 §8): manifest and
/// profile are staged under `generations/.staging-<N>` and renamed
/// into place with a no-clobber check — a failed apply never leaves a
/// half-populated `generations/N` visible to listings, allocation, or
/// rollback. A leftover staging dir from a crashed apply is not a
/// generation and is cleared first.
pub fn publish_generation(
    home: &gripsack_fs::Dir,
    generation: &Generation,
    profile: Option<&str>,
    home_path: &Path,
) -> io::Result<()> {
    // construction and load share the ONE validator (0029 §7): we
    // never publish what read_manifest would reject
    validate(generation, generation.number, home_path)?;
    // pid-tagged (0030 §18): a staging dir from a crashed apply is
    // cleared only when it's ours to clear
    let staging = Path::new(crate::paths::GENERATIONS_DIR).join(format!(
        ".staging-{}-{}",
        generation.number,
        std::process::id()
    ));
    let final_dir = Path::new(crate::paths::GENERATIONS_DIR).join(generation.number.to_string());
    if home.symlink_metadata(&final_dir).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "generation {} already exists — generations are immutable",
                generation.number
            ),
        ));
    }
    match home.remove_dir_all(&staging) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let json = serde_json::to_string_pretty(generation)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    gripsack_fs::atomic_write(home, &staging.join("manifest.json"), json.as_bytes())?;
    if let Some(profile) = profile {
        gripsack_fs::atomic_write(
            home,
            &staging.join("env").join("profile.sh"),
            profile.as_bytes(),
        )?;
    }
    // the high-water mark moves BEFORE the rename (0029 §9 ordering):
    // a failure after the rename must never leave a visible generation
    // the allocator doesn't know about
    gripsack_fs::atomic_write(
        home,
        Path::new("generations/high-water"),
        generation.number.to_string().as_bytes(),
    )?;
    gripsack_fs::fsync_dir(home, &staging)?;
    gripsack_fs::rename(home, &staging, home, &final_dir).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("publish generation {}: {e}", generation.number),
        )
    })?;
    gripsack_fs::fsync_dir(home, Path::new(crate::paths::GENERATIONS_DIR))
}
