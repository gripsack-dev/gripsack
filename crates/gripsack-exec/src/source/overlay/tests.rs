use super::*;

fn plan(froms: &[&str]) -> PreparedModule {
    let entries: Vec<_> = froms
        .iter()
        .enumerate()
        .map(|(index, from)| {
            serde_json::json!({
                "from": from, "to": format!("~/.overlay-{index}"), "mode": "owned",
            })
        })
        .collect();
    let module = serde_json::from_value(serde_json::json!({ "config": entries })).unwrap();
    PreparedModule::new(&module).unwrap()
}

#[test]
fn overlay_pin_and_bytes_belong_to_the_same_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let stage = root.path().join("stage");
    std::fs::create_dir_all(repo.join("config")).unwrap();
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(repo.join("config/file"), b"captured").unwrap();
    let expected = gripsack_store::canonical_overlay_hash(&repo, &["./config".into()]).unwrap();
    let overlay = Overlay::capture(&plan(&["./config"]), &repo, &stage).unwrap();
    std::fs::write(repo.join("config/file"), b"later edit").unwrap();
    assert_eq!(
        overlay.merge(&stage).unwrap().as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(
        std::fs::read(stage.join("config/file")).unwrap(),
        b"captured"
    );
    assert_ne!(
        gripsack_store::canonical_overlay_hash(&repo, &["./config".into()]).unwrap(),
        expected
    );
}

#[cfg(unix)]
#[test]
fn leaf_link_is_replaced_without_writing_its_target() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let stage = root.path().join("stage");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&stage).unwrap();
    let outside = root.path().join("unrelated");
    std::fs::write(&outside, b"keep").unwrap();
    std::fs::write(repo.join("config"), b"overlay").unwrap();
    std::os::unix::fs::symlink(&outside, stage.join("config")).unwrap();
    Overlay::capture(&plan(&["config"]), &repo, &stage)
        .unwrap()
        .merge(&stage)
        .unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    assert_eq!(std::fs::read(stage.join("config")).unwrap(), b"overlay");
    assert!(
        !stage
            .join("config")
            .symlink_metadata()
            .unwrap()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn fetched_link_ancestor_cannot_redirect_an_overlay() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let stage = root.path().join("stage");
    let outside = root.path().join("unrelated");
    std::fs::create_dir_all(repo.join("config")).unwrap();
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("file"), b"keep").unwrap();
    std::fs::write(repo.join("config/file"), b"overlay").unwrap();
    std::os::unix::fs::symlink(&outside, stage.join("config")).unwrap();
    assert!(
        Overlay::capture(&plan(&["config"]), &repo, &stage)
            .unwrap()
            .merge(&stage)
            .is_err()
    );
    assert_eq!(std::fs::read(outside.join("file")).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn top_level_links_have_the_same_preview_and_staged_identity() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let stage = root.path().join("stage");
    std::fs::create_dir_all(repo.join("directory")).unwrap();
    std::fs::create_dir_all(&stage).unwrap();
    std::os::unix::fs::symlink("directory", repo.join("link")).unwrap();
    std::os::unix::fs::symlink("missing", repo.join("dangling")).unwrap();
    let expected =
        gripsack_store::canonical_overlay_hash(&repo, &["link".into(), "dangling".into()]).unwrap();
    let overlay = Overlay::capture(&plan(&["link", "dangling"]), &repo, &stage).unwrap();
    assert_eq!(
        overlay.merge(&stage).unwrap().as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(
        gripsack_store::canonical_tree_hash(&stage).unwrap(),
        expected
    );
}
