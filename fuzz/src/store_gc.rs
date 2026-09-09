use crate::sandbox::Sandbox;
use gripsack_store::{Generation, ModuleState};
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) fn exercise(s: &Sandbox, input: &[u8]) {
    if let Ok(raw) = std::str::from_utf8(input) {
        // Readonly lexical validation: never forward this path to GC or a writer.
        let _ = gripsack_store::paths::validate_store_root(s.home(), Path::new(raw));
    }
    for rel in [
        "store/a/payload",
        "store/b/payload",
        "store/c/payload",
        "store/orphan/payload",
    ] {
        s.write(rel, b"bounded payload");
    }
    for (number, object) in [(1, "store/a"), (2, "store/b"), (3, "store/c")] {
        let state = ModuleState {
            store_path: s.fixed(object),
            build_only: true,
            entries: vec![],
            intents: vec![],
            verified: None,
            env: vec![],
            tree256: None,
            build_closure: if number == 3 {
                vec![s.fixed("store/a")]
            } else {
                vec![]
            },
        };
        gripsack_store::write_manifest(
            s.cap(),
            &Generation {
                number,
                modules: BTreeMap::from([("m".to_owned(), state)]),
            },
        )
        .unwrap();
    }
    gripsack_store::flip(s.cap(), s.home(), 3).unwrap();
    let keep = input.first().map(|n| u32::from(*n % 5));
    let session = gripsack_exec::LifecycleSession::acquire(s.home()).unwrap();
    let report = gripsack_exec::gc(&session, keep, true).unwrap();
    assert!(!report.generations_removed.contains(&3));
    assert!(
        !report.store_removed.contains(&s.fixed("store/a")),
        "retained build closure must pin a"
    );
    assert!(!report.store_removed.contains(&s.fixed("store/c")));
    assert!(report.store_removed.contains(&s.fixed("store/orphan")));
    for rel in [
        "store/a/payload",
        "store/b/payload",
        "store/c/payload",
        "store/orphan/payload",
    ] {
        assert_eq!(std::fs::read(s.fixed(rel)).unwrap(), b"bounded payload");
    }
    assert_eq!(
        gripsack_store::list_generations(s.home()).unwrap(),
        vec![1, 2, 3]
    );
    // Corrupt even a prunable manifest: real GC must fail closed.
    s.write(
        "generations/1/manifest.json",
        b"{\"number\":-1,\"modules\":{}}",
    );
    assert!(gripsack_exec::gc(&session, Some(0), true).is_err());
    assert!(s.fixed("store/orphan/payload").exists());
}
