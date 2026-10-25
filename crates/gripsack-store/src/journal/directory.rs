//! Directory ownership is separate from leaf content ownership. A receipt
//! grants authority over exact directory inodes, never arbitrary descendants.
pub use super::directory_binding::{bind_existing, destination_capability};
pub(super) use super::directory_binding::{bound_parent, recovery_parent};
pub use super::directory_evolution::{
    EvolutionPlan, finish, is_directory, plans, receipts, restore_shape,
};
pub use super::directory_prepare::{prepare, publish, reactivate};
pub use super::directory_resolve::{
    ParentInspection, canonical_destination, canonical_for_receipts, inspect_parent,
};
#[cfg(test)]
pub(super) use super::directory_wire::record_name;
pub(super) use super::directory_wire::{Direction, DirectoryRecord, persist, read_records};
use gripsack_fs::Dir;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryIdentity {
    filesystem_kind: i64,
    filesystem_id: u64,
    inode: u64,
    birth_seconds: i64,
    birth_nanoseconds: u32,
}

impl DirectoryIdentity {
    pub fn observe(directory: &Dir, name: &Path) -> io::Result<Self> {
        Self::observe_pinned(&gripsack_fs::open_dir_nofollow(directory, name)?)
    }

    fn observe_pinned(pinned: &Dir) -> io::Result<Self> {
        let object = gripsack_fs::directory_object_identity(pinned)?;
        Ok(Self {
            filesystem_kind: object.filesystem_kind,
            filesystem_id: object.filesystem_id,
            inode: object.inode,
            birth_seconds: object.birth_seconds,
            birth_nanoseconds: object.birth_nanoseconds,
        })
    }
    pub fn verify(self, directory: &Dir, name: &Path) -> io::Result<()> {
        if Self::observe(directory, name)? != self {
            return Err(invalid(
                "owned directory inode changed; foreign replacement preserved",
            ));
        }
        Ok(())
    }

    /// Open `name` once and admit it only as this exact verified inode. The
    /// returned capability is the sole basis for later mutation: verifying the
    /// name and then re-opening it would admit a replacement swapped in
    /// between the check and the use.
    pub fn open_verified(self, directory: &Dir, name: &Path) -> io::Result<Dir> {
        let pinned = gripsack_fs::open_dir_nofollow(directory, name)?;
        if Self::observe_pinned(&pinned)? != self {
            return Err(invalid(
                "owned directory inode changed; foreign replacement preserved",
            ));
        }
        Ok(pinned)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryTransition {
    pub destination: PathBuf,
    pub predecessor_target: PathBuf,
    #[serde(with = "crate::generation_wire")]
    pub predecessor_generation: crate::GenerationId,
    /// Relative paths, including the empty path for the root itself.
    #[serde(deserialize_with = "unique_map")]
    pub directories: BTreeMap<PathBuf, DirectoryIdentity>,
    /// Independently retained empty nodes, always siblings of the root.
    #[serde(deserialize_with = "unique_map")]
    pub directory_staging: BTreeMap<PathBuf, PathBuf>,
    /// Observed real ancestors grant traversal, never directory removal.
    #[serde(deserialize_with = "unique_map")]
    pub borrowed_directories: BTreeMap<PathBuf, DirectoryIdentity>,
    pub leaves: Vec<PathBuf>,
    pub(crate) staging: PathBuf,
    pub(crate) parent_identity: DirectoryIdentity,
}

impl DirectoryTransition {
    pub fn validate(&self) -> io::Result<()> {
        if !self.destination.is_absolute()
            || !self.predecessor_target.is_absolute()
            || self.destination.file_name().is_none()
            || self.staging.parent() != self.destination.parent()
            || !self
                .staging
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".gripsack-directory-"))
            || !self.directories.contains_key(Path::new(""))
            || self.leaves.is_empty()
        {
            return Err(invalid("invalid directory transition receipt"));
        }
        for path in [&self.destination, &self.predecessor_target, &self.staging] {
            if path.to_str().is_none()
                || path
                    .components()
                    .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            {
                return Err(invalid(
                    "directory transition contains an unsafe absolute path",
                ));
            }
        }
        for path in self
            .directories
            .keys()
            .chain(self.borrowed_directories.keys())
            .chain(self.leaves.iter())
        {
            if path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(invalid(
                    "directory transition contains a non-relative descendant",
                ));
            }
        }
        for path in self
            .directories
            .keys()
            .chain(self.borrowed_directories.keys())
            .filter(|path| !path.as_os_str().is_empty())
        {
            if !self
                .directories
                .contains_key(path.parent().unwrap_or(Path::new("")))
                && !self
                    .borrowed_directories
                    .contains_key(path.parent().unwrap_or(Path::new("")))
            {
                return Err(invalid(
                    "directory transition lacks an intermediate ancestor",
                ));
            }
        }
        if self
            .borrowed_directories
            .keys()
            .any(|path| self.directories.contains_key(path))
            || self.directory_staging.len() + 1 != self.directories.len()
        {
            return Err(invalid(
                "directory ownership and borrowed/staging authority disagree",
            ));
        }
        let mut stages = std::collections::BTreeSet::from([&self.staging]);
        for (relative, stage) in &self.directory_staging {
            if relative.as_os_str().is_empty()
                || !self.directories.contains_key(relative)
                || stage.parent() != self.destination.parent()
                || !stage
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".gripsack-directory-"))
                || !stages.insert(stage)
                || stage
                    .components()
                    .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            {
                return Err(invalid("invalid owned directory node staging identity"));
            }
        }
        let identities: std::collections::BTreeSet<_> = self
            .directories
            .values()
            .chain(self.borrowed_directories.values())
            .collect();
        if identities.len() != self.directories.len() + self.borrowed_directories.len()
            || identities
                .iter()
                .copied()
                .chain(std::iter::once(&self.parent_identity))
                .any(|identity| {
                    identity.filesystem_id == 0
                        || identity.inode == 0
                        || identity.birth_nanoseconds >= 1_000_000_000
                })
        {
            return Err(invalid("invalid or aliased directory birth identities"));
        }
        if self
            .leaves
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != self.leaves.len()
        {
            return Err(invalid("duplicate directory transition leaf"));
        }
        for leaf in &self.leaves {
            if leaf.as_os_str().is_empty()
                || self.directories.contains_key(leaf)
                || self.borrowed_directories.contains_key(leaf)
                || !(self
                    .directories
                    .contains_key(leaf.parent().unwrap_or(Path::new("")))
                    || self
                        .borrowed_directories
                        .contains_key(leaf.parent().unwrap_or(Path::new(""))))
            {
                return Err(invalid("invalid owned directory leaf"));
            }
        }
        Ok(())
    }

    pub fn parent(&self) -> io::Result<Dir> {
        self.validate()?;
        let parent = gripsack_fs::open(
            self.destination
                .parent()
                .ok_or_else(|| invalid("missing parent"))?,
        )?;
        self.parent_identity.verify(&parent, Path::new("."))?;
        Ok(parent)
    }

    pub fn verify_borrowed(&self) -> io::Result<()> {
        let outer = self.parent()?;
        let name = Path::new(
            self.destination
                .file_name()
                .ok_or_else(|| invalid("missing root name"))?,
        );
        let root = self.directories[Path::new("")].open_verified(&outer, name)?;
        for relative in self.borrowed_directories.keys() {
            self.open_relative(&root, relative)?;
        }
        Ok(())
    }

    /// Open each component no-follow and verify every persisted directory inode.
    pub fn leaf_parent(&self, destination: &Path) -> io::Result<(Dir, PathBuf)> {
        self.leaf_parent_with(destination, &BTreeMap::new())
    }

    pub(super) fn leaf_parent_with(
        &self,
        destination: &Path,
        supplemental: &BTreeMap<PathBuf, DirectoryIdentity>,
    ) -> io::Result<(Dir, PathBuf)> {
        let relative = destination
            .strip_prefix(&self.destination)
            .map_err(|_| invalid("leaf outside owned directory"))?;
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(invalid("invalid logical leaf below directory transition"));
        }
        let parent = self.parent()?;
        let root_name = self
            .destination
            .file_name()
            .ok_or_else(|| invalid("missing directory name"))?;
        let root = self.directories[Path::new("")].open_verified(&parent, Path::new(root_name))?;
        let directory = self.open_relative_with(
            &root,
            relative.parent().unwrap_or(Path::new("")),
            supplemental,
        )?;
        Ok((
            directory,
            PathBuf::from(
                relative
                    .file_name()
                    .ok_or_else(|| invalid("missing leaf name"))?,
            ),
        ))
    }

    pub(super) fn open_relative(&self, root: &Dir, relative: &Path) -> io::Result<Dir> {
        self.open_relative_with(root, relative, &BTreeMap::new())
    }

    fn open_relative_with(
        &self,
        root: &Dir,
        relative: &Path,
        supplemental: &BTreeMap<PathBuf, DirectoryIdentity>,
    ) -> io::Result<Dir> {
        self.directories[Path::new("")].verify(root, Path::new("."))?;
        let mut directory = root.try_clone()?;
        let mut walked = PathBuf::new();
        for component in relative.components() {
            walked.push(component.as_os_str());
            let identity = self
                .directories
                .get(&walked)
                .or_else(|| self.borrowed_directories.get(&walked))
                .or_else(|| supplemental.get(&walked))
                .ok_or_else(|| invalid("missing directory ancestor authority"))?;
            directory = identity.open_verified(&directory, Path::new(component.as_os_str()))?;
        }
        Ok(directory)
    }
}

pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn unique_map<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<BTreeMap<PathBuf, T>, D::Error> {
    struct UniqueDirectories<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for UniqueDirectories<T> {
        type Value = BTreeMap<PathBuf, T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("unique relative directory identities")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut directories = BTreeMap::new();
            while let Some((path, identity)) = map.next_entry()? {
                if directories.insert(path, identity).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate directory identity path",
                    ));
                }
            }
            Ok(directories)
        }
    }
    deserializer.deserialize_map(UniqueDirectories(std::marker::PhantomData))
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn open_verified_admits_only_the_pinned_inode() {
        let temporary = tempfile::tempdir().unwrap();
        let parent = gripsack_fs::open_or_create(temporary.path()).unwrap();
        std::fs::create_dir(temporary.path().join("owned")).unwrap();
        let identity = DirectoryIdentity::observe(&parent, Path::new("owned")).unwrap();
        std::fs::rename(
            temporary.path().join("owned"),
            temporary.path().join("staged"),
        )
        .unwrap();
        // Same name, different inode: a name-based re-open would hand back the
        // replacement; the verified open must refuse it.
        std::fs::create_dir(temporary.path().join("owned")).unwrap();
        assert!(identity.open_verified(&parent, Path::new("owned")).is_err());
        let pinned = identity
            .open_verified(&parent, Path::new("staged"))
            .unwrap();
        assert_eq!(
            DirectoryIdentity::observe(&pinned, Path::new(".")).unwrap(),
            identity
        );
    }
}
