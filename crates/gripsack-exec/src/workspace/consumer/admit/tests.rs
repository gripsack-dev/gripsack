use super::*;
use gripsack_ir::workspace::{OsVersion, PlatformAbi, PlatformArch, PlatformOs};
use gripsack_process::executable::{
    Endianness, ExecutableArch, ExecutableFormat, Interpreter, ObjectKind, WordClass,
};

#[test]
fn declared_abi_cannot_hide_an_unproven_minimum_os_floor() {
    let target = WorkspacePlatform {
        os: PlatformOs::Linux,
        arch: PlatformArch::X86_64,
        abi: Some(PlatformAbi::Gnu),
        minimum_os: Some(OsVersion {
            major: 6,
            minor: 0,
            patch: None,
        }),
    };
    let host = HostTarget {
        requirement: TargetRequirement {
            os: TargetOs::Linux,
            arch: TargetArch::X86_64,
            abi: Some(BinaryAbi::Gnu),
            minimum_os: None,
        },
        os: TargetOs::Linux,
        arch: TargetArch::X86_64,
        abi: Some(BinaryAbi::Gnu),
    };
    let span = Span {
        file: "workspace.ts".into(),
        line: 1,
        col: None,
    };
    assert!(admit_target(&target, &host, &span, "package").is_err());
    let no_floor = WorkspacePlatform {
        minimum_os: None,
        ..target
    };
    assert!(admit_target(&no_floor, &host, &span, "package").is_ok());
}

#[test]
fn native_runtime_libraries_require_compatible_objects_and_contained_targets() {
    use crate::workspace::artifact::Artifact;
    use gripsack_process::executable::ExecutableMetadata;
    use std::sync::Arc;

    let directory = tempfile::tempdir().unwrap();
    let payload = directory.path().canonicalize().unwrap();
    std::fs::create_dir(payload.join("lib")).unwrap();
    let package = Package {
        identity: serde_json::from_str(&format!("\"{}\"", "0".repeat(64))).unwrap(),
        root: payload.clone(),
        producer: Arc::new(Artifact {
            root: payload.clone(),
            payload: payload.clone(),
            tree: serde_json::from_str(&format!("\"{}\"", "0".repeat(64))).unwrap(),
            retention: BTreeSet::new(),
        }),
        commands: BTreeMap::new(),
        runtime: Vec::new(),
        target: WorkspacePlatform {
            os: PlatformOs::Linux,
            arch: PlatformArch::X86_64,
            abi: Some(PlatformAbi::Gnu),
            minimum_os: None,
        },
        layout: CatalogPackageLayout::Relocatable,
        conda: None,
        host_runtime: None,
    };
    let host = HostTarget {
        requirement: TargetRequirement {
            os: TargetOs::Linux,
            arch: TargetArch::X86_64,
            abi: Some(BinaryAbi::Gnu),
            minimum_os: None,
        },
        os: TargetOs::Linux,
        arch: TargetArch::X86_64,
        abi: Some(BinaryAbi::Gnu),
    };
    let mut metadata = ExecutableMetadata {
        format: Some(ExecutableFormat::Elf),
        class: Some(WordClass::SixtyFour),
        arch: Some(ExecutableArch::X86_64),
        endianness: Some(Endianness::Little),
        object: Some(ObjectKind::Executable),
        has_entry_point: true,
        interpreter: Some(Interpreter::Loader("/lib64/ld-linux-x86-64.so.2".into())),
        needed_libraries: vec!["libexample.so".into()],
        ..ExecutableMetadata::default()
    };
    let span = Span {
        file: "workspace.ts".into(),
        line: 12,
        col: None,
    };
    let admit = |metadata: &ExecutableMetadata| {
        elf::admit(
            metadata,
            &payload.join("program"),
            &payload,
            &package,
            Some(BinaryAbi::Gnu),
            &host,
            &span,
        )
    };
    let mut library = vec![0; 64];
    library[..6].copy_from_slice(b"\x7fELF\x02\x01");
    library[16..18].copy_from_slice(&3u16.to_le_bytes());
    library[18..20].copy_from_slice(&183u16.to_le_bytes());
    let path = payload.join("lib/libexample.so");
    std::fs::write(&path, &library).unwrap();
    assert!(
        admit(&metadata).is_err(),
        "a library for a different machine must be refused"
    );
    library[18..20].copy_from_slice(&62u16.to_le_bytes());
    std::fs::write(&path, &library).unwrap();
    assert_eq!(
        admit(&metadata).unwrap().directories,
        vec![payload.join("lib")]
    );
    metadata
        .elf_loader_extensions
        .push(gripsack_process::executable::ElfLoaderExtension::Audit(
            "outside.so".into(),
        ));
    assert!(
        admit(&metadata).is_err(),
        "main-image auditing escapes DT_NEEDED"
    );
    metadata.elf_loader_extensions.clear();
    for tag in [0x6fff_fefb_i64, 0x6fff_fefc, 0x7fff_fffd, 0x7fff_ffff] {
        let mut indirect = dynamic_library("outside.so");
        indirect[0x110..0x118].copy_from_slice(&tag.to_le_bytes());
        std::fs::write(&path, indirect).unwrap();
        assert!(
            admit(&metadata).is_err(),
            "a dependency's audit/filter object must not escape the admitted graph",
        );
    }
    std::fs::write(&path, &library).unwrap();
    for capability in ["tls", "haswell", "x86_64", "avx512_1"] {
        let shadow = payload.join("lib").join(capability);
        std::fs::create_dir(&shadow).unwrap();
        assert!(
            admit(&metadata).is_ok(),
            "an empty reserved directory cannot shadow lookup"
        );
        std::fs::write(shadow.join("libexample.so"), &library).unwrap();
        assert!(
            admit(&metadata).is_err(),
            "legacy hwcap lookup must not substitute an unadmitted library"
        );
        std::fs::remove_file(shadow.join("libexample.so")).unwrap();
        std::fs::remove_dir(shadow).unwrap();
    }
    for unsafe_name in ["lib;other", "lib$PLATFORM"] {
        let directory = payload.join(unsafe_name);
        std::fs::create_dir(&directory).unwrap();
        metadata.rpaths = vec![format!("$ORIGIN/{unsafe_name}").into()];
        assert!(
            admit(&metadata).is_err(),
            "an admitted literal path must not expand into other loader search paths"
        );
        std::fs::remove_dir(directory).unwrap();
    }
    metadata.rpaths.clear();
    let ordinary_needed =
        std::mem::replace(&mut metadata.needed_libraries, vec!["$PLATFORM".into()]);
    assert!(
        admit(&metadata).is_err(),
        "DT_NEEDED tokens are not literal SONAMEs"
    );
    metadata.needed_libraries = ordinary_needed;
    let foreign = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(foreign.path(), &library).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(foreign.path(), &path).unwrap();
    assert!(
        admit(&metadata).is_err(),
        "a closure entry must not redirect to an ambient library"
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, dynamic_library("libinner.so")).unwrap();
    assert!(
        admit(&metadata).is_err(),
        "a missing transitive library must be refused"
    );
    std::fs::create_dir(payload.join("private")).unwrap();
    std::fs::create_dir(payload.join("empty")).unwrap();
    std::fs::write(payload.join("private/libinner.so"), &library).unwrap();
    metadata.rpaths = vec!["$ORIGIN/absent:$ORIGIN/empty:${ORIGIN}/private".into()];
    assert_eq!(
        admit(&metadata).unwrap().directories,
        vec![
            payload.join("empty"),
            payload.join("private"),
            payload.join("lib")
        ]
    );
    let valid_paths = std::mem::replace(&mut metadata.rpaths, vec!["$ORIGIN/../absent".into()]);
    assert!(
        admit(&metadata).is_err(),
        "an absent search directory cannot grant the surrounding store namespace"
    );
    metadata.rpaths.clear();
    metadata.runpaths = valid_paths;
    assert!(
        admit(&metadata).is_err(),
        "RUNPATH is not inherited by a dependent library"
    );
}

fn dynamic_library(needed: &str) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x802 + needed.len()];
    bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
    bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
    bytes[0x20..0x28].copy_from_slice(&64u64.to_le_bytes());
    bytes[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
    bytes[0x38..0x3a].copy_from_slice(&2u16.to_le_bytes());
    for (index, kind, offset, size) in [(0, 1u32, 0u64, bytes.len() as u64), (1, 2, 0x100, 48)] {
        let base = 64 + index * 56;
        bytes[base..base + 4].copy_from_slice(&kind.to_le_bytes());
        bytes[base + 8..base + 16].copy_from_slice(&offset.to_le_bytes());
        bytes[base + 32..base + 40].copy_from_slice(&size.to_le_bytes());
    }
    bytes[0x100..0x108].copy_from_slice(&5u64.to_le_bytes());
    bytes[0x108..0x110].copy_from_slice(&0x800u64.to_le_bytes());
    bytes[0x110..0x118].copy_from_slice(&1u64.to_le_bytes());
    bytes[0x118..0x120].copy_from_slice(&1u64.to_le_bytes());
    bytes[0x801..0x801 + needed.len()].copy_from_slice(needed.as_bytes());
    bytes
}
