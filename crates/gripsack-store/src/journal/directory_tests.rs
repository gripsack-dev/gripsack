//! Concrete hierarchy refinement through real journal records and filesystem
//! mutations. Each cut leaves the same durable evidence as process death.
use super::{
    directory::{self, Direction, DirectoryTransition},
    *,
};
use gripsack_fs::Dir;
use std::path::{Path, PathBuf};

mod forward;
mod reverse;
mod wire;

fn fixture() -> (tempfile::TempDir, Dir, DirectoryTransition) {
    let temporary = tempfile::tempdir().unwrap();
    let home = gripsack_fs::open_or_create(temporary.path()).unwrap();
    let old = temporary.path().join("store/old-tree");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("untouched"), b"retained payload").unwrap();
    let destination = temporary.path().join("tree");
    std::os::unix::fs::symlink(&old, &destination).unwrap();
    let receipt = directory::prepare(
        destination,
        old,
        crate::GenerationId::new(0),
        vec![PathBuf::from("nested/command")],
    )
    .unwrap();
    (temporary, home, receipt)
}

fn intended() -> Intended {
    Intended::Object(ObjectIdentity::File(crate::canonical_bytes_identity(
        b"new command\n",
        0o755,
    )))
}

fn write_leaf(run: &JournalRun<'_>, receipt: &DirectoryTransition) {
    let destination = receipt.destination.join("nested/command");
    let (parent, name) = receipt.leaf_parent(&destination).unwrap();
    let captured = capture(run, parent, name, &destination, None).unwrap();
    record(captured, &intended())
        .unwrap()
        .execute(|directory, name| {
            gripsack_fs::atomic_write_with_mode(directory, name, b"new command\n", 0o755)
        })
        .unwrap();
}

fn generation(number: u64, receipts: Vec<DirectoryTransition>) -> crate::Generation {
    crate::Generation {
        number: crate::GenerationId::new(number),
        deployments: Default::default(),
        directory_transitions: receipts,
        migration: None,
    }
}
