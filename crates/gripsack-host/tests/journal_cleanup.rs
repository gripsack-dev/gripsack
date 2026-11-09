//! F2 step 1 exit (plan/0056 §9): `MemFs` must rediscover the 0.19.1/0.19.2
//! "resurrection" when the journal cleanup's entry barrier is removed, and
//! the shipped two-barrier order must survive every crash.
//!
//! The bug (CHANGELOG 0.19.2, plan/0020 amendment, `journal/marker.rs`
//! `cleanup`): after a committed run, cleanup deletes the per-destination
//! entries and the run marker. Recovery reads "entries without a marker" as
//! an uncommitted legacy run and restores the entries' priors. If a power
//! loss keeps the marker's unlink but drops an entry's, the committed
//! generation's content is overwritten with the previous generation's.
//! A kill cannot produce that state (the page cache survives in order), so
//! only S3's power-loss subsets find it.
//!
//! The run and recovery below are the shipped protocol at storage-op level
//! (`begin_run` → `Journal::write` → leaf mutation → flip → `commit_run`;
//! `reconcile`), cut down to one destination and no directory records.

mod support;

use gripsack_host::storage::{
    Boundary, Crash, CrashKinds, Exploration, ExploreError, MemDir, MemFs, PendingBound,
    Permissions, Storage,
};
use std::io;
use support::{WriteOrder, atomic_write, name, symlink_replace};

const PRIOR: &[u8] = b"generation 1 bytes";
const INTENDED: &[u8] = b"generation 2 bytes";
const PREVIOUS: &str = "generations/1";
const TARGET: &str = "generations/2";
const RUN_MARKER: &str = "run.json";
const ENTRY: &str = "dest.json";
const DEST_MODE: Permissions = Permissions::new(0o644);

/// `marker::cleanup`'s barrier placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cleanup {
    /// Shipped (0026 §5): unlink entries, fsync; unlink marker, fsync.
    TwoBarriers,
    /// 0.19.2: one trailing fsync — the entry barrier removed.
    TrailingBarrier,
    /// 0.19.1: no fsync at all.
    NoBarrier,
}

/// `$GRIPSACK_HOME` after generation 1 deployed, all durable.
fn home() -> MemFs {
    let fs = MemFs::new();
    let root = fs.root().unwrap();
    for at in ["generations", "journal"] {
        fs.create_dir(&root, &name(at), Permissions::new(0o700))
            .unwrap();
    }
    let generations = fs.open_dir(&root, &name("generations")).unwrap();
    for at in ["1", "2"] {
        fs.create_dir(&generations, &name(at), Permissions::new(0o755))
            .unwrap();
    }
    atomic_write(&fs, &root, "dest", PRIOR, DEST_MODE, WriteOrder::Shipped).unwrap();
    fs.symlink(&root, &name("current"), PREVIOUS.as_ref())
        .unwrap();
    fs.settled()
}

fn journal(fs: &MemFs) -> io::Result<MemDir> {
    fs.open_dir(&fs.root()?, &name("journal"))
}

/// `Journal::write`: the staged-file publication with mode 0600.
fn publish_record(fs: &MemFs, journal: &MemDir, at: &str, bytes: &[u8]) -> io::Result<()> {
    atomic_write(
        fs,
        journal,
        at,
        bytes,
        Permissions::new(0o600),
        WriteOrder::Shipped,
    )
}

/// Entry and marker wire, reduced to what classification and restore read.
fn encode_entry() -> Vec<u8> {
    [b"dest\n".as_slice(), PRIOR, b"\n", INTENDED].concat()
}

struct Entry {
    dest: String,
    prior: Vec<u8>,
    intended: Vec<u8>,
}

fn decode_entry(bytes: &[u8]) -> io::Result<Entry> {
    let fields: Vec<&[u8]> = bytes.split(|&b| b == b'\n').collect();
    match fields.as_slice() {
        [dest, prior, intended] => Ok(Entry {
            dest: String::from_utf8_lossy(dest).into_owned(),
            prior: prior.to_vec(),
            intended: intended.to_vec(),
        }),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "torn journal entry",
        )),
    }
}

fn decode_marker(bytes: &[u8]) -> io::Result<(String, String)> {
    let text = String::from_utf8_lossy(bytes);
    match text.split_once('\n') {
        Some((previous, target)) => Ok((previous.to_owned(), target.to_owned())),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "torn run marker",
        )),
    }
}

/// `marker::cleanup` with the barrier placement under test.
fn cleanup(fs: &MemFs, journal: &MemDir, entries: &[String], order: Cleanup) -> io::Result<()> {
    for entry in entries {
        fs.unlink(journal, &name(entry))?;
    }
    if order == Cleanup::TwoBarriers {
        fs.sync_dir(journal)?;
    }
    match fs.unlink(journal, &name(RUN_MARKER)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    if order != Cleanup::NoBarrier {
        fs.sync_dir(journal)?;
    }
    Ok(())
}

fn entry_names(fs: &MemFs, journal: &MemDir) -> io::Result<Vec<String>> {
    Ok(fs
        .read_dir(journal)?
        .into_iter()
        .map(|entry| entry.as_os_str().to_string_lossy().into_owned())
        .filter(|entry| entry != RUN_MARKER && !entry.starts_with('.') && entry.ends_with(".json"))
        .collect())
}

/// One committed apply: declare, record, mutate, flip, clean up.
fn apply(fs: &MemFs, order: Cleanup) -> io::Result<()> {
    let root = fs.root()?;
    let journal = journal(fs)?;
    // begin_run: the marker is durable before any mutation
    publish_record(
        fs,
        &journal,
        RUN_MARKER,
        format!("{PREVIOUS}\n{TARGET}").as_bytes(),
    )?;
    // record the prior, durably, then mutate the destination
    publish_record(fs, &journal, ENTRY, &encode_entry())?;
    atomic_write(fs, &root, "dest", INTENDED, DEST_MODE, WriteOrder::Shipped)?;
    // the flip: the commit point
    symlink_replace(fs, &root, "current", TARGET)?;
    // commit_run
    let entries = entry_names(fs, &journal)?;
    cleanup(fs, &journal, &entries, order)
}

/// `reconcile`: classify by the marker against `current`, then clean up
/// (committed) or restore priors under the drift guard (uncommitted, or
/// entries with no marker — the legacy shape the bug produces).
fn recover(fs: &MemFs, order: Cleanup) -> io::Result<()> {
    let root = fs.root()?;
    let journal = journal(fs)?;
    for straggler in fs.read_dir(&journal)? {
        if straggler
            .as_os_str()
            .to_string_lossy()
            .starts_with(".tmp-write-")
        {
            fs.unlink(&journal, &straggler)?;
            fs.sync_dir(&journal)?;
        }
    }
    let marker = match fs.read(&journal, &name(RUN_MARKER)) {
        Ok(bytes) => Some(decode_marker(&bytes)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let entries = entry_names(fs, &journal)?;
    if marker.is_none() && entries.is_empty() {
        return Ok(());
    }
    let current = fs.read_link(&root, &name("current"))?;
    let committed = match &marker {
        Some((_, target)) if current.as_os_str() == target.as_str() => true,
        Some((previous, _)) if current.as_os_str() == previous.as_str() => false,
        Some(_) => return Err(io::Error::other("ambiguous journal selections")),
        None => false,
    };
    if committed {
        // seal the observed flip before the evidence goes
        fs.sync_dir(&root)?;
        return cleanup(fs, &journal, &entries, order);
    }
    for at in &entries {
        let entry = decode_entry(&fs.read(&journal, &name(at))?)?;
        let live = match fs.read(&root, &name(&entry.dest)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if live.as_deref() == Some(entry.intended.as_slice()) {
            atomic_write(
                fs,
                &root,
                &entry.dest,
                &entry.prior,
                DEST_MODE,
                WriteOrder::Shipped,
            )?;
        } else if live.as_deref() == Some(entry.prior.as_slice()) {
            // seal_observed_prior
            let file = fs.open_file(&root, &name(&entry.dest))?;
            fs.sync_file(&file)?;
            fs.sync_dir(&root)?;
        }
        // anything else drifted after the crash: the user's now, kept
    }
    cleanup(fs, &journal, &entries, order)
}

/// P2 in the small: after recovery the journal is empty, `current` names a
/// published generation, and the destination holds exactly that
/// generation's bytes.
fn recovered_state_is_consistent(fs: &MemFs) -> Result<(), String> {
    let root = fs.root().map_err(|e| e.to_string())?;
    let journal = journal(fs).map_err(|e| e.to_string())?;
    let left = fs.read_dir(&journal).map_err(|e| e.to_string())?;
    if !left.is_empty() {
        return Err(format!("journal not cleared: {left:?}"));
    }
    let current = fs
        .read_link(&root, &name("current"))
        .map_err(|e| format!("current unreadable: {e}"))?;
    let dest = fs
        .read(&root, &name("dest"))
        .map_err(|e| format!("dest unreadable: {e}"))?;
    let expected = match current.to_str() {
        Some(PREVIOUS) => PRIOR,
        Some(TARGET) => INTENDED,
        _ => return Err(format!("current names {current:?}")),
    };
    if dest == expected {
        Ok(())
    } else if current.as_os_str() == TARGET && dest == PRIOR {
        Err(
            "RESURRECTION: current selects generation 2 but recovery restored \
             generation 1's prior into dest"
                .into(),
        )
    } else {
        Err(format!(
            "dest holds {:?} under {current:?}",
            String::from_utf8_lossy(&dest)
        ))
    }
}

/// Recover to completion on a crashed image, then check.
fn recovers(fs: MemFs, order: Cleanup) -> Result<(), String> {
    recover(&fs, order).map_err(|e| format!("recovery failed: {e}"))?;
    recovered_state_is_consistent(&fs)
}

fn explore_apply(
    order: Cleanup,
    kinds: CrashKinds,
    crash_recovery_too: bool,
) -> Result<Exploration, ExploreError> {
    let mut nested = Exploration::default();
    let result = gripsack_host::storage::explore(
        &home(),
        kinds,
        PendingBound::default(),
        |fs| apply(fs, order),
        |image, _| {
            if !crash_recovery_too {
                return recovers(image, order);
            }
            // crash the first recovery anywhere too, then recover again
            let inner = gripsack_host::storage::explore(
                &image,
                kinds,
                PendingBound::default(),
                |fs| recover(fs, order),
                |again, _| recovers(again, order),
            )
            .map_err(|error| format!("during recovery: {error}"))?;
            nested.cuts += inner.cuts;
            nested.images += inner.images;
            Ok(())
        },
    );
    result.map(|outer| Exploration {
        cuts: outer.cuts + nested.cuts,
        images: outer.images + nested.images,
    })
}

#[test]
fn shipped_two_barrier_cleanup_survives_every_crash_and_every_recovery_crash() {
    let explored = explore_apply(Cleanup::TwoBarriers, CrashKinds::Both, true)
        .unwrap_or_else(|error| panic!("{error}"));
    eprintln!("two-barrier cleanup: {explored:?}");
    assert!(explored.cuts > 20 && explored.images > explored.cuts);
}

#[test]
fn removing_the_entry_barrier_resurrects_priors_after_power_loss() {
    // A kill never finds it: the page cache keeps the unlinks in order.
    explore_apply(Cleanup::TrailingBarrier, CrashKinds::Kill, true)
        .unwrap_or_else(|error| panic!("kill alone must not resurrect: {error}"));

    let error = explore_apply(Cleanup::TrailingBarrier, CrashKinds::PowerLoss, false)
        .expect_err("MemFs must find the 0.19.2 resurrection");
    let violation = error.violation().expect("a resurrection, not a run error");
    eprintln!("{violation}");
    assert!(violation.failure.starts_with("RESURRECTION"));
    let Crash::PowerLoss { lost } = &violation.point.crash else {
        panic!("expected a power loss");
    };
    // the witness: the entry's unlink is lost, the marker's survives
    let unlinks: Vec<(usize, String)> = violation.trace[violation.first..]
        .iter()
        .enumerate()
        .filter(|(_, event)| event.boundary == Boundary::Unlink)
        .filter_map(|(i, event)| {
            let at = event.name.as_ref()?.as_os_str().to_string_lossy();
            (at == ENTRY || at == RUN_MARKER).then(|| (violation.first + i, at.into_owned()))
        })
        .collect();
    let entry = unlinks.iter().find(|(_, at)| at == ENTRY).unwrap().0;
    let marker = unlinks.iter().find(|(_, at)| at == RUN_MARKER).unwrap().0;
    assert!(lost.contains(&entry), "entry unlink lost");
    assert!(!lost.contains(&marker), "marker unlink survived");
}

#[test]
fn the_0_19_1_cleanup_without_barriers_resurrects_after_returning() {
    let error = explore_apply(Cleanup::NoBarrier, CrashKinds::PowerLoss, false)
        .expect_err("MemFs must find the 0.19.1 resurrection");
    let violation = error.violation().expect("a resurrection");
    assert!(violation.failure.starts_with("RESURRECTION"), "{violation}");
}
