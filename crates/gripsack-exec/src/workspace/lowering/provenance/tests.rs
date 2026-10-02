use super::{Origin, SourceMap};
use gripsack_buildkit::{plan::NodeIndex, protocol::FailureCode, transport::TransportError};
use gripsack_ir::Span;
use std::collections::BTreeSet;

#[test]
fn split_shell_diagnostics_preserve_each_shared_requester_source_line() {
    let first = Span {
        file: "recipes/one.ts".into(),
        line: 10,
        col: Some(4),
    };
    let second = Span {
        file: "recipes/two.ts".into(),
        line: 50,
        col: Some(8),
    };
    let node = NodeIndex::new(0).unwrap();
    let mut sources = SourceMap::default();
    sources.push(Origin {
        output: "first",
        span: &first,
        line_map: &[11, 14, 16],
    });
    sources.note(
        node,
        Origin {
            output: "second",
            span: &second,
            line_map: &[51, 55, 57],
        },
    );
    for chunk in [
        b"ordinary output\ngripsack-ba".as_slice(),
        b"sh: li",
        b"ne 2",
        b": command not found\n",
    ] {
        sources.log(&[node], "unused-log-display", chunk, false, None);
    }
    let failed = sources.failure(&TransportError::Rejected {
        code: FailureCode::ExportFailed,
        message: "command failed".into(),
        vertices: Vec::new(),
        nodes: vec![node],
    });
    let locations: BTreeSet<_> = failed
        .labels
        .iter()
        .filter_map(|label| label.span.as_ref())
        .map(|span| (span.file.as_str(), span.line))
        .collect();
    assert_eq!(
        locations,
        BTreeSet::from([("recipes/one.ts", 14), ("recipes/two.ts", 55)])
    );
}
