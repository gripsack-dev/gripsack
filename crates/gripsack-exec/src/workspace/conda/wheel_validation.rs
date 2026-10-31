//! Translate independently admitted wheel bytes into the common tree inventory.
//! Neither the helper's installed RECORD nor its reported file set is authority.
use super::inventory::{Content, ExpectedFile};
use gripsack_conda::wheels::WheelArchive;
use gripsack_ir::workspace_model::lock::{LockedCondaEnvironment, LockedWheel};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    path::Path,
};

fn python_version(locked: &LockedCondaEnvironment) -> Result<&str, String> {
    if locked.platform != "linux-64" {
        return Err("wheel installation requires Linux x86_64".into());
    }
    let python = locked
        .packages
        .iter()
        .find(|package| package.name == "python")
        .ok_or("wheel installation requires selected CPython")?;
    Ok(&python.version)
}

pub(super) fn admit_bounded(
    locked: &LockedCondaEnvironment,
    record: &LockedWheel,
    path: &Path,
    bytes: u64,
    entries: usize,
) -> Result<WheelArchive, String> {
    python_version(locked)?;
    gripsack_conda::wheels::validate_lock(locked).map_err(|error| error.to_string())?;
    WheelArchive::admit_bounded(path, record, bytes, entries).map_err(|error| error.to_string())
}

pub(super) fn inventory(
    locked: &LockedCondaEnvironment,
    record: &LockedWheel,
    archive: &WheelArchive,
    final_prefix: &str,
) -> Result<(BTreeMap<String, ExpectedFile>, u64, usize), String> {
    let target = gripsack_conda::wheels::PythonTarget::from_lock(locked)
        .map_err(|error| error.to_string())?;
    let files = archive
        .installation_inventory_at(
            python_version(locked)?,
            Path::new(final_prefix),
            &target.site_packages,
        )
        .map_err(|error| error.to_string())?;
    let mut inventory = BTreeMap::new();
    let mut generated_bytes = 0u64;
    let mut generated_entries = 0usize;
    for file in files {
        let path = file
            .path()
            .to_str()
            .ok_or("non-UTF-8 wheel installation path")?
            .to_owned();
        if !super::super::archive::safe_path(&path) {
            return Err(format!("unsafe wheel installation path {path:?}"));
        }
        let bytes = file.bytes();
        if file.is_generated() {
            generated_bytes = generated_bytes
                .checked_add(bytes.len() as u64)
                .ok_or("generated wheel byte count overflow")?;
            generated_entries = generated_entries
                .checked_add(1)
                .ok_or("generated wheel entry count overflow")?;
        }
        validate_native(bytes)?;
        let sha256 = super::super::archive::hash_reader(
            &mut Cursor::new(bytes),
            super::super::archive::MAX_ENTRY_BYTES,
        )
        .map_err(|error| error.to_string())?;
        // The inventory owns only an identity, not another copy of the wheel.
        // Generated scripts and RECORD are independently derived by the shared
        // archive parser using the final prefix, then checked identically.
        let expected = ExpectedFile {
            archive: String::new(),
            mode: file.mode(),
            content: Content::Original {
                path: path.clone(),
                sha256,
                size: bytes.len() as u64,
                patch: None,
            },
        };
        if inventory.insert(path.clone(), expected).is_some() {
            return Err(format!("wheel owns installed path {path:?} more than once"));
        }
    }
    let files: BTreeSet<_> = inventory.keys().cloned().collect();
    let mut receipt = Vec::new();
    gripsack_conda::receipt::write_wheel(&mut receipt, record, &files, final_prefix)
        .map_err(|error| error.to_string())?;
    let path = gripsack_conda::receipt::wheel_path(record);
    if inventory
        .insert(
            path.clone(),
            ExpectedFile {
                archive: String::new(),
                mode: gripsack_conda::receipt::FILE_MODE,
                content: Content::Generated(receipt),
            },
        )
        .is_some()
    {
        return Err(format!("wheel payload collides with receipt {path:?}"));
    }
    Ok((inventory, generated_bytes, generated_entries))
}

fn validate_native(bytes: &[u8]) -> Result<(), String> {
    if bytes.starts_with(b"\x7fELF") {
        use gripsack_process::executable::{
            self, Endianness, ExecutableArch, ObjectKind, WordClass,
        };
        let runtime = executable::classify(&mut Cursor::new(bytes))
            .map_err(|error| format!("invalid native wheel payload: {error}"))?;
        if runtime.arch != Some(ExecutableArch::X86_64)
            || runtime.class != Some(WordClass::SixtyFour)
            || runtime.endianness != Some(Endianness::Little)
            || !matches!(
                runtime.object,
                Some(ObjectKind::Executable | ObjectKind::SharedObject)
            )
            || !runtime.elf_loader_extensions.is_empty()
        {
            return Err("wheel native payload is not a supported Linux x86_64 ELF".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{links, merge_inventory};
    use super::*;

    fn file() -> ExpectedFile {
        ExpectedFile {
            archive: String::new(),
            mode: 0o644,
            content: Content::Generated(Vec::new()),
        }
    }

    #[test]
    fn rejects_foreign_wheel_elf_abi() {
        let mut elf = [0u8; 64];
        elf[..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[16] = 3;
        elf[18..20].copy_from_slice(&62u16.to_le_bytes());
        assert!(validate_native(&elf).is_ok());
        elf[18..20].copy_from_slice(&183u16.to_le_bytes());
        assert!(validate_native(&elf).is_err());
        elf[18..20].copy_from_slice(&62u16.to_le_bytes());
        elf[4] = 1;
        assert!(validate_native(&elf).is_err());
        assert!(validate_native(b"\x7fELF").is_err());
    }

    #[test]
    fn rejects_cross_format_file_ownership_collision() {
        let mut conda = BTreeMap::from([("lib/python3.12/site-packages/demo.py".into(), file())]);
        let wheel = BTreeMap::from([("lib/python3.12/site-packages/demo.py".into(), file())]);
        assert!(merge_inventory(&mut conda, wheel).is_err());
    }

    #[test]
    fn rejects_wheel_traversing_conda_file_or_symlink() {
        for content in [
            Content::Generated(Vec::new()),
            Content::Symlink("elsewhere".into()),
        ] {
            let mut conda = BTreeMap::from([(
                "lib/python3.12/site-packages/demo".into(),
                ExpectedFile {
                    archive: "conda".into(),
                    mode: 0o755,
                    content,
                },
            )]);
            let wheel =
                BTreeMap::from([("lib/python3.12/site-packages/demo/module.py".into(), file())]);
            merge_inventory(&mut conda, wheel).unwrap();
            assert!(links::directories(&conda).is_err());
        }
    }

    #[test]
    fn rejects_modified_original_wheel_bytes() {
        use std::io::Write;
        let original = b"original wheel payload";
        let digest =
            super::super::super::archive::hash_reader(&mut Cursor::new(original), 1024).unwrap();
        let expected = ExpectedFile {
            archive: String::new(),
            mode: 0o644,
            content: Content::Original {
                path: "module.py".into(),
                sha256: digest,
                size: original.len() as u64,
                patch: None,
            },
        };
        let mut actual = tempfile::tempfile().unwrap();
        actual.write_all(b"tampered wheel payload").unwrap();
        use std::io::Seek;
        actual.rewind().unwrap();
        assert!(
            super::super::compare_file(&expected, &mut actual, None, "/final", "linux-64").is_err()
        );
    }
}
