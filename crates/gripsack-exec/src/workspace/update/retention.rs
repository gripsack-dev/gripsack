//! A repository's published lock and an explicit build are independent consumers.
//! Keep the ordinary v2 output-root format and GC reader. Only paths already
//! admitted by an update or retained by this home's previous roots may transfer.
mod segments;

use crate::workspace::{
    conda,
    inputs::InputKind,
    roots::{self, RootId},
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_ir::workspace_model::{
    LockedSource,
    lock::{LockedPin, WorkspaceLock},
};
use segments::{Handoff, Segments};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

fn archives<'a>(ctx: &'a Ctx, pin: &'a LockedPin) -> impl Iterator<Item = PathBuf> + 'a {
    pin.conda.iter().flat_map(move |environment| {
        environment
            .packages
            .iter()
            .map(|record| &record.sha256)
            .chain(environment.wheels.iter().map(|record| &record.sha256))
            .map(move |digest| conda::archive_object(&ctx.home, digest))
    })
}

fn pixi_inputs(pin: &LockedPin) -> impl Iterator<Item = &str> {
    let inputs = match &pin.source {
        LockedSource::PixiLock(source) => [
            source.manifest_sha256.as_deref(),
            source.lock_sha256.as_deref(),
        ],
        _ => [None, None],
    };
    inputs.into_iter().flatten()
}

#[derive(Default)]
pub(super) struct Acquired {
    paths: BTreeSet<PathBuf>,
    input_ids: BTreeSet<String>,
}

impl Acquired {
    pub(super) fn record(&mut self, ctx: &Ctx, pin: &LockedPin, inputs: BTreeSet<PathBuf>) {
        self.paths.extend(inputs);
        self.paths.extend(archives(ctx, pin));
        self.input_ids.extend(pixi_inputs(pin).map(str::to_owned));
    }
}

fn named_object(path: &Path, namespace: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split_once('-'))
        .is_some_and(|(digest, name)| {
            name == namespace
                && digest.len() == gripsack_host::store::paths::HASH_LEN * 2
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

fn lock_object(path: &Path) -> bool {
    named_object(path, "conda-archive") || named_object(path, "workspace-input")
}

pub(in crate::workspace) struct LockRoots {
    segments: Handoff,
    legacy_lock_only: Vec<RootId>,
}

impl LockRoots {
    pub(super) fn plan(
        ctx: &Ctx,
        session: &LifecycleSession,
        mut legacy_names: BTreeSet<String>,
        lock: Option<&WorkspaceLock>,
        mut acquired: Acquired,
    ) -> Result<Option<Self>, ExecError> {
        let segments = Segments::read(ctx, session)?;
        let current = &segments.inventory;
        let home = ctx.home_dir()?;
        let mut legacy_paths = BTreeSet::new();
        let mut legacy_build_evidence = BTreeSet::new();
        let mut legacy_lock_only = Vec::new();
        // A historical failed update could have installed a lock-only root
        // for a new output before adding its first pin to the lockfile.
        for pin in lock
            .into_iter()
            .flat_map(|lock| lock.resolutions.values())
            .flat_map(|r| &r.pins)
        {
            if pin.conda.is_some() && !legacy_names.contains(&pin.output) {
                legacy_names.insert(pin.output.clone());
            }
        }
        for name in legacy_names {
            let legacy = roots::built_output_id(ctx.repository.identity(), &name)?;
            let Some(paths) = roots::output_paths(home, session, &legacy)? else {
                continue;
            };
            if paths.is_empty() {
                continue;
            }
            // Every build retains its workspace-package receipt. Only the exact
            // object classes of the old update producer identify a lock-only
            // record; never retire a built or unfamiliar consumer on update.
            if paths.iter().all(|path| lock_object(path)) {
                legacy_paths.extend(paths);
                legacy_lock_only.push(legacy);
            } else {
                // A build may have replaced the old lock root. Transfer its
                // locked originals, not its payload or package receipt.
                legacy_build_evidence.extend(paths.into_iter().filter(|path| lock_object(path)));
            }
            if legacy_paths.len() > roots::MAX_RETAINED_OBJECTS
                || legacy_build_evidence.len() > roots::MAX_RETAINED_OBJECTS
            {
                return Err(std::io::Error::other(
                    "legacy lock roots exceed inventory object bound",
                )
                .into());
            }
        }
        if segments.is_empty()
            && legacy_lock_only.is_empty()
            && acquired.paths.is_empty()
            && legacy_build_evidence.is_empty()
        {
            return Ok(None);
        }
        let mut unknown_inputs = BTreeSet::new();
        for pin in lock
            .into_iter()
            .flat_map(|lock| lock.resolutions.values())
            .flat_map(|r| &r.pins)
        {
            acquired.paths.extend(archives(ctx, pin).filter(|p| {
                current.contains(p) || legacy_paths.contains(p) || legacy_build_evidence.contains(p)
            }));
            unknown_inputs.extend(pixi_inputs(pin).filter(|id| !acquired.input_ids.contains(*id)));
        }
        // Scoped updates must retain unselected Pixi input objects without
        // capturing their potentially changed declarations. The pin records a
        // domain-separated file identity, not the store's truncated tree hash.
        // Examine only already-retained input objects and only when necessary.
        if !unknown_inputs.is_empty() {
            let candidates = current
                .iter()
                .chain(legacy_paths.iter())
                .chain(legacy_build_evidence.iter());
            for path in candidates.filter(|p| named_object(p, "workspace-input")) {
                if !path.symlink_metadata()?.is_dir() {
                    return Err(std::io::Error::other(
                        "retained workspace input is not a directory",
                    )
                    .into());
                }
                let checked = crate::workspace::stage::validate_output_tree(
                    path,
                    gripsack_ir::workspace::RecipeOutputKind::Tree,
                    "locked Pixi input",
                    ctx.fetch.limits(),
                )?;
                if gripsack_host::store::content_path(
                    &ctx.home,
                    "workspace-input",
                    checked.tree_hash().as_str(),
                ) != *path
                {
                    return Err(std::io::Error::other(
                        "retained workspace input tree disagrees with its store path",
                    )
                    .into());
                }
                if unknown_inputs.remove(
                    InputKind::File
                        .identity(checked.tree_hash())
                        .to_string()
                        .as_str(),
                ) {
                    acquired.paths.insert(path.clone());
                    if unknown_inputs.is_empty() {
                        break;
                    }
                }
            }
        }
        // Transfer lock-only records before their old IDs may disappear. A
        // built consumer remains authoritative independently; only objects
        // actually named by the next lock need another binding.
        legacy_paths.retain(|path| !current.contains(path));
        legacy_paths.extend(
            acquired
                .paths
                .iter()
                .filter(|p| !current.contains(*p))
                .cloned(),
        );
        let segments = segments.handoff(ctx, session, legacy_paths, acquired.paths)?;
        Ok(Some(Self {
            segments,
            legacy_lock_only,
        }))
    }

    pub(super) fn needs_reconciliation(&self) -> bool {
        !self.legacy_lock_only.is_empty() || self.segments.changed()
    }

    /// Under both the lifecycle and repository write locks, durably cover both
    /// outcomes before the lock rename (even a reported error can follow visibility).
    pub(in crate::workspace) fn protect(
        &self,
        session: &LifecycleSession,
    ) -> Result<(), ExecError> {
        self.segments.protect(session)?;
        // Retire migrated producers before the lock can forget their names.
        for id in &self.legacy_lock_only {
            roots::remove_output_root(session, id)?;
        }
        Ok(())
    }

    /// Only durable lock publication authorizes dropping its former closure.
    /// A failed trim leaves conservative evidence at discoverable identities.
    pub(in crate::workspace) fn finish(self, session: &LifecycleSession) -> Result<(), ExecError> {
        self.segments.finish(session)
    }
}
