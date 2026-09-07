use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let target = args.next().expect("target");
    let root = PathBuf::from(args.next().expect("corpus directory"));
    assert!(args.next().is_none());
    assert!(gripsack_fuzz::TARGETS.contains(&target.as_str()));
    let mut paths = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "empty corpus");
    for path in paths {
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(meta.is_file(), "corpus must contain only regular files");
        assert!(meta.len() <= gripsack_fuzz::MAX_INPUT as u64);
        let bytes = std::fs::read(&path).unwrap();
        eprintln!("replay {target}: {}", path.display());
        gripsack_fuzz::dispatch(&target, &bytes);
    }
}
