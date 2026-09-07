use crate::sandbox::Sandbox;

pub(crate) fn exercise(s: &Sandbox, input: &[u8]) {
    s.write("generations/1/manifest.json", input);
    let first = gripsack_store::read_manifest(s.home(), 1);
    let second = gripsack_store::read_manifest(s.home(), 1);
    match (first, second) {
        (Ok(a), Ok(b)) => {
            assert_eq!(a, b);
            assert_eq!(a.number, 1);
            for state in a.modules.values() {
                assert!(
                    gripsack_store::paths::validate_store_root(s.home(), &state.store_path).is_ok()
                );
                for path in &state.build_closure {
                    assert!(gripsack_store::paths::validate_store_root(s.home(), path).is_ok());
                }
            }
        }
        (Err(a), Err(b)) => assert_eq!(a.kind(), b.kind()),
        _ => panic!("reading an unchanged manifest is nondeterministic"),
    }
    assert_eq!(
        std::fs::read(s.fixed("generations/1/manifest.json")).unwrap(),
        input
    );
}
