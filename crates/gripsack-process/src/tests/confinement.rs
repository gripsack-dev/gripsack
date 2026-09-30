//! Landlock boundary oracles: the OS denies reads outside the assembled
//! boundary while granted reads and scratch writes keep working. These run on
//! Linux only; other platforms fail closed at assembly and are exercised by
//! the evaluator's operational error instead.
use super::*;
use crate::confinement::{Boundary, Ruleset, runtime_read_roots};
use std::path::Path;
use std::path::PathBuf;

fn confine_command(body: &str, boundary: Boundary) -> Command {
    let ruleset = Ruleset::assemble(&boundary, false).expect("landlock ruleset assembles");
    let mut command = command(body);
    // SAFETY: the closure only performs the raw restriction syscalls.
    unsafe {
        command.pre_exec(move || ruleset.restrict());
    }
    command
}

fn sh_boundary(extra_read: &Path) -> Boundary {
    let environment = OperatorEnvironment::capture().unwrap();
    let mut boundary = Boundary::new();
    for root in runtime_read_roots(&environment, Path::new("/bin/sh")).unwrap() {
        boundary = boundary.read_beneath(&root).unwrap();
    }
    boundary.read_beneath(extra_read).unwrap()
}

#[test]
fn confined_process_reads_granted_paths_but_not_outside_them() {
    let root = tempfile::tempdir().unwrap();
    let inside = root.path().join("inside");
    std::fs::create_dir(&inside).unwrap();
    std::fs::write(inside.join("file"), b"granted\n").unwrap();
    let outside = root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("secret"), b"denied\n").unwrap();

    let boundary = sh_boundary(&inside);
    let (outcome, lines) = peer_of(
        confine_command(
            "cat <(printf x) 2>/dev/null; cat 'inside/file'; test ! -r '../outside/secret'",
            boundary,
        ),
        root.path(),
    );
    assert!(outcome.status.unwrap().success(), "{lines:?}");
    assert_eq!(lines, [b"granted".to_vec()]);

    // The same command with the outside path referenced by content fails: the
    // kernel denies the read regardless of the shell's own permissions.
    let boundary = sh_boundary(&inside);
    let (outcome, _) = peer_of(
        confine_command("cat '../outside/secret'", boundary),
        root.path(),
    );
    let status = outcome.status.unwrap();
    assert!(
        !status.success(),
        "ambient read must be denied, got {status}"
    );
}

#[test]
fn confined_process_writes_only_within_scratch_roots() {
    let root = tempfile::tempdir().unwrap();
    let scratch = root.path().join("scratch");
    std::fs::create_dir(&scratch).unwrap();
    let boundary = sh_boundary(&scratch).read_write_beneath(&scratch).unwrap();
    let (outcome, _) = peer_of(
        confine_command(
            "printf written > 'scratch/file'; (printf x > '../denied') 2>/dev/null; test ! -e '../denied'",
            boundary,
        ),
        root.path(),
    );
    assert!(outcome.status.unwrap().success());
    assert_eq!(std::fs::read(scratch.join("file")).unwrap(), b"written");
}

#[test]
fn boundary_roots_must_exist_and_runtime_roots_are_real_directories() {
    let missing = PathBuf::from("/nonexistent-confinement-root");
    assert!(Boundary::new().read_beneath(&missing).is_err());

    let environment = OperatorEnvironment::capture().unwrap();
    let roots = runtime_read_roots(&environment, Path::new("/bin/sh")).unwrap();
    assert!(
        !roots.is_empty(),
        "the runtime must name at least its own directory"
    );
    for root in &roots {
        assert!(root.is_dir(), "{root:?} is not a directory");
    }
}

fn peer_of(mut command: Command, cwd: &Path) -> (Outcome, Vec<Vec<u8>>) {
    command.current_dir(cwd);
    let mut lines = Vec::new();
    let outcome = run(&mut command, b"", limits(), |line| {
        lines.push(line.to_vec());
        Control::Continue
    })
    .expect("supervision");
    (outcome, lines)
}
