//! Bounded source inventories identify copied bytes, not a mutable source tree.
use crate::prior::FileMode;
use gripsack_process::Sha256Digest;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::{self, Write},
    path::{Component, Path},
};

pub(super) const INVENTORY_VERSION: u32 = 1;
/// Capture policy: metadata is bounded separately from file contents.
pub(super) const MAX_INVENTORY_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_ENTRIES: usize = 100_000;
pub(super) const MAX_DEPTH: usize = 128;
pub(super) const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
pub(super) const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const MAX_LINK_EXPANSIONS: usize = 40;
pub(super) const MAX_RESOLUTION_STEPS: usize = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceBundleDigest(Sha256Digest);

impl SourceBundleDigest {
    pub fn parse(value: &str) -> io::Result<Self> {
        Sha256Digest::parse(value).map(Self)
    }

    pub(super) fn of_inventory(bytes: &[u8]) -> Self {
        Self(Sha256Digest::of(bytes))
    }
}

impl fmt::Display for SourceBundleDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRootKind {
    Repository,
    Frontend,
    PinnedFrontend,
}

impl SourceRootKind {
    pub fn directory(self) -> &'static str {
        match self {
            Self::Repository => "repo",
            Self::Frontend => "frontend",
            Self::PinnedFrontend => "pin",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SourceFileBytes(u64);

impl SourceFileBytes {
    pub fn bytes(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for SourceFileBytes {
    type Error = io::Error;
    fn try_from(bytes: u64) -> io::Result<Self> {
        if bytes > MAX_FILE_BYTES {
            return Err(invalid("source file exceeds its byte budget"));
        }
        Ok(Self(bytes))
    }
}

impl<'de> Deserialize<'de> for SourceFileBytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceObject {
    Directory,
    File {
        bytes: SourceFileBytes,
        sha256: Sha256Digest,
        /// Original permissions inform native materialization; bundle files
        /// themselves are private/read-only and preserve only executability.
        mode: FileMode,
    },
    Alias {
        /// Resolved root-qualified object inside this admitted inventory.
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceEntry {
    pub path: String,
    pub object: SourceObject,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(try_from = "InventoryWire")]
pub struct SourceInventory {
    version: u32,
    roots: Vec<SourceRootKind>,
    entries: Vec<SourceEntry>,
    /// Actual excluded paths, never an implicit gitignore filter.
    exclusions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryWire {
    version: u32,
    roots: Vec<SourceRootKind>,
    entries: Vec<SourceEntry>,
    exclusions: Vec<String>,
}

impl TryFrom<InventoryWire> for SourceInventory {
    type Error = io::Error;
    fn try_from(wire: InventoryWire) -> io::Result<Self> {
        let inventory = Self {
            version: wire.version,
            roots: wire.roots,
            entries: wire.entries,
            exclusions: wire.exclusions,
        };
        inventory.validate()?;
        Ok(inventory)
    }
}

impl SourceInventory {
    pub(super) fn new(
        mut roots: Vec<SourceRootKind>,
        mut entries: Vec<SourceEntry>,
        mut exclusions: Vec<String>,
    ) -> io::Result<Self> {
        roots.sort_unstable();
        entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        exclusions.sort_unstable();
        let inventory = Self {
            version: INVENTORY_VERSION,
            roots,
            entries,
            exclusions,
        };
        inventory.validate()?;
        Ok(inventory)
    }

    pub fn roots(&self) -> &[SourceRootKind] {
        &self.roots
    }
    pub fn entries(&self) -> &[SourceEntry] {
        &self.entries
    }
    pub fn exclusions(&self) -> &[String] {
        &self.exclusions
    }

    pub fn entry(&self, path: &str) -> Option<&SourceEntry> {
        self.entries
            .binary_search_by(|entry| entry.path.as_str().cmp(path))
            .ok()
            .map(|index| &self.entries[index])
    }

    pub fn decode(bytes: &[u8], expected: SourceBundleDigest) -> io::Result<Self> {
        if bytes.len() > MAX_INVENTORY_BYTES || SourceBundleDigest::of_inventory(bytes) != expected
        {
            return Err(invalid(
                "source inventory digest or size does not match its approval",
            ));
        }
        serde_json::from_slice(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub fn encode(&self) -> io::Result<Vec<u8>> {
        struct Bounded(Vec<u8>);
        impl Write for Bounded {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if bytes.len() > MAX_INVENTORY_BYTES - self.0.len() {
                    return Err(invalid("source inventory exceeds its byte budget"));
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut output = Bounded(Vec::new());
        serde_json::to_writer(&mut output, self).map_err(io::Error::other)?;
        Ok(output.0)
    }

    fn validate(&self) -> io::Result<()> {
        if self.version != INVENTORY_VERSION {
            return Err(invalid("unsupported source inventory version"));
        }
        if !matches!(
            self.roots.as_slice(),
            [SourceRootKind::Repository, SourceRootKind::Frontend]
                | [
                    SourceRootKind::Repository,
                    SourceRootKind::Frontend,
                    SourceRootKind::PinnedFrontend
                ]
        ) {
            return Err(invalid(
                "source inventory has missing, duplicate or unordered roots",
            ));
        }
        if self.entries.len() > MAX_ENTRIES || self.exclusions.len() > MAX_ENTRIES {
            return Err(invalid("source inventory exceeds its entry budget"));
        }
        let valid_path = |value: &str| {
            let path = Path::new(value);
            let mut components = path.components();
            let Some(Component::Normal(first)) = components.next() else {
                return false;
            };
            self.roots.iter().any(|root| first == root.directory())
                && path.components().count() <= MAX_DEPTH
                && components.all(|component| matches!(component, Component::Normal(_)))
                && !value.contains('\0')
                && path.to_str() == Some(value)
                && !value.ends_with('/')
                && !value.contains("//")
                && !value
                    .split('/')
                    .any(|component| component == "." || component == "..")
        };
        let mut total = 0_u64;
        let mut previous: Option<&str> = None;
        for entry in &self.entries {
            if !valid_path(&entry.path) || previous.is_some_and(|path| path >= entry.path.as_str())
            {
                return Err(invalid(
                    "source inventory paths are invalid, duplicated or unordered",
                ));
            }
            previous = Some(&entry.path);
            if let Some(parent) = entry.path.rsplit_once('/').map(|(parent, _)| parent)
                && !self
                    .entry(parent)
                    .is_some_and(|entry| entry.object == SourceObject::Directory)
            {
                return Err(invalid(
                    "source inventory has a missing or non-directory parent",
                ));
            }
            match &entry.object {
                SourceObject::File { bytes, .. } => {
                    total = total
                        .checked_add(bytes.bytes())
                        .ok_or_else(|| invalid("source byte count overflow"))?;
                    if total > MAX_SOURCE_BYTES {
                        return Err(invalid("source inventory exceeds its content budget"));
                    }
                }
                SourceObject::Alias { target } => {
                    if !valid_path(target)
                        || !self.entry(target).is_some_and(|entry| {
                            !matches!(entry.object, SourceObject::Alias { .. })
                        })
                    {
                        return Err(invalid(
                            "source alias does not resolve to an admitted object",
                        ));
                    }
                }
                SourceObject::Directory => {}
            }
        }
        for root in &self.roots {
            if !self
                .entry(root.directory())
                .is_some_and(|entry| entry.object == SourceObject::Directory)
            {
                return Err(invalid("source inventory is missing a root directory"));
            }
        }
        let mut previous: Option<&str> = None;
        for path in &self.exclusions {
            if !valid_path(path)
                || previous.is_some_and(|prior| prior >= path.as_str())
                || self.entry(path).is_some()
            {
                return Err(invalid(
                    "source exclusions are invalid or overlap captured objects",
                ));
            }
            previous = Some(path);
        }
        self.reject_directory_cycles()
    }

    fn reject_directory_cycles(&self) -> io::Result<()> {
        // Real directory edges and resolved directory aliases form one graph.
        // DFS colors bound each entry to one completion; aliases cannot turn
        // recursive consumers into an unbounded traversal of the snapshot.
        let mut children = vec![Vec::new(); self.entries.len()];
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some((parent, _)) = entry.path.rsplit_once('/') {
                let parent = self
                    .entries
                    .binary_search_by(|entry| entry.path.as_str().cmp(parent))
                    .unwrap();
                children[parent].push(index);
            }
            if let SourceObject::Alias { target } = &entry.object {
                let target = self
                    .entries
                    .binary_search_by(|entry| entry.path.as_str().cmp(target))
                    .unwrap();
                if self.entries[target].object == SourceObject::Directory {
                    children[index].push(target);
                }
            }
        }
        #[derive(Clone, Copy, PartialEq)]
        enum Visit {
            Unvisited,
            Active,
            Complete,
        }
        enum Work {
            Enter(usize),
            Leave(usize),
        }
        let mut states = vec![Visit::Unvisited; self.entries.len()];
        for root in 0..self.entries.len() {
            if states[root] != Visit::Unvisited {
                continue;
            }
            let mut stack = vec![Work::Enter(root)];
            while let Some(work) = stack.pop() {
                let node = match work {
                    Work::Leave(node) => {
                        states[node] = Visit::Complete;
                        continue;
                    }
                    Work::Enter(node) => node,
                };
                match states[node] {
                    Visit::Complete => continue,
                    Visit::Active => {
                        return Err(invalid("source directory aliases contain a cycle"));
                    }
                    Visit::Unvisited => {}
                }
                states[node] = Visit::Active;
                stack.push(Work::Leave(node));
                for &child in children[node].iter().rev() {
                    stack.push(Work::Enter(child));
                }
            }
        }
        Ok(())
    }
}

pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
