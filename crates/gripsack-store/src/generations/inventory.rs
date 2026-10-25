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

const RETIRING: &str = ".retiring-";
const RETIRED: &str = ".retired-";
const MAX_RETIREMENTS: usize = 4096;
const MAX_RETIREMENT_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

pub struct GenerationDirectory {
    directory: Option<Dir>,
    generations: GenerationList,
    retiring: Vec<GenerationId>,
    retired: Vec<GenerationId>,
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
        let mut retiring = Vec::new();
        let mut retired = Vec::new();
        if let Some(directory) = &directory {
            for entry in directory.entries()? {
                let name = entry?.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF-8 generation name"))?;
                if let Some((prefix, suffix)) = name
                    .strip_prefix(RETIRING)
                    .map(|suffix| (RETIRING, suffix))
                    .or_else(|| name.strip_prefix(RETIRED).map(|suffix| (RETIRED, suffix)))
                {
                    let id = suffix
                        .parse::<GenerationId>()
                        .map_err(|_| invalid("invalid generation retirement identity"))?;
                    if suffix != id.to_string() {
                        return Err(invalid("noncanonical generation retirement identity"));
                    }
                    open_dir_nofollow(directory, Path::new(name))?;
                    if prefix == RETIRING {
                        retiring.push(id);
                    } else {
                        retired.push(id);
                    }
                    if retiring.len() + retired.len() > MAX_RETIREMENTS {
                        return Err(invalid("generation retirement inventory exceeds limit"));
                    }
                    continue;
                }
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
        retiring.sort_unstable();
        retired.sort_unstable();
        if retiring
            .iter()
            .chain(&retired)
            .any(|id| generations.inventory().as_slice().binary_search(id).is_ok())
            || retiring.iter().any(|id| retired.binary_search(id).is_ok())
        {
            return Err(invalid("generation and retirement identities overlap"));
        }
        Ok(Self {
            directory,
            generations,
            retiring,
            retired,
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
        let manifest = read_manifest_named(
            directory,
            home,
            id,
            &id.to_string(),
            ManifestAccess::EffectAuthority,
            Some(MAX_RETIREMENT_MANIFEST_BYTES),
        )?;
        gripsack_fs::fsync_pinned_dir(home_directory, Path::new("."))?;
        Ok(manifest)
    }

    /// Read-only GC inspection, through the same pinned inventory.
    pub fn inspect_manifest(&self, home: &Path, id: GenerationId) -> io::Result<Generation> {
        let directory = self
            .directory
            .as_ref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        read_manifest_named(
            directory,
            home,
            id,
            &id.to_string(),
            ManifestAccess::Inspection,
            Some(MAX_RETIREMENT_MANIFEST_BYTES),
        )
    }

    pub fn retiring(&self) -> &[GenerationId] {
        &self.retiring
    }

    /// Retirement authority is the original immutable manifest, not a copied
    /// subset of its identities. Inspection does not chmod, create or sync.
    pub fn retirement_manifest(
        &self,
        home_directory: &Dir,
        home: &Path,
        id: GenerationId,
        dry_run: bool,
    ) -> io::Result<Generation> {
        if self.retiring.binary_search(&id).is_err() {
            return Err(invalid("retirement is not in the admitted inventory"));
        }
        let directory = self
            .directory
            .as_ref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let manifest = read_manifest_named(
            directory,
            home,
            id,
            &format!("{RETIRING}{id}"),
            if dry_run {
                ManifestAccess::Inspection
            } else {
                ManifestAccess::EffectAuthority
            },
            Some(MAX_RETIREMENT_MANIFEST_BYTES),
        )?;
        if !dry_run {
            gripsack_fs::fsync_pinned_dir(home_directory, Path::new("."))?;
        }
        Ok(manifest)
    }

    /// Move the admitted batch out of retained history without destroying its
    /// receipt authority, then seal absence before any staging/payload deletion.
    /// Empty retries seal earlier interrupted renames as well.
    pub fn prune(&self, ids: &[GenerationId]) -> io::Result<()> {
        let Some(directory) = &self.directory else {
            return if ids.is_empty() {
                Ok(())
            } else {
                Err(io::Error::from(io::ErrorKind::NotFound))
            };
        };
        if ids.len() + self.retiring.len() + self.retired.len() > MAX_RETIREMENTS {
            return Err(invalid("generation retirement inventory exceeds limit"));
        }
        super::publication::preserve_allocation_floor(
            directory,
            self.inventory()
                .as_slice()
                .last()
                .copied()
                .max(self.retiring.last().copied())
                .max(self.retired.last().copied()),
        )?;
        // Receipt predecessors are strictly older. Remove descendants first:
        // interruption must not leave a live receipt with a pruned predecessor.
        for id in ids.iter().rev() {
            let name = id.to_string();
            gripsack_fs::rename_noreplace(
                directory,
                Path::new(&name),
                Path::new(&format!("{RETIRING}{id}")),
            )?;
        }
        gripsack_fs::fsync_pinned_dir(directory, Path::new(GENERATIONS_DIR))
    }

    /// Only call after every staging deletion (including observed absence) is
    /// sealed. The second namespace is cleanup-only: partial recursive removal
    /// can no longer destroy authority needed to finish directory retirement.
    pub fn finish_retirement(&self, id: GenerationId) -> io::Result<()> {
        let directory = self
            .directory
            .as_ref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        gripsack_fs::rename_noreplace(
            directory,
            Path::new(&format!("{RETIRING}{id}")),
            Path::new(&format!("{RETIRED}{id}")),
        )?;
        gripsack_fs::fsync_pinned_dir(directory, Path::new(GENERATIONS_DIR))?;
        remove_retired(directory, id)
    }

    pub fn cleanup_retired(&self) -> io::Result<()> {
        if let Some(directory) = &self.directory {
            // Seal a previous successful rename even if its sync failed.
            gripsack_fs::fsync_pinned_dir(directory, Path::new(GENERATIONS_DIR))?;
            for id in &self.retired {
                remove_retired(directory, *id)?;
            }
        }
        Ok(())
    }
}

fn remove_retired(directory: &Dir, id: GenerationId) -> io::Result<()> {
    let name = format!("{RETIRED}{id}");
    operation(Boundary::Unlink, Path::new(&name), || {
        directory.remove_dir_all(&name)
    })?;
    gripsack_fs::fsync_pinned_dir(directory, Path::new(GENERATIONS_DIR))
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
    read_manifest_named(directory, home, id, &id.to_string(), access, None)
}

fn read_manifest_named(
    directory: &Dir,
    home: &Path,
    id: GenerationId,
    name: &str,
    access: ManifestAccess,
    limit: Option<u64>,
) -> io::Result<Generation> {
    let generation = open_dir_nofollow(directory, Path::new(name))?;
    let mut file = open_file_nofollow(&generation, Path::new("manifest.json"))?;
    let mut raw = Vec::new();
    // Bound GC admission before pruning, so every admitted live receipt is
    // also readable as retirement authority after a crash.
    (&mut file)
        .take(limit.map_or(u64::MAX, |limit| limit + 1))
        .read_to_end(&mut raw)?;
    if limit.is_some_and(|limit| raw.len() as u64 > limit) {
        return Err(invalid(
            "generation manifest exceeds retirement authority limit",
        ));
    }
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
        gripsack_fs::fsync_pinned_dir(&generation, Path::new(name))?;
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
