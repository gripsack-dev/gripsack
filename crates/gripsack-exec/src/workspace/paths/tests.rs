use super::resolve_exact;
use gripsack_ir::{Span, codes};
use std::{fs, os::unix::fs::symlink};

#[test]
fn canonical_selection_keeps_internal_aliases_and_refuses_escaped_files_and_directories() {
    let temporary = tempfile::tempdir().unwrap();
    let payload = temporary.path().join("payload");
    let outside = temporary.path().join("outside");
    fs::create_dir_all(payload.join("inside")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(payload.join("inside/value"), b"owned").unwrap();
    fs::write(outside.join("value"), b"foreign").unwrap();
    fs::write(payload.join("literal-{version}"), b"retained literal").unwrap();
    symlink("inside", payload.join("alias")).unwrap();
    symlink("../outside", payload.join("escape")).unwrap();
    let span = Span {
        file: "selectors.ts".into(),
        line: 7,
        col: None,
    };

    assert_eq!(
        resolve_exact(&payload, ".", &span).unwrap(),
        payload.canonicalize().unwrap(),
    );
    assert_eq!(
        resolve_exact(&payload, "alias/value", &span).unwrap(),
        payload.join("inside/value").canonicalize().unwrap(),
    );
    assert_eq!(
        fs::read(resolve_exact(&payload, "literal-{version}", &span).unwrap()).unwrap(),
        b"retained literal",
    );
    for selector in ["escape", "escape/value", "../outside/value"] {
        let error = resolve_exact(&payload, selector, &span).unwrap_err();
        let crate::ExecError::Gate(diagnostic) = error else {
            panic!("escape must be rejected at selector admission");
        };
        assert_eq!(diagnostic.code, codes::BAD_WORKSPACE_CONTEXT);
        assert_eq!(diagnostic.labels[0].span.as_ref(), Some(&span));
    }
}
