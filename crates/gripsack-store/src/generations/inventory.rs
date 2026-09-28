//! Generation inventory, manifest admission and pruning share one pinned root.
use super::{Generation, admission::validate};
use crate::{GENERATIONS_DIR, GenerationId, GenerationInventory, GenerationList};
use gripsack_fs::{
    Dir,
    fault::{Boundary, operation},
    open_dir_nofollow, open_file_nofollow,
};
use std::io::{self, Read};
use std::path::Path;

pub struct GenerationDirectory {
    directory: Option<Dir>,
    generations: GenerationList,
}

impl GenerationDirectory {
    /// Missing means an empty history; symlinks, non-directories and incomplete
    /// inventories are errors, never authority to prune a different directory.
    pub fn open(home: &Dir) -> io::Result<Self> {
        let directory = match open_dir_nofollow(home, Path::new(GENERATIONS_DIR)) {
            Ok(directory) => Some(directory),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let mut ids = Vec::new();
        if let Some(directory) = &directory {
            for entry in directory.entries()? {
                let name = entry?.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF-8 generation name"))?;
                match name.parse::<GenerationId>() {
                    Ok(id) => {
                        if name != id.to_string() {
                            return Err(invalid("noncanonical generation identity"));
                        }
                        // A numeric file/symlink must not impersonate a published generation.
                        open_dir_nofollow(directory, Path::new(name))?;
                        ids.push(id);
                    }
                    Err(_) if name.bytes().all(|byte| byte.is_ascii_digit()) => {
                        return Err(invalid("generation identity exceeds the wire range"));
                    }
                    Err(_) => {} // high-water and recognized non-generation staging names
                }
            }
        }
        ids.sort_unstable();
        let generations = GenerationList::new(ids)
            .ok_or_else(|| invalid("generation inventory is not strictly ascending"))?;
        Ok(Self {
            directory,
            generations,
        })
    }

    pub fn inventory(&self) -> GenerationInventory<'_> {
        self.generations.inventory()
    }

    pub fn into_generations(self) -> GenerationList {
        self.generations
    }

    /// Admit retained roots through the pinned inventory before GC effects.
    pub fn admit_manifest(
        &self,
        home_directory: &Dir,
        home: &Path,
        id: GenerationId,
    ) -> io::Result<Generation> {
        let directory = self
            .directory
            .as_ref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let manifest = read_manifest_in(directory, home, id, ManifestAccess::EffectAuthority)?;
        gripsack_fs::fsync_pinned_dir(home_directory, Path::new("."))?;
        Ok(manifest)
    }

    /// Remove the admitted batch, then seal this exact inventory directory
    /// before payload roots can be discarded. Even an empty batch must seal
    /// observed absence left by an earlier interrupted pruning attempt.
    pub fn prune(&self, ids: &[GenerationId]) -> io::Result<()> {
        let Some(directory) = &self.directory else {
            return if ids.is_empty() {
                Ok(())
            } else {
                Err(io::Error::from(io::ErrorKind::NotFound))
            };
        };
        super::publication::preserve_allocation_floor(
            directory,
            self.inventory().as_slice().last().copied(),
        )?;
        for id in ids {
            let name = id.to_string();
            gripsack_fs::fault::operation(
                gripsack_fs::fault::Boundary::Unlink,
                Path::new(&name),
                || directory.remove_dir_all(&name),
            )?;
        }
        gripsack_fs::fsync_pinned_dir(directory, Path::new(GENERATIONS_DIR))
    }
}

enum ManifestAccess {
    Inspection,
    EffectAuthority,
}

fn read_manifest_in(
    directory: &Dir,
    home: &Path,
    id: GenerationId,
    access: ManifestAccess,
) -> io::Result<Generation> {
    let name = id.to_string();
    let generation = open_dir_nofollow(directory, Path::new(&name))?;
    let mut file = open_file_nofollow(&generation, Path::new("manifest.json"))?;
    let mut raw = Vec::new();
    file.read_to_end(&mut raw)?;
    let manifest: Generation = serde_json::from_slice(&raw)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    validate(&manifest, id, home)?;
    if matches!(access, ManifestAccess::EffectAuthority) {
        // A previous publisher may have renamed successfully, then failed its
        // sync. Validation of cached bytes alone cannot authorize effects.
        operation(Boundary::FileSync, Path::new("manifest.json"), || {
            file.sync_all()
        })?;
        seal_observed_profile(&generation)?;
        gripsack_fs::fsync_pinned_dir(&generation, Path::new(&name))?;
        gripsack_fs::fsync_pinned_dir(directory, Path::new(GENERATIONS_DIR))?;
    }
    Ok(manifest)
}

fn seal_observed_profile(generation: &Dir) -> io::Result<()> {
    let directory = match open_dir_nofollow(generation, Path::new("env")) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    match open_file_nofollow(&directory, Path::new("profile.sh")) {
        Ok(file) => operation(Boundary::FileSync, Path::new("env/profile.sh"), || {
            file.sync_all()
        })?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    gripsack_fs::fsync_pinned_dir(&directory, Path::new("env"))
}

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}

/// All admitted generation numbers on disk, strictly ascending.
pub fn list(home: &Path) -> io::Result<GenerationList> {
    let home = match gripsack_fs::open(home) {
        Ok(home) => home,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(GenerationList::new(Vec::new()).expect("empty inventory is ordered"));
        }
        Err(error) => return Err(error),
    };
    Ok(GenerationDirectory::open(&home)?.into_generations())
}

/// Inspect persisted metadata without acquiring durability authority for effects.
pub fn read_manifest(home: &Path, id: GenerationId) -> io::Result<Generation> {
    let home_directory = gripsack_fs::open(home)?;
    let root = open_dir_nofollow(&home_directory, Path::new(GENERATIONS_DIR))?;
    read_manifest_in(&root, home, id, ManifestAccess::Inspection)
}

/// Validate and seal retained generation artifacts before they authorize effects.
pub fn admit_manifest(home: &Path, id: GenerationId) -> io::Result<Generation> {
    admit_manifest_at(&gripsack_fs::open(home)?, home, id)
}

/// Keep authoritative recovery reads on the same home capability as effects.
pub(crate) fn admit_manifest_at(
    home: &Dir,
    home_path: &Path,
    id: GenerationId,
) -> io::Result<Generation> {
    let root = open_dir_nofollow(home, Path::new(GENERATIONS_DIR))?;
    let manifest = read_manifest_in(&root, home_path, id, ManifestAccess::EffectAuthority)?;
    gripsack_fs::fsync_pinned_dir(home, Path::new("."))?;
    Ok(manifest)
}
