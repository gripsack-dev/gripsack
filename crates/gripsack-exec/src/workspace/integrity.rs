//! Disk-only native payload auditing. Retention admits object names; receipts
//! supply full payload identities. Unknown historical roots never gain native
//! interpretation from a matching filename alone.
use super::{artifact, image};
use crate::ExecError;
use gripsack_store as store;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub(crate) enum RootAuthority {
    Workspace,
    Unclassified,
}

pub(crate) enum Finding {
    Corrupt { root: PathBuf, detail: String },
    Unverifiable { root: PathBuf, detail: String },
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TreeLocation {
    Root,
    Payload,
}

pub(super) struct Expectations<'a> {
    pub home: &'a Path,
    roots: &'a BTreeMap<PathBuf, RootAuthority>,
    native: BTreeSet<&'a Path>,
    trees: BTreeMap<(&'a Path, TreeLocation), store::hash::PayloadHash>,
    archives: BTreeMap<&'a Path, gripsack_process::Sha256Digest>,
}

impl<'a> Expectations<'a> {
    pub fn admit_reference(&self, path: &Path) -> Result<&'a Path, ExecError> {
        self.roots
            .get_key_value(path)
            .map(|(root, _)| root.as_path())
            .ok_or_else(|| {
                failure(format!(
                    "receipt references an unretained object: {}",
                    path.display()
                ))
            })
    }

    pub fn retain(&mut self, path: &Path) -> Result<(), ExecError> {
        self.native.insert(self.admit_reference(path)?);
        Ok(())
    }

    pub fn archive(&mut self, digest: &str) -> Result<(), ExecError> {
        let expected = gripsack_process::Sha256Digest::parse(digest)?;
        let path = store::content_path(self.home, "conda-archive", digest);
        let root = self.admit_reference(&path)?;
        if let Some(previous) = self.archives.get(root) {
            if previous != &expected {
                return Err(failure(
                    "retained receipts disagree about an archive identity",
                ));
            }
        } else {
            self.archives.insert(root, expected);
        }
        self.native.insert(root);
        Ok(())
    }

    pub fn tree(
        &mut self,
        object: &Path,
        payload: &Path,
        expected: store::hash::PayloadHash,
    ) -> Result<(), ExecError> {
        let object = self.admit_reference(object)?;
        let admitted = match namespace(object) {
            Some(kind) if content_addressed_tree(kind) || kind == "workspace-conda" => {
                TreeLocation::Root
            }
            Some("workspace-recipe" | "workspace-image" | "workspace-conda-image") => {
                TreeLocation::Payload
            }
            _ => {
                return Err(failure(
                    "receipt names an unsupported native payload domain",
                ));
            }
        };
        let location = if payload == object {
            TreeLocation::Root
        } else if payload.parent() == Some(object)
            && payload.file_name().is_some_and(|name| name == "payload")
        {
            TreeLocation::Payload
        } else {
            return Err(failure(
                "receipt payload is not the admitted object or its payload directory",
            ));
        };
        if location != admitted {
            return Err(failure(
                "receipt payload does not match its native object layout",
            ));
        }
        if let Some(kind) = namespace(object).filter(|kind| content_addressed_tree(kind)) {
            bind_identity(object, kind, expected.as_str())?;
        }
        let key = (object, location);
        if let Some(previous) = self.trees.get(&key) {
            if previous != &expected {
                return Err(failure(
                    "retained receipts disagree about a payload identity",
                ));
            }
        } else {
            self.trees.insert(key, expected);
        }
        self.native.insert(object);
        Ok(())
    }
}

pub(super) fn bind_identity(root: &Path, namespace: &str, digest: &str) -> Result<(), ExecError> {
    if identity_matches(root, namespace, digest) {
        Ok(())
    } else {
        Err(failure(
            "receipt identity does not name its retained store object",
        ))
    }
}

fn identity_matches(root: &Path, namespace: &str, digest: &str) -> bool {
    root.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split_once('-'))
        .is_some_and(|(prefix, kind)| {
            kind == namespace && digest.get(..store::paths::HASH_LEN * 2) == Some(prefix)
        })
}

pub(super) fn bind_digest(
    root: &Path,
    namespace: &str,
    digest: &[u8; 32],
) -> Result<(), ExecError> {
    if digest_matches(root, namespace, digest) {
        Ok(())
    } else {
        Err(failure(
            "receipt identity does not name its retained store object",
        ))
    }
}

fn digest_matches(root: &Path, namespace: &str, digest: &[u8; 32]) -> bool {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    root.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split_once('-'))
        .is_some_and(|(prefix, kind)| {
            kind == namespace
                && prefix.len() == store::paths::HASH_LEN * 2
                && prefix
                    .as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(digest)
                    .all(|(pair, byte)| {
                        pair[0] == HEX[(byte >> 4) as usize]
                            && pair[1] == HEX[(byte & 0xf) as usize]
                    })
        })
}

pub(crate) fn inspect(
    home: &Path,
    roots: &BTreeMap<PathBuf, RootAuthority>,
) -> Result<Vec<Finding>, ExecError> {
    for root in roots.keys() {
        store::paths::validate_store_root(home, root)?;
    }
    let mut expected = Expectations {
        home,
        roots,
        native: roots
            .iter()
            .filter_map(|(root, origin)| {
                matches!(origin, RootAuthority::Workspace).then_some(root.as_path())
            })
            .collect(),
        trees: BTreeMap::new(),
        archives: BTreeMap::new(),
    };
    let mut findings = Vec::new();
    for root in roots.keys() {
        let result = match namespace(root) {
            Some("workspace-package") => artifact::integrity::package(root, &mut expected),
            Some("workspace-recipe") => artifact::integrity::recipe(root, &mut expected),
            Some("workspace-image") => image::integrity(root, &mut expected),
            Some("workspace-conda-image") => image::materialize::integrity(root, &mut expected),
            // Review artifacts bind the complete tree digest (not a shortened
            // payload lookup key) and have their own bounded private schema.
            Some("semantic-review") => crate::semantic::retention::audit_root(home, root)
                .map_err(|error| failure(error.to_string())),
            _ => continue,
        };
        if let Err(error) = result {
            findings.push(Finding::Unverifiable {
                root: root.clone(),
                detail: error.to_string(),
            });
        }
    }

    // One traversal per payload even when many packages/generations retain it.
    for (&(root, location), digest) in &expected.trees {
        match observe_tree(home, root, location) {
            Ok(actual) if actual != *digest => {
                if namespace(root)
                    .filter(|kind| content_addressed_tree(kind))
                    .is_some_and(|kind| identity_matches(root, kind, actual.as_str()))
                {
                    findings.push(Finding::Unverifiable {
                        root: root.to_owned(),
                        detail: format!("receipt records {digest} but the self-identifying payload hashes {actual}"),
                    });
                } else {
                    findings.push(Finding::Corrupt {
                        root: root.to_owned(),
                        detail: "payload differs from its retained receipt".into(),
                    });
                }
            }
            Err(error) => findings.push(Finding::Unverifiable {
                root: root.to_owned(),
                detail: error.to_string(),
            }),
            _ => {}
        }
    }
    for root in expected.native {
        match namespace(root) {
            Some(kind)
                if content_addressed_tree(kind)
                    && !expected.trees.contains_key(&(root, TreeLocation::Root)) =>
            {
                match observe_tree(home, root, TreeLocation::Root) {
                    Ok(actual) if !identity_matches(root, kind, actual.as_str()) => {
                        findings.push(Finding::Corrupt {
                            root: root.to_owned(),
                            detail: "content-addressed tree differs from its store identity".into(),
                        });
                    }
                    Err(error) => findings.push(Finding::Unverifiable {
                        root: root.to_owned(),
                        detail: error.to_string(),
                    }),
                    _ => {}
                }
            }
            Some("conda-archive") => match archive_digest(home, root) {
                Ok(digest) if !digest_matches(root, "conda-archive", digest.as_bytes()) => {
                    findings.push(Finding::Corrupt {
                        root: root.to_owned(),
                        detail: "archive bytes differ from their store identity".into(),
                    });
                }
                Ok(digest)
                    if expected
                        .archives
                        .get(root)
                        .is_some_and(|expected| *expected != digest) =>
                {
                    findings.push(Finding::Unverifiable {
                        root: root.to_owned(),
                        detail: "receipt conflicts with self-identifying archive bytes".into(),
                    });
                }
                Err(error) => findings.push(Finding::Unverifiable {
                    root: root.to_owned(),
                    detail: error.to_string(),
                }),
                _ => {}
            },
            Some("workspace-conda")
                if !expected.trees.contains_key(&(root, TreeLocation::Root)) =>
            {
                findings.push(Finding::Unverifiable {
                    root: root.to_owned(),
                    detail: "native prefix has no retained package payload identity".into(),
                });
            }
            _ => {}
        }
    }
    Ok(findings)
}

fn namespace(root: &Path) -> Option<&str> {
    root.file_name()?
        .to_str()?
        .split_once('-')
        .map(|(_, kind)| kind)
}

fn content_addressed_tree(namespace: &str) -> bool {
    matches!(
        namespace,
        "workspace-source" | "workspace-input" | "workspace-files" | "package-payload"
    )
}

fn observe_tree(
    home: &Path,
    root: &Path,
    location: TreeLocation,
) -> Result<store::hash::PayloadHash, ExecError> {
    let directory = artifact::retained_directory(home, root)?
        .ok_or_else(|| failure("retained native store object is missing"))?;
    let directory = match location {
        TreeLocation::Root => directory,
        TreeLocation::Payload => gripsack_fs::open_dir_nofollow(&directory, Path::new("payload"))?,
    };
    Ok(store::hash::canonical_tree_hash_in(&directory)?)
}

fn archive_digest(home: &Path, root: &Path) -> Result<gripsack_process::Sha256Digest, ExecError> {
    let directory = artifact::retained_directory(home, root)?
        .ok_or_else(|| failure("retained Conda archive object is missing"))?;
    let name = Path::new("archive");
    if !directory.symlink_metadata(name)?.is_file() {
        return Err(failure("retained Conda archive is not a regular file"));
    }
    let mut file = gripsack_fs::open_file_nofollow(&directory, name)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => digest.update(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(gripsack_process::Sha256Digest::from_bytes(
        digest.finalize().into(),
    ))
}

pub(super) fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        output: "workspace".into(),
        step: "store-verify".into(),
        detail: detail.into(),
    }
}
