//! Generation allocation and durable publication.
use super::{Generation, admission::validate, current, list};
use crate::GenerationId;
use gripsack_fs::{Dir, cap_std, open_dir_nofollow, open_file_nofollow};
use std::{
    io::{self, Read},
    path::Path,
};
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
    let high_water = match open_dir_nofollow(home, Path::new(crate::GENERATIONS_DIR)) {
        Ok(directory) => observe_high_water(&directory)?.map(|observed| observed.number),
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

struct ObservedHighWater {
    number: GenerationId,
    file: cap_std::fs::File,
}

fn observe_high_water(directory: &Dir) -> io::Result<Option<ObservedHighWater>> {
    let mut file = match open_file_nofollow(directory, Path::new("high-water")) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    let number = text.trim().parse::<GenerationId>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "generations/high-water is corrupt — refusing allocation or pruning",
        )
    })?;
    Ok(Some(ObservedHighWater { number, file }))
}

/// Preserve all admitted allocation history before publishing or pruning names.
/// Legacy histories may lack the counter; existing counters may still be dirty
/// after an interrupted write. Neither case permits discarding the retained tip.
pub(super) fn preserve_allocation_floor(
    directory: &Dir,
    retained_maximum: Option<GenerationId>,
) -> io::Result<()> {
    let observed = observe_high_water(directory)?;
    let Some(floor) = retained_maximum.max(observed.as_ref().map(|value| value.number)) else {
        return Ok(());
    };
    match observed {
        Some(observed) if observed.number == floor => {
            gripsack_fs::fault::operation(
                gripsack_fs::fault::Boundary::FileSync,
                Path::new("high-water"),
                || observed.file.sync_all(),
            )?;
            gripsack_fs::fsync_pinned_dir(directory, Path::new(crate::GENERATIONS_DIR))
        }
        _ => gripsack_fs::atomic_write(
            directory,
            Path::new("high-water"),
            floor.to_string().as_bytes(),
        ),
    }
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
    let generations = open_dir_nofollow(home, Path::new(crate::GENERATIONS_DIR))?;
    preserve_allocation_floor(&generations, Some(generation.number))?;
    gripsack_fs::fsync_dir(home, &staging)?;
    gripsack_fs::rename(home, &staging, home, &final_dir).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("publish generation {}: {e}", generation.number),
        )
    })?;
    gripsack_fs::fsync_dir(home, Path::new(crate::paths::GENERATIONS_DIR))
}
