//! Deterministic, independent histories over the real collector. Expected roots
//! come from fixture history, never the collector's projection/deletion kernel.
use super::gc;
use crate::LifecycleSession;
use gripsack_store::{self as store, GenerationId};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const OBJECTS: [&str; 7] = [
    "old-consumer",
    "old-compiler",
    "old-sdk",
    "new-consumer",
    "new-compiler",
    "new-sdk",
    "orphan",
];

fn history() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path();
    let cap = gripsack_fs::open_or_create(home).unwrap();
    for name in OBJECTS {
        let path = home.join("store").join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("payload"), name).unwrap();
    }
    let prior = store::prior::store_blob(&cap, b"original user bytes").unwrap();
    for (number, names) in [(1, &OBJECTS[..3]), (2, &OBJECTS[3..6]), (3, &OBJECTS[..0])] {
        let mut modules = BTreeMap::new();
        if !names.is_empty() {
            modules.insert(
                "consumer".into(),
                store::ModuleState {
                    store_path: home.join("store").join(names[0]),
                    build_only: false,
                    entries: vec![store::DeployedEntry {
                        from: "payload".into(),
                        to: home.join("deployed").to_str().unwrap().into(),
                        key: None,
                        ownership: store::StoredOwnership::Legacy(
                            gripsack_ir::Ownership::TrackedCopy,
                        ),
                        vars: Default::default(),
                        hash: store::hash::ManifestHash::from_raw("a".repeat(64)),
                        file_mode: Some(0o600),
                        source_executable: None,
                        prior: Some(store::Prior::File {
                            hash: prior.clone(),
                            mode: store::prior::FileMode::try_from(0o600).unwrap(),
                        }),
                        preserved_drift: false,
                    }],
                    intents: vec![],
                    verified: None,
                    env: vec![],
                    tree256: None,
                    build_closure: names[1..]
                        .iter()
                        .map(|name| home.join("store").join(name))
                        .collect(),
                },
            );
        }
        store::write_manifest(
            &cap,
            &store::Generation {
                number: GenerationId::new(number),
                modules,
            },
        )
        .unwrap();
    }
    let blob = prior.path_in(home);
    (temporary, blob)
}

fn assert_payloads(home: &Path, retained: &[&str]) {
    for name in OBJECTS {
        let path = home.join("store").join(name).join("payload");
        if retained.contains(&name) {
            assert_eq!(
                fs::read(&path).ok().as_deref(),
                Some(name.as_bytes()),
                "retained_history_root_missing: {name}"
            );
        } else {
            assert!(!path.exists(), "unrooted_history_object_retained: {name}");
        }
    }
}

#[test]
fn retained_histories_protect_complete_roots() {
    // Explicit expected retained generations and roots: no reimplementation of
    // the excess-prefix algorithm is used to compute this oracle.
    struct RetentionHistory<'a> {
        keep: Option<u32>,
        current: Option<GenerationId>,
        retained_generations: &'a [GenerationId],
        roots: &'a [&'a str],
    }
    let ids = [1, 2, 3].map(GenerationId::new);
    let cases = [
        RetentionHistory {
            keep: None,
            current: Some(ids[2]),
            retained_generations: &ids,
            roots: &OBJECTS[..6],
        },
        RetentionHistory {
            keep: Some(0),
            current: Some(ids[1]),
            retained_generations: &ids[1..2],
            roots: &OBJECTS[3..6],
        },
        RetentionHistory {
            keep: Some(1),
            current: Some(ids[0]),
            retained_generations: &[ids[0], ids[2]],
            roots: &OBJECTS[..3],
        },
        RetentionHistory {
            keep: Some(1),
            current: Some(ids[2]),
            retained_generations: &ids[2..],
            roots: &[],
        },
        RetentionHistory {
            keep: Some(2),
            current: Some(ids[2]),
            retained_generations: &ids[1..],
            roots: &OBJECTS[3..6],
        },
        RetentionHistory {
            keep: Some(0),
            current: None,
            retained_generations: &[],
            roots: &[],
        },
        RetentionHistory {
            keep: Some(u32::MAX),
            current: None,
            retained_generations: &ids,
            roots: &OBJECTS[..6],
        },
    ];
    for RetentionHistory {
        keep,
        current,
        retained_generations,
        roots,
    } in cases
    {
        let (temporary, prior) = history();
        let home = temporary.path();
        if let Some(current) = current {
            std::os::unix::fs::symlink(format!("generations/{current}"), home.join("current"))
                .unwrap();
        }
        let session = LifecycleSession::acquire(home).unwrap();
        let preview = gc(&session, keep, true).unwrap();
        assert_payloads(home, &OBJECTS);
        assert_eq!(fs::read(&prior).unwrap(), b"original user bytes");
        for id in 1..=3 {
            assert!(
                home.join(format!("generations/{id}/manifest.json"))
                    .exists()
            );
        }
        let collected = gc(&session, keep, false).unwrap();
        assert_eq!(preview.generations_removed, collected.generations_removed);
        assert_eq!(preview.store_removed, collected.store_removed);
        assert_eq!(preview.bytes_freed, collected.bytes_freed);
        assert_payloads(home, roots);
        assert_eq!(
            prior.exists(),
            !roots.is_empty(),
            "prior lifetime follows adopted history"
        );
        for id in 1..=3 {
            assert_eq!(
                home.join(format!("generations/{id}/manifest.json"))
                    .exists(),
                retained_generations.contains(&GenerationId::new(id))
            );
        }
        assert_eq!(store::current_generation(home).unwrap(), current);
    }
    println!("GC_RETAINED_HISTORIES=7");
}

#[test]
fn unfinished_recovery_preserves_every_history_object() {
    for state in ["marker", "entry", "quarantine", "malformed"] {
        let (temporary, prior) = history();
        let home = temporary.path();
        let cap = gripsack_fs::open(home).unwrap();
        std::os::unix::fs::symlink("generations/3", home.join("current")).unwrap();
        let journal_blob = store::prior::store_blob(&cap, b"journal-only bytes")
            .unwrap()
            .path_in(home);
        fs::create_dir_all(home.join("journal")).unwrap();
        let evidence = match state {
            "marker" => {
                store::journal::begin_run(
                    &cap,
                    home,
                    Some(GenerationId::new(3)),
                    GenerationId::new(4),
                    store::journal::RunOp::Apply,
                )
                .unwrap();
                home.join("journal/run.json")
            }
            "entry" => {
                let path = home.join("journal/entry.json");
                fs::write(&path, b"retained recovery evidence").unwrap();
                path
            }
            "quarantine" => {
                fs::create_dir(home.join("journal/quarantine")).unwrap();
                let path = home.join("journal/quarantine/entry.json");
                fs::write(&path, b"quarantined evidence").unwrap();
                path
            }
            _ => {
                let path = home.join("journal/run.json");
                fs::write(&path, b"{torn").unwrap();
                path
            }
        };
        let bytes = fs::read(&evidence).unwrap();
        let session = LifecycleSession::acquire(home).unwrap();
        for dry_run in [true, false] {
            assert!(gc(&session, Some(0), dry_run).is_err());
            assert_payloads(home, &OBJECTS);
            assert_eq!(fs::read(&evidence).unwrap(), bytes);
            assert_eq!(fs::read(&journal_blob).unwrap(), b"journal-only bytes");
            assert_eq!(fs::read(&prior).unwrap(), b"original user bytes");
            for id in 1..=3 {
                assert!(
                    home.join(format!("generations/{id}/manifest.json"))
                        .exists()
                );
            }
        }
    }
    println!("GC_RECOVERY_ADMISSIONS=8");
}

#[test]
fn pinned_collection_survives_root_replacement() {
    use super::inventory::ObjectInventory;
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let cap = gripsack_fs::open_or_create(&home).unwrap();
    fs::create_dir_all(home.join("store/orphan")).unwrap();
    fs::write(home.join("store/orphan/payload"), b"owned").unwrap();
    let inventory = ObjectInventory::open(&cap, &home, "store").unwrap();
    let plan = inventory.plan(&[]).unwrap();
    fs::rename(home.join("store"), home.join("moved-store")).unwrap();
    let outside = temporary.path().join("outside");
    fs::create_dir_all(outside.join("orphan")).unwrap();
    fs::write(outside.join("orphan/payload"), b"foreign").unwrap();
    std::os::unix::fs::symlink(&outside, home.join("store")).unwrap();
    plan.collect(false, &mut super::GcReport::default())
        .unwrap();
    assert!(!home.join("moved-store/orphan").exists());
    assert_eq!(
        fs::read(outside.join("orphan/payload")).unwrap(),
        b"foreign"
    );
    println!("GC_PINNED_ROOT_REPLACEMENTS=1");
}
