use super::*;
mod policy_tests;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

struct Fixture {
    _temporary: tempfile::TempDir,
    repo: PathBuf,
    frontend: PathBuf,
    pin: PathBuf,
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        let frontend = temporary.path().join("frontend");
        let pin = temporary.path().join("sdk");
        let home = repo.join("runtime-state");
        for directory in [&repo, &frontend, &pin, &home] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(repo.join(".gitignore"), b"ignored.ts\n").unwrap();
        fs::write(repo.join("ignored.ts"), b"export const version = 1;\n").unwrap();
        fs::set_permissions(repo.join("ignored.ts"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(frontend.join("driver.ts"), b"// driver fixture\n").unwrap();
        fs::write(pin.join("package.json"), br#"{"name":"@gripsack/core"}"#).unwrap();
        fs::write(pin.join("index.ts"), b"export const pin = 1;\n").unwrap();
        fs::create_dir_all(repo.join("vendor_modules/@gripsack")).unwrap();
        symlink(&pin, repo.join("vendor_modules/@gripsack/core")).unwrap();
        symlink("vendor_modules", repo.join("node_modules")).unwrap();
        symlink("ignored.ts", repo.join("alias.ts")).unwrap();
        symlink(repo.join("ignored.ts"), repo.join("absolute.ts")).unwrap();
        Self {
            _temporary: temporary,
            repo,
            frontend,
            pin,
            home,
        }
    }
    fn capture(&self) -> io::Result<SourceBundle> {
        SourceBundle::capture(&self.repo, &self.frontend, Some(&self.pin), &self.home, crate::source_bundle::SourceCapturePolicy::default())
    }
}

#[test]
fn live_edits_and_link_retargets_cannot_change_captured_code_or_pin() {
    let fixture = Fixture::new();
    let captured = fixture.capture().unwrap();
    let owned = captured
        .repository()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    fs::write(
        fixture.repo.join("ignored.ts"),
        b"export const version = 2;\n",
    )
    .unwrap();
    fs::write(fixture.pin.join("index.ts"), b"export const pin = 2;\n").unwrap();
    fs::remove_file(fixture.repo.join("alias.ts")).unwrap();
    symlink("missing", fixture.repo.join("alias.ts")).unwrap();
    assert_eq!(
        fs::read(captured.repository().join("alias.ts")).unwrap(),
        b"export const version = 1;\n"
    );
    assert_eq!(
        fs::read(captured.pinned_frontend().unwrap().join("index.ts")).unwrap(),
        b"export const pin = 1;\n"
    );
    assert_eq!(
        fs::canonicalize(captured.repository().join("absolute.ts")).unwrap(),
        fs::canonicalize(captured.repository().join("ignored.ts")).unwrap()
    );
    assert_eq!(
        fs::canonicalize(captured.repository().join("node_modules/@gripsack/core")).unwrap(),
        captured.pinned_frontend().unwrap()
    );
    assert_eq!(
        captured
            .original_mode(Path::new("alias.ts"))
            .unwrap()
            .bits(),
        0o755
    );
    assert_eq!(
        fs::metadata(captured.repository())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o500
    );
    assert_eq!(
        fs::metadata(captured.repository().join("ignored.ts"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o500
    );
    drop(captured);
    assert!(!owned.exists());
    assert_eq!(
        fs::metadata(fixture.repo.join("ignored.ts"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}

#[test]
fn root_aliases_bind_native_paths_and_diagnostics_without_changing_trust_identity() {
    let fixture = Fixture::new();
    // Keep the fixture's absolute link within the canonical admitted root;
    // this case selects the repository through a separate declared alias.
    fs::remove_file(fixture.repo.join("absolute.ts")).unwrap();
    symlink(
        fixture.repo.canonicalize().unwrap().join("ignored.ts"),
        fixture.repo.join("absolute.ts"),
    )
    .unwrap();
    let alias = fixture._temporary.path().join("declared-repo");
    symlink(&fixture.repo, &alias).unwrap();
    let captured =
        SourceBundle::capture(&alias, &fixture.frontend, Some(&fixture.pin), &fixture.home, crate::source_bundle::SourceCapturePolicy::default())
            .unwrap();
    assert_eq!(
        captured.repository_identity(),
        fixture.repo.canonicalize().unwrap()
    );
    let original = alias.join("ignored.ts");
    let canonical = original.canonicalize().unwrap();
    fs::write(&original, b"changed after capture").unwrap();
    for input in [&original, &canonical] {
        let selected = captured.native_path(input.to_str().unwrap()).unwrap();
        assert_eq!(
            Path::new(selected.as_ref()),
            captured.repository().join("ignored.ts")
        );
        assert_eq!(
            fs::read(selected.as_ref()).unwrap(),
            b"export const version = 1;\n"
        );
    }
    let captured_url = url::Url::from_file_path(captured.repository().join("ignored.ts")).unwrap();
    let declared_url = url::Url::from_file_path(&original).unwrap();
    assert_eq!(
        captured.logical_text(captured_url.as_str()),
        declared_url.as_str()
    );
}

#[test]
fn excluded_state_escape_cycles_and_special_files_never_become_source() {
    let fixture = Fixture::new();
    fs::write(fixture.home.join("secret"), b"private state fixture").unwrap();
    fs::create_dir(fixture.repo.join(".git")).unwrap();
    fs::write(fixture.repo.join(".git/config"), b"control metadata").unwrap();
    let captured = fixture.capture().unwrap();
    assert!(!captured.repository().join("runtime-state").exists());
    assert!(!captured.repository().join(".git").exists());
    let outside = fixture._temporary.path().join("outside");
    fs::write(&outside, b"outside fixture").unwrap();
    for target in [
        outside.as_path(),
        Path::new("runtime-state/secret"),
        Path::new("."),
    ] {
        symlink(target, fixture.repo.join("bad-link")).unwrap();
        assert!(fixture.capture().is_err());
        fs::remove_file(fixture.repo.join("bad-link")).unwrap();
    }
    let socket = std::os::unix::net::UnixListener::bind(fixture.repo.join("socket")).unwrap();
    assert!(fixture.capture().is_err());
    drop(socket);
    assert_eq!(fs::read(outside).unwrap(), b"outside fixture");
    assert_eq!(
        fs::read(fixture.home.join("secret")).unwrap(),
        b"private state fixture"
    );
}

#[test]
fn metadata_length_and_closed_inventory_admission_are_independent_of_digests() {
    let fixture = Fixture::new();
    let captured = fixture.capture().unwrap();
    let document: serde_json::Value = serde_json::from_slice(captured.inventory_bytes()).unwrap();
    let mut invalid = Vec::new();
    let mut version = document.clone();
    version["version"] = serde_json::json!(2);
    invalid.push(version);
    let mut unknown = document.clone();
    unknown["authority"] = serde_json::json!(true);
    invalid.push(unknown);
    let mut absent = document.clone();
    absent.as_object_mut().unwrap().remove("entries");
    invalid.push(absent);
    let mut root = document.clone();
    root["roots"] = serde_json::json!(["repository"]);
    invalid.push(root);
    let mut paths = document;
    paths["entries"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"path":"repo/../outside", "object":{"kind":"directory"}}));
    invalid.push(paths);
    for document in invalid {
        let bytes = serde_json::to_vec(&document).unwrap();
        let digest = SourceBundleDigest::of_inventory(&bytes);
        assert!(SourceInventory::decode(&bytes, digest).is_err());
    }
    let sparse = fs::File::create(fixture.repo.join("oversized")).unwrap();
    sparse.set_len(inventory::MAX_FILE_BYTES + 1).unwrap();
    assert!(fixture.capture().is_err());
    assert_eq!(
        fs::metadata(fixture.repo.join("oversized")).unwrap().len(),
        inventory::MAX_FILE_BYTES + 1
    );
}

#[test]
fn diagnostic_urls_keep_original_spelling_without_exposing_capture_paths() {
    let fixture = Fixture::new();
    let spaced = fixture._temporary.path().join("repo with space");
    fs::rename(&fixture.repo, &spaced).unwrap();
    // The absolute alias named the old source directory; retarget deliberately
    // before capture so both source aliases still denote admitted objects.
    fs::remove_file(spaced.join("absolute.ts")).unwrap();
    symlink(spaced.join("ignored.ts"), spaced.join("absolute.ts")).unwrap();
    let bundle = SourceBundle::capture(&spaced, &fixture.frontend, Some(&fixture.pin), &spaced.join("runtime-state"), crate::source_bundle::SourceCapturePolicy::default())
    .unwrap();
    let captured = url::Url::from_file_path(bundle.repository().join("ignored.ts")).unwrap();
    let logical = url::Url::from_file_path(spaced.join("ignored.ts")).unwrap();
    assert_eq!(bundle.logical_text(captured.as_str()), logical.as_str());
    assert!(matches!(
        bundle.logical_text("modules/ordinary.ts"),
        std::borrow::Cow::Borrowed(_)
    ));
}
