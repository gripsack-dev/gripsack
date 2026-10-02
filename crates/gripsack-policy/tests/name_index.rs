use gripsack_policy::graph::name_index::bind_output_index;

#[test]
fn catalog_indices_cannot_bind_another_name_or_escape_the_catalog() {
    let names = ["build", "hello"];
    let build = bind_output_index(&names, "build", Some(0)).expect("exact catalog entry");
    let hello = bind_output_index(&names, "hello", Some(1)).expect("exact catalog entry");
    assert_ne!(build.position(), hello.position());

    for candidate in [Some(1), Some(2), None] {
        assert!(bind_output_index(&names, "build", candidate).is_none());
    }
    assert!(bind_output_index(&names, "hello", Some(0)).is_none());
    assert!(bind_output_index(&names, "missing", Some(0)).is_none());
}
