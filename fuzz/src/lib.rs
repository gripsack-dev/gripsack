pub mod archive;
pub mod journal;
pub mod manifest;
pub mod merge;
mod sandbox;
pub mod store_gc;

pub const MAX_INPUT: usize = 65_536;
pub const TARGETS: &[&str] = &["manifest", "journal", "merge", "store_gc", "archive"];

/// Replay and libFuzzer enter the identical implementation.
pub fn dispatch(target: &str, input: &[u8]) {
    assert!(TARGETS.contains(&target), "unknown target: {target}");
    if input.len() > MAX_INPUT {
        return;
    }
    // Runner establishes an OS boundary; temp paths alone cannot contain bugs.
    assert_eq!(
        std::env::var("GRIPSACK_FUZZ_ISOLATED").as_deref(),
        Ok("1"),
        "use fuzz/run.py; direct execution has no OS containment"
    );
    let _guard = sandbox::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let sandbox = sandbox::Sandbox::new();
    match target {
        "manifest" => manifest::exercise(&sandbox, input),
        "journal" => journal::exercise(&sandbox, input),
        "merge" => merge::exercise(input),
        "store_gc" => store_gc::exercise(&sandbox, input),
        "archive" => archive::exercise(&sandbox, input),
        _ => unreachable!(),
    }
}
