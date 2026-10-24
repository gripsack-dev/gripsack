//! One strict root selection shared by publication, inspection and replay.
use super::*;
use gripsack_ir::workspace_model::ArchiveRootPolicy;

pub(super) fn select(
    policy: Option<&ArchiveRootPolicy>,
    layout: &CatalogPackageLayout,
    payload: &Path,
) -> Result<Option<String>, ExecError> {
    let Some(ArchiveRootPolicy::SingleDirectory) = policy else {
        return Ok(None);
    };
    if !matches!(layout, CatalogPackageLayout::Relocatable) {
        return Err(failure(
            "archive root selection requires a relocatable native package",
        ));
    }
    if !std::fs::symlink_metadata(payload)?.is_dir() {
        return Err(failure("archive root payload must be a real directory"));
    }
    let mut entries = std::fs::read_dir(payload)?;
    let entry = entries.next().transpose()?.ok_or_else(|| {
        failure("single_directory archive root requires exactly one real directory")
    })?;
    if entries.next().transpose()?.is_some() || !entry.file_type()?.is_dir() {
        return Err(failure(
            "single_directory archive root refuses files, links and additional root entries",
        ));
    }
    let name = entry
        .file_name()
        .into_string()
        .map_err(|_| failure("archive root directory name is not UTF-8"))?;
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(failure("archive root directory name is unsafe"));
    }
    Ok(Some(name))
}

pub(super) fn payload(producer: &Path, selected: Option<&str>) -> PathBuf {
    selected.map_or_else(|| producer.to_owned(), |root| producer.join(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn select_root(path: &Path) -> Result<Option<String>, ExecError> {
        select(
            Some(&ArchiveRootPolicy::SingleDirectory),
            &CatalogPackageLayout::Relocatable,
            path,
        )
    }

    #[test]
    fn versioned_root_keeps_literal_selector() {
        for version in ["ripgrep-1-x86_64", "ripgrep-2-x86_64"] {
            let temp = tempfile::tempdir().unwrap();
            std::fs::create_dir(temp.path().join(version)).unwrap();
            std::fs::write(temp.path().join(version).join("rg"), b"command").unwrap();
            let selected = select_root(temp.path()).unwrap();
            assert_eq!(selected.as_deref(), Some(version));
            assert_eq!(
                std::fs::read(payload(temp.path(), selected.as_deref()).join("rg")).unwrap(),
                b"command"
            );
        }
    }

    #[test]
    fn receipt_versions_do_not_grant_root_authority_to_old_receipts() {
        let mut wire = serde_json::json!({
            "version": 1,
            "package": "0".repeat(64),
            "producer_root": "/store/source",
            "producer_payload": "/store/source",
            "tree": "0".repeat(64),
            "commands": {},
            "runtime": {},
            "target": {"os": "linux", "arch": "x86_64"},
            "layout": {"kind": "relocatable"}
        });
        let receipt: PackageReceipt = serde_json::from_value(wire.clone()).unwrap();
        assert!(receipt.supported_version());
        assert_eq!(serde_json::to_value(receipt).unwrap(), wire);
        wire["archive_root"] = serde_json::json!({"kind": "single_directory"});
        wire["selected_root"] = serde_json::json!("ripgrep-v1");
        for version in [1, 2, 3] {
            wire["version"] = serde_json::json!(version);
            let receipt: PackageReceipt = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(receipt.supported_version(), version == 3);
        }
        wire.as_object_mut().unwrap().remove("archive_root");
        let receipt: PackageReceipt = serde_json::from_value(wire).unwrap();
        assert!(!receipt.supported_version());
    }

    #[test]
    fn refuses_materialized_and_fixed_prefix_layouts() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("root")).unwrap();
        for layout in [
            CatalogPackageLayout::PrefixMaterialized,
            serde_json::from_str(r#"{"kind":"fixed_prefix","prefix":"/opt/tools"}"#).unwrap(),
        ] {
            assert!(
                select(
                    Some(&ArchiveRootPolicy::SingleDirectory),
                    &layout,
                    temp.path()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn refuses_empty_ambiguous_files_metadata_and_root_alias() {
        let temp = tempfile::tempdir().unwrap();
        assert!(select_root(temp.path()).is_err());
        std::fs::write(temp.path().join("metadata"), b"metadata").unwrap();
        assert!(select_root(temp.path()).is_err());
        std::fs::create_dir(temp.path().join("root")).unwrap();
        assert!(select_root(temp.path()).is_err());
        std::fs::remove_file(temp.path().join("metadata")).unwrap();
        std::fs::create_dir(temp.path().join("second")).unwrap();
        assert!(select_root(temp.path()).is_err());
        std::fs::remove_dir(temp.path().join("second")).unwrap();
        std::fs::remove_dir(temp.path().join("root")).unwrap();
        let target = tempfile::tempdir().unwrap();
        symlink(target.path(), temp.path().join("root")).unwrap();
        assert!(select_root(temp.path()).is_err());
    }
}
