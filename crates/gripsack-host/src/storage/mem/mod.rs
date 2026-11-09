//! `MemFs`: an in-memory [`Storage`] that mirrors the storage model and
//! injects both crash kinds (plan/0056 §4, S1–S3).
//!
//! The model:
//!
//! - an inode table, held twice: the **volatile** image every operation
//!   sees (the page cache) and the **durable** image (the disk);
//! - every effect stages idempotent [`Update`]s: applied to the volatile
//!   image at once, kept *pending* for the durable one;
//! - S2 barriers move pending updates to the durable image: `sync_file`
//!   those touching the file's bytes and mode, `sync_dir` the directory's
//!   entries. Nothing else does (no implicit ordering, no `data=ordered`);
//! - S1: a rename within a directory is one update, so it is atomic and
//!   replaces; across directories it is two independent entry updates, one
//!   per directory barrier. Exclusive create/mkdir/`RENAME_NOREPLACE`
//!   check the volatile image;
//! - S3 **kill**: the process dies, the page cache survives —
//!   [`MemFs::kill`] keeps both images and the pending updates; later
//!   processes observe non-durable state, and may still lose it;
//! - S3 **power loss**: [`MemFs::power_loss`] keeps an *arbitrary subset*
//!   of the pending updates, applied in trace order over the durable image.
//!   No prefix closure: a later update may survive an earlier one's loss.
//!   Barriers already completed are respected because they emptied the
//!   pending set of what they covered.
//!
//! Inode creation itself (with its initial mode, or a symlink's target) is
//! durable at once: an inode is unreachable until an entry names it, and
//! that entry is what needs the barrier.
//!
//! Every effect is recorded as a [`TraceEvent`] in the shared fault-boundary
//! vocabulary; [`explore`] enumerates every crash cut and loss subset.
//!
//! S5 (external writers): every call, observation or effect, is numbered as
//! an [`Operation`]. [`MemFs::inject`] arms one [`ExternalWrite`] — another
//! process replacing a file — to land just before a chosen operation runs,
//! so a test can place it between a driver's re-check and its act, or
//! anywhere else, and state what the driver may and may not overwrite.
mod explore;
mod image;

pub use explore::{
    Crash, CrashKinds, CrashPoint, Exploration, ExploreError, PendingBound, Violation, explore,
};

use super::{Boundary, Metadata, Name, ObjectId, ObjectKind, Permissions, Storage};
use image::{Barrier, Image, Ino, Node, ROOT, Update};
use rustix::io::Errno;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// The filesystem id every `MemFs` object reports in its [`ObjectId`].
const MEM_FILESYSTEM: u64 = 0x6d656d6673;

/// One effect, in the shared fault-boundary vocabulary. `object` is the
/// directory for namespace effects and the file for data/mode/sync effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEvent {
    pub boundary: Boundary,
    pub object: ObjectId,
    pub name: Option<Name>,
}

impl fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} ino {}", self.boundary, self.object.inode)?;
        if let Some(name) = &self.name {
            write!(f, " {name:?}")?;
        }
        Ok(())
    }
}

/// The error every operation returns once the process has crashed.
#[derive(Debug)]
struct Crashed;

impl fmt::Display for Crashed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("injected crash: the process is gone")
    }
}

impl std::error::Error for Crashed {}

#[derive(Debug, Clone)]
struct Pending {
    /// Trace index of the effect that staged it.
    effect: usize,
    update: Update,
}

/// One call into the storage, in order: an observation (`boundary: None`)
/// or an effect. `name` is the directory entry it resolved, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub boundary: Option<Boundary>,
    pub name: Option<Name>,
}

/// Another process (the user's editor) replacing `dir/name` with a new
/// regular file. It lands durably at once: S5 writers are only *detected*,
/// never crash-explored here.
#[derive(Debug, Clone)]
pub struct ExternalWrite {
    pub dir: MemDir,
    pub name: Name,
    pub bytes: Vec<u8>,
    pub permissions: Permissions,
}

#[derive(Debug, Clone, Default)]
struct Faults {
    /// Crash when this many effects have completed (absolute trace length).
    crash_at: Option<usize>,
    /// Fail the effect at this trace index once, without crashing.
    fail_at: Option<(usize, Errno)>,
    crashed: bool,
}

#[derive(Debug, Clone)]
struct State {
    volatile: Image,
    durable: Image,
    pending: Vec<Pending>,
    next_ino: Ino,
    trace: Vec<TraceEvent>,
    faults: Faults,
    operations: Vec<Operation>,
    /// The external write armed for operation `.0`.
    injection: Option<(usize, ExternalWrite)>,
}

impl State {
    /// Number and log one call; land an armed external write first.
    fn operation(&mut self, boundary: Option<Boundary>, name: Option<&Name>) {
        if let Some((at, _)) = &self.injection
            && *at == self.operations.len()
        {
            let (_, write) = self.injection.take().expect("armed");
            self.land(write);
        }
        self.operations.push(Operation {
            boundary,
            name: name.cloned(),
        });
    }

    fn land(&mut self, write: ExternalWrite) {
        let ino = self.allocate(Node::File {
            data: write.bytes,
            permissions: write.permissions,
        });
        for image in [&mut self.volatile, &mut self.durable] {
            Update::Entry {
                dir: write.dir.ino,
                name: write.name.clone(),
                target: Some(ino),
            }
            .apply(image);
        }
    }

    fn object(ino: Ino) -> ObjectId {
        ObjectId {
            filesystem: MEM_FILESYSTEM,
            inode: ino,
        }
    }

    fn stage(&mut self, update: Update) {
        update.apply(&mut self.volatile);
        self.pending.push(Pending {
            effect: self.trace.len(),
            update,
        });
    }

    fn barrier(&mut self, barrier: Barrier) {
        let (covered, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|pending| pending.update.covered_by(barrier));
        for pending in covered {
            pending.update.apply(&mut self.durable);
        }
        self.pending = rest;
    }

    /// A fresh inode exists in both images; only entries naming it are
    /// pending.
    fn allocate(&mut self, node: Node) -> Ino {
        let ino = self.next_ino;
        self.next_ino += 1;
        self.durable.nodes.insert(ino, node.clone());
        self.volatile.nodes.insert(ino, node);
        ino
    }

    fn lookup(&self, dir: Ino, name: &Name) -> io::Result<Ino> {
        self.volatile
            .lookup(dir, name)
            .ok_or_else(|| Errno::NOENT.into())
    }

    fn node(&self, ino: Ino) -> &Node {
        self.volatile
            .node(ino)
            .expect("handles and entries name allocated inodes")
    }

    /// A directory that can still gain entries.
    fn live_dir(&self, dir: Ino) -> io::Result<()> {
        match self.node(dir) {
            Node::Dir { removed: false, .. } => Ok(()),
            Node::Dir { removed: true, .. } => Err(Errno::NOENT.into()),
            _ => Err(Errno::NOTDIR.into()),
        }
    }

    fn vacant(&self, dir: Ino, name: &Name) -> io::Result<()> {
        self.live_dir(dir)?;
        match self.volatile.lookup(dir, name) {
            Some(_) => Err(Errno::EXIST.into()),
            None => Ok(()),
        }
    }

    fn mark_removed(&mut self, ino: Ino) {
        if let Some(Node::Dir { removed, .. }) = self.volatile.nodes.get_mut(&ino) {
            *removed = true;
        }
    }

    /// Whether `ino` is `target` or contains it (rename into itself).
    fn contains(&self, ino: Ino, target: Ino) -> bool {
        if ino == target {
            return true;
        }
        self.volatile
            .entries(ino)
            .is_some_and(|entries| entries.values().any(|&child| self.contains(child, target)))
    }

    fn rename(
        &mut self,
        (from_dir, from): (Ino, &Name),
        (to_dir, to): (Ino, &Name),
        replace: bool,
    ) -> io::Result<()> {
        let source = self.lookup(from_dir, from)?;
        self.live_dir(to_dir)?;
        let source_is_dir = matches!(self.node(source), Node::Dir { .. });
        let replaced = self.volatile.lookup(to_dir, to);
        if let Some(existing) = replaced {
            if !replace {
                return Err(Errno::EXIST.into());
            }
            if existing == source {
                return Ok(()); // POSIX: renaming a name onto its own inode does nothing
            }
            match (source_is_dir, self.node(existing)) {
                (true, Node::Dir { entries, .. }) if !entries.is_empty() => {
                    return Err(Errno::NOTEMPTY.into());
                }
                (true, Node::Dir { .. }) | (false, Node::File { .. } | Node::Symlink { .. }) => {}
                (true, _) => return Err(Errno::NOTDIR.into()),
                (false, _) => return Err(Errno::ISDIR.into()),
            }
        }
        if source_is_dir && self.contains(source, to_dir) {
            return Err(Errno::INVAL.into());
        }
        if from_dir == to_dir {
            self.stage(Update::Rename {
                dir: from_dir,
                from: from.clone(),
                to: to.clone(),
                ino: source,
            });
        } else {
            self.stage(Update::Entry {
                dir: to_dir,
                name: to.clone(),
                target: Some(source),
            });
            self.stage(Update::Entry {
                dir: from_dir,
                name: from.clone(),
                target: None,
            });
        }
        if let Some(existing) = replaced {
            self.mark_removed(existing);
        }
        Ok(())
    }
}

/// A crash-injecting in-memory filesystem. See the module docs for the
/// model. Each value is one independent filesystem; [`MemFs::snapshot`],
/// [`MemFs::kill`] and [`MemFs::power_loss`] derive new ones.
pub struct MemFs {
    state: Mutex<State>,
}

#[derive(Debug, Clone, Copy)]
pub struct MemDir {
    ino: Ino,
}

#[derive(Debug)]
pub struct MemFile {
    ino: Ino,
    offset: u64,
    writable: bool,
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MemFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.lock();
        f.debug_struct("MemFs")
            .field("effects", &state.trace.len())
            .field("pending", &state.pending.len())
            .field("crashed", &state.faults.crashed)
            .finish()
    }
}

impl MemFs {
    /// An empty, fully durable filesystem: one root directory.
    pub fn new() -> Self {
        Self::from_state(State {
            volatile: Image::empty(),
            durable: Image::empty(),
            pending: Vec::new(),
            next_ino: ROOT + 1,
            trace: Vec::new(),
            faults: Faults::default(),
            operations: Vec::new(),
            injection: None,
        })
    }

    fn from_state(state: State) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Whether `error` is the injected crash.
    pub fn is_crash(error: &io::Error) -> bool {
        error.get_ref().is_some_and(|inner| inner.is::<Crashed>())
    }

    /// An independent copy of this filesystem, faults disarmed.
    pub fn snapshot(&self) -> MemFs {
        let mut state = self.lock().clone();
        state.faults = Faults::default();
        Self::from_state(state)
    }

    /// An independent copy with every pending update durable and an empty
    /// trace: the starting point a test builds once and explores from.
    pub fn settled(&self) -> MemFs {
        let mut state = self.lock().clone();
        for pending in std::mem::take(&mut state.pending) {
            pending.update.apply(&mut state.durable);
        }
        state.trace.clear();
        state.faults = Faults::default();
        Self::from_state(state)
    }

    /// S3 kill: the process dies, the page cache survives. The result is
    /// the filesystem the next process sees: volatile state visible, the
    /// same updates still pending (a later power loss can still drop them).
    /// The trace continues, so effect indices stay unique across processes.
    pub fn kill(&self) -> MemFs {
        self.snapshot()
    }

    /// Number of pending updates a power loss chooses among.
    pub fn pending_updates(&self) -> usize {
        self.lock().pending.len()
    }

    /// S3 power loss: pending update `i` survives iff `keep(i)`. Survivors
    /// apply in trace order over the durable image; nothing is pending
    /// afterwards. Returns the filesystem and the trace indices of the
    /// effects that lost at least one update.
    pub fn power_loss(&self, keep: impl Fn(usize) -> bool) -> (MemFs, Vec<usize>) {
        let state = self.lock();
        let mut image = state.durable.clone();
        let mut lost = Vec::new();
        for (index, pending) in state.pending.iter().enumerate() {
            if keep(index) {
                pending.update.apply(&mut image);
            } else if lost.last() != Some(&pending.effect) {
                lost.push(pending.effect);
            }
        }
        let next = State {
            volatile: image.clone(),
            durable: image,
            pending: Vec::new(),
            next_ino: state.next_ino,
            trace: state.trace.clone(),
            faults: Faults::default(),
            operations: state.operations.clone(),
            injection: None,
        };
        (Self::from_state(next), lost)
    }

    /// Effects completed so far (the trace length).
    pub fn effects(&self) -> usize {
        self.lock().trace.len()
    }

    pub fn trace(&self) -> Vec<TraceEvent> {
        self.lock().trace.clone()
    }

    /// Crash once `effects` effects have completed: that attempt and every
    /// later operation, reads included, return the crash error.
    pub fn crash_at(&self, effects: usize) {
        self.lock().faults.crash_at = Some(effects);
    }

    /// Fail the effect at trace index `effect` once with `errno`, without
    /// performing it and without crashing (a refused syscall).
    pub fn fail_at(&self, effect: usize, errno: Errno) {
        self.lock().faults.fail_at = Some((effect, errno));
    }

    pub fn crashed(&self) -> bool {
        self.lock().faults.crashed
    }

    /// S5: land `write` immediately before operation number `at` (counted
    /// over this filesystem's whole history, see [`MemFs::operations`]).
    pub fn inject(&self, at: usize, write: ExternalWrite) {
        self.lock().injection = Some((at, write));
    }

    /// Whether an armed external write has not landed yet.
    pub fn injection_pending(&self) -> bool {
        self.lock().injection.is_some()
    }

    /// Every call so far, observations included.
    pub fn operations(&self) -> Vec<Operation> {
        self.lock().operations.clone()
    }

    fn effect<T>(
        &self,
        boundary: Boundary,
        object: Ino,
        name: Option<&Name>,
        perform: impl FnOnce(&mut State) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut state = self.lock();
        let index = state.trace.len();
        if state.faults.crashed || state.faults.crash_at == Some(index) {
            state.faults.crashed = true;
            return Err(io::Error::other(Crashed));
        }
        state.operation(Some(boundary), name);
        if let Some((at, errno)) = state.faults.fail_at
            && at == index
        {
            state.faults.fail_at = None;
            return Err(errno.into());
        }
        let value = perform(&mut state)?;
        state.trace.push(TraceEvent {
            boundary,
            object: State::object(object),
            name: name.cloned(),
        });
        Ok(value)
    }

    fn observe<T>(
        &self,
        name: Option<&Name>,
        observe: impl FnOnce(&State) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut state = self.lock();
        if state.faults.crashed {
            return Err(io::Error::other(Crashed));
        }
        state.operation(None, name);
        observe(&state)
    }
}

impl Storage for MemFs {
    type Dir = MemDir;
    type File = MemFile;

    fn root(&self) -> io::Result<MemDir> {
        self.observe(None, |_| Ok(MemDir { ino: ROOT }))
    }

    fn open_dir(&self, parent: &MemDir, name: &Name) -> io::Result<MemDir> {
        self.observe(Some(name), |state| {
            let ino = state.lookup(parent.ino, name)?;
            match state.node(ino) {
                Node::Dir { .. } => Ok(MemDir { ino }),
                Node::Symlink { .. } => Err(Errno::LOOP.into()),
                Node::File { .. } => Err(Errno::NOTDIR.into()),
            }
        })
    }

    fn create_dir(&self, parent: &MemDir, name: &Name, permissions: Permissions) -> io::Result<()> {
        self.effect(Boundary::Mkdir, parent.ino, Some(name), |state| {
            state.vacant(parent.ino, name)?;
            let ino = state.allocate(Node::Dir {
                entries: Default::default(),
                permissions,
                removed: false,
            });
            state.stage(Update::Entry {
                dir: parent.ino,
                name: name.clone(),
                target: Some(ino),
            });
            Ok(())
        })
    }

    fn remove_dir(&self, parent: &MemDir, name: &Name) -> io::Result<()> {
        self.effect(Boundary::Unlink, parent.ino, Some(name), |state| {
            let ino = state.lookup(parent.ino, name)?;
            match state.node(ino) {
                Node::Dir { entries, .. } if !entries.is_empty() => {
                    return Err(Errno::NOTEMPTY.into());
                }
                Node::Dir { .. } => {}
                _ => return Err(Errno::NOTDIR.into()),
            }
            state.mark_removed(ino);
            state.stage(Update::Entry {
                dir: parent.ino,
                name: name.clone(),
                target: None,
            });
            Ok(())
        })
    }

    fn create_file(
        &self,
        parent: &MemDir,
        name: &Name,
        permissions: Permissions,
    ) -> io::Result<MemFile> {
        self.effect(Boundary::Write, parent.ino, Some(name), |state| {
            state.vacant(parent.ino, name)?;
            let ino = state.allocate(Node::File {
                data: Vec::new(),
                permissions,
            });
            state.stage(Update::Entry {
                dir: parent.ino,
                name: name.clone(),
                target: Some(ino),
            });
            Ok(MemFile {
                ino,
                offset: 0,
                writable: true,
            })
        })
    }

    fn open_file(&self, parent: &MemDir, name: &Name) -> io::Result<MemFile> {
        self.observe(Some(name), |state| {
            let ino = state.lookup(parent.ino, name)?;
            match state.node(ino) {
                Node::File { .. } => Ok(MemFile {
                    ino,
                    offset: 0,
                    writable: false,
                }),
                Node::Dir { .. } => Err(Errno::ISDIR.into()),
                Node::Symlink { .. } => Err(Errno::LOOP.into()),
            }
        })
    }

    fn write(&self, file: &mut MemFile, bytes: &[u8]) -> io::Result<()> {
        let offset = file.offset;
        self.effect(Boundary::Write, file.ino, None, |state| {
            if !file.writable {
                return Err(Errno::BADF.into());
            }
            state.stage(Update::Data {
                ino: file.ino,
                offset,
                bytes: bytes.to_vec(),
            });
            Ok(())
        })?;
        file.offset += bytes.len() as u64;
        Ok(())
    }

    fn set_mode(&self, file: &MemFile, permissions: Permissions) -> io::Result<()> {
        self.effect(Boundary::Mode, file.ino, None, |state| {
            state.stage(Update::Mode {
                ino: file.ino,
                permissions,
            });
            Ok(())
        })
    }

    fn read_file(&self, file: &MemFile) -> io::Result<Vec<u8>> {
        self.observe(None, |state| match state.node(file.ino) {
            Node::File { data, .. } => Ok(data.clone()),
            _ => Err(Errno::ISDIR.into()),
        })
    }

    fn sync_file(&self, file: &MemFile) -> io::Result<()> {
        self.effect(Boundary::FileSync, file.ino, None, |state| {
            state.barrier(Barrier::File(file.ino));
            Ok(())
        })
    }

    fn sync_dir(&self, dir: &MemDir) -> io::Result<()> {
        self.effect(Boundary::DirSync, dir.ino, None, |state| {
            state.barrier(Barrier::Dir(dir.ino));
            Ok(())
        })
    }

    fn rename(&self, from_dir: &MemDir, from: &Name, to_dir: &MemDir, to: &Name) -> io::Result<()> {
        self.effect(Boundary::FilePublish, to_dir.ino, Some(to), |state| {
            state.rename((from_dir.ino, from), (to_dir.ino, to), true)
        })
    }

    fn rename_noreplace(
        &self,
        from_dir: &MemDir,
        from: &Name,
        to_dir: &MemDir,
        to: &Name,
    ) -> io::Result<()> {
        self.effect(Boundary::TreePublish, to_dir.ino, Some(to), |state| {
            state.rename((from_dir.ino, from), (to_dir.ino, to), false)
        })
    }

    fn unlink(&self, parent: &MemDir, name: &Name) -> io::Result<()> {
        self.effect(Boundary::Unlink, parent.ino, Some(name), |state| {
            let ino = state.lookup(parent.ino, name)?;
            if matches!(state.node(ino), Node::Dir { .. }) {
                return Err(Errno::ISDIR.into());
            }
            state.stage(Update::Entry {
                dir: parent.ino,
                name: name.clone(),
                target: None,
            });
            Ok(())
        })
    }

    fn symlink(&self, parent: &MemDir, name: &Name, target: &Path) -> io::Result<()> {
        self.effect(Boundary::Symlink, parent.ino, Some(name), |state| {
            state.vacant(parent.ino, name)?;
            let ino = state.allocate(Node::Symlink {
                target: target.to_owned(),
            });
            state.stage(Update::Entry {
                dir: parent.ino,
                name: name.clone(),
                target: Some(ino),
            });
            Ok(())
        })
    }

    fn read_link(&self, parent: &MemDir, name: &Name) -> io::Result<PathBuf> {
        self.observe(Some(name), |state| {
            match state.node(state.lookup(parent.ino, name)?) {
                Node::Symlink { target } => Ok(target.clone()),
                _ => Err(Errno::INVAL.into()),
            }
        })
    }

    fn metadata(&self, parent: &MemDir, name: &Name) -> io::Result<Metadata> {
        self.observe(Some(name), |state| {
            let ino = state.lookup(parent.ino, name)?;
            let (kind, len, permissions) = match state.node(ino) {
                Node::File { data, permissions } => {
                    (ObjectKind::File, data.len() as u64, *permissions)
                }
                Node::Dir { permissions, .. } => (ObjectKind::Dir, 0, *permissions),
                Node::Symlink { target } => (
                    ObjectKind::Symlink,
                    target.as_os_str().len() as u64,
                    Permissions::new(0o777),
                ),
            };
            Ok(Metadata {
                kind,
                len,
                permissions,
                object: State::object(ino),
            })
        })
    }

    fn read_dir(&self, dir: &MemDir) -> io::Result<Vec<Name>> {
        self.observe(None, |state| match state.volatile.entries(dir.ino) {
            Some(entries) => Ok(entries.keys().cloned().collect()),
            None => Err(Errno::NOTDIR.into()),
        })
    }
}

#[cfg(test)]
mod tests;
