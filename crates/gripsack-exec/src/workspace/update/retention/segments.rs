//! A lock may cover more objects than one consumer record can encode. Segment
//! IDs are dense, ordered and derived from the repository, never from a path in
//! a lock or a directory entry. A missing ID ends discovery. No GC format or
//! new deletion authority: GC independently admits each ordinary output root.
use crate::workspace::roots::{self, RetentionSet, RootId};
use crate::{Ctx, ExecError, LifecycleSession};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

// The v2 output envelope (version/kind, a 64-hex ID, separators) is <256
// bytes. Count an upper bound on each JSON string plus its comma. The writer
// still checks the exact serialized record against its own bound.
const PATH_BUDGET: usize = roots::MAX_ROOT_BYTES as usize - 256;
fn cost(path: &Path) -> usize {
    3 + path
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|byte| match byte {
            b'"' | b'\\' => 2,
            0..=31 => 6,
            _ => 1,
        })
        .sum::<usize>()
}

fn id(repository: &Path, index: usize) -> Result<RootId, ExecError> {
    Ok(RootId::from_identity(&serde_json::to_vec(&(
        repository,
        "conda-lock",
        index,
    ))?))
}

struct Segment {
    id: RootId,
    paths: BTreeSet<PathBuf>,
    bytes: usize,
    extended: bool,
    // A prior trim can durably copy a binding down before dropping the
    // higher copy. Keep only its lowest occurrence in memory.
    redundant: bool,
}

pub(super) struct Segments {
    rows: Vec<Segment>,
    pub(super) inventory: BTreeSet<PathBuf>,
}
impl Segments {
    pub(super) fn read(ctx: &Ctx, session: &LifecycleSession) -> Result<Self, ExecError> {
        let home = ctx.home_dir()?;
        let mut segments = Vec::new();
        let mut inventory = BTreeSet::new();
        loop {
            let id = id(ctx.repository.identity(), segments.len())?;
            let Some(mut paths) = roots::output_paths(home, session, &id)? else {
                break;
            };
            if segments.len() == roots::MAX_ROOT_ENTRIES {
                return Err(
                    std::io::Error::other("lock roots exceed inventory entry bound").into(),
                );
            }
            let count = paths.len();
            paths.retain(|path| !inventory.contains(path));
            if inventory.len() + paths.len() > roots::MAX_RETAINED_OBJECTS {
                return Err(
                    std::io::Error::other("lock roots exceed inventory object bound").into(),
                );
            }
            let redundant = paths.len() != count;
            inventory.extend(paths.iter().cloned());
            let bytes = paths.iter().map(|path| cost(path)).sum();
            segments.push(Segment {
                id,
                paths,
                bytes,
                extended: false,
                redundant,
            });
        }
        Ok(Self {
            rows: segments,
            inventory,
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(super) fn handoff(
        mut self,
        ctx: &Ctx,
        session: &LifecycleSession,
        added: BTreeSet<PathBuf>,
        mut next: BTreeSet<PathBuf>,
    ) -> Result<Handoff, ExecError> {
        // Before the lock is sealed, only EXTEND the last segment or append at
        // the first absent index. Never overwrite an old-only object here.
        let old_count = self.rows.len();
        drop(self.inventory);
        let grows = !added.is_empty();
        for path in added {
            let bytes = cost(&path);
            if bytes > PATH_BUDGET {
                return Err(std::io::Error::other("store path exceeds root record bound").into());
            }
            if self
                .rows
                .last()
                .is_none_or(|last| last.redundant || last.bytes + bytes > PATH_BUDGET)
            {
                if self.rows.len() == roots::MAX_ROOT_ENTRIES {
                    return Err(
                        std::io::Error::other("lock roots exceed inventory entry bound").into(),
                    );
                }
                self.rows.push(Segment {
                    id: id(ctx.repository.identity(), self.rows.len())?,
                    paths: BTreeSet::new(),
                    bytes: 0,
                    extended: true,
                    redundant: false,
                });
            }
            let last = self
                .rows
                .last_mut()
                .expect("a segment for the admitted object");
            last.paths.insert(path);
            last.bytes += bytes;
            last.extended = true;
        }
        if grows {
            roots::admit_output_growth(
                ctx.home_dir()?,
                session,
                self.rows.len() - old_count,
                self.rows
                    .iter()
                    .filter(|s| s.extended)
                    .flat_map(|s| &s.paths),
            )?;
        }
        let mut before = Vec::with_capacity(self.rows.len());
        let mut after: Vec<BTreeSet<PathBuf>> = Vec::new();
        let mut tail_bytes = 0;
        for segment in self.rows {
            // Move each surviving binding to the same or a LOWER segment, in
            // source order. Never split a source segment across destinations:
            // when destination i is replaced, all of source i's still-needed
            // bindings will already be durable at or below i.
            let mut kept: BTreeSet<_> = segment.paths.iter().filter_map(|p| next.take(p)).collect();
            if !kept.is_empty() {
                let bytes = kept.iter().map(|p| cost(p)).sum::<usize>();
                if let Some(tail) = after.last_mut()
                    && tail_bytes + bytes <= PATH_BUDGET
                {
                    tail.append(&mut kept);
                    tail_bytes += bytes;
                } else {
                    after.push(kept);
                    tail_bytes = bytes;
                }
            }
            before.push(Protected {
                id: segment.id,
                paths: RetentionSet::admit(session, segment.paths)?,
                extended: segment.extended,
                redundant: segment.redundant,
            });
        }
        if !next.is_empty() {
            return Err(std::io::Error::other("handoff omitted an admitted next binding").into());
        }
        let after = after
            .into_iter()
            .map(|p| RetentionSet::admit(session, p))
            .collect::<Result<_, _>>()?;
        Ok(Handoff { before, after })
    }
}

struct Protected {
    id: RootId,
    paths: RetentionSet,
    extended: bool,
    redundant: bool,
}

pub(super) struct Handoff {
    before: Vec<Protected>,
    after: Vec<RetentionSet>,
}
impl Handoff {
    pub(super) fn changed(&self) -> bool {
        self.before.len() != self.after.len()
            || self.before.iter().zip(&self.after).any(|(before, after)| {
                before.extended || before.redundant || before.paths.paths() != after.paths()
            })
    }

    pub(super) fn protect(&self, session: &LifecycleSession) -> Result<(), ExecError> {
        if !self.before.iter().any(|s| s.extended) {
            roots::seal_output_roots(session)?;
        }
        for segment in &self.before {
            if segment.extended {
                roots::register_output_root(session, &segment.id, &segment.paths)?;
            }
        }
        Ok(())
    }

    pub(super) fn finish(self, session: &LifecycleSession) -> Result<(), ExecError> {
        for (before, after) in self.before.iter().zip(&self.after) {
            if before.redundant || before.paths.paths() != after.paths() {
                roots::register_output_root(session, &before.id, after)?;
            }
        }
        // Each remove seals the output-root directory before proceeding. A
        // crash never strands a live upper segment behind a newly absent ID.
        for segment in self.before[self.after.len()..].iter().rev() {
            roots::remove_output_root(session, &segment.id)?;
        }
        Ok(())
    }
}
