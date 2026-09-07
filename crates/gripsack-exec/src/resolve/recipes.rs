//! Immutable command projections plus invalidatable dependency recipe keys.
//! Source trees and provenance-free module serialization are computed once.

use crate::{ctx::ExecError, lockfile::Lockfile};
use gripsack_ir::{Ir, prepared::PreparedModule};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

struct Node {
    base: String,
    dependencies: Vec<String>,
}
struct Key {
    input: Arc<str>,
    digest: String,
}

pub(crate) struct RecipeGraph {
    nodes: BTreeMap<String, Node>,
    dependents: BTreeMap<String, Vec<String>>,
    keys: Mutex<BTreeMap<String, Key>>,
}

impl RecipeGraph {
    pub(crate) fn new<'a>(
        ir: &Ir,
        repo: &Path,
        plans: &BTreeMap<String, PreparedModule>,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, ExecError> {
        let mut nodes = BTreeMap::new();
        let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut files = BTreeMap::new();
        for name in names {
            let module = &ir.modules[name];
            let mut base = serde_json::to_string(&super::input::without_spans(module))?;
            for entry in plans[name].entries() {
                let path = repo.join(&entry.from);
                match path.symlink_metadata() {
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                        ) =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                }
                let hash = match files.entry(path) {
                    std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        let hash = gripsack_store::canonical_file_hash(entry.key())?;
                        entry.insert(hash)
                    }
                };
                write!(base, "|{}={hash}", entry.from).expect("string write");
            }
            let dependencies: Vec<String> =
                gripsack_ir::dependencies::ordering_dependencies(name, module)
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
            for dependency in &dependencies {
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(name.to_owned());
            }
            nodes.insert(name.to_owned(), Node { base, dependencies });
        }
        Ok(Self {
            nodes,
            dependents,
            keys: Mutex::new(BTreeMap::new()),
        })
    }

    pub(crate) fn input(&self, name: &str, lock: &Lockfile) -> Arc<str> {
        let mut keys = self.keys.lock().expect("recipe keys");
        self.populate(name, lock, &mut keys);
        Arc::clone(&keys[name].input)
    }

    fn populate(&self, name: &str, lock: &Lockfile, keys: &mut BTreeMap<String, Key>) {
        if keys.contains_key(name) {
            return;
        }
        let node = &self.nodes[name];
        let mut input = node.base.clone();
        for dependency in &node.dependencies {
            if self.nodes.contains_key(dependency) {
                self.populate(dependency, lock, keys);
                write!(input, "|dep:{dependency}={}", keys[dependency].digest)
                    .expect("string write");
            }
            if let Some(pin) = lock
                .modules
                .get(dependency)
                .and_then(|entry| entry.resolved.as_ref())
            {
                write!(
                    input,
                    "|dep-pin:{dependency}={}:{}:{}",
                    pin.sha256.as_deref().unwrap_or("-"),
                    pin.tree256.as_deref().unwrap_or("-"),
                    pin.version.as_deref().unwrap_or("-")
                )
                .expect("string write");
            }
        }
        let digest = gripsack_store::hash::hex_sha256(input.as_bytes());
        keys.insert(
            name.into(),
            Key {
                input: input.into(),
                digest,
            },
        );
    }

    /// The scheduler calls this before releasing consumers of a newly pinned
    /// dependency. No global cache and no stale pin-sensitive descendants.
    pub(crate) fn invalidate(&self, name: &str) {
        let mut keys = self.keys.lock().expect("recipe keys");
        let mut pending = vec![name];
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !visited.insert(name) {
                continue;
            }
            keys.remove(name);
            if let Some(dependents) = self.dependents.get(name) {
                pending.extend(dependents.iter().map(String::as_str));
            }
        }
    }
}
