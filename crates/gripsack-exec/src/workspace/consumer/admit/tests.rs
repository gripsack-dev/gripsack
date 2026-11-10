use super::*;
use gripsack_ir::workspace::{OsVersion, PlatformAbi, PlatformArch, PlatformOs};
use gripsack_process::executable::{
    Endianness, ExecutableArch, ExecutableFormat, Interpreter, ObjectKind, WordClass,
};

#[test]
fn native_host_admission_is_linux_x86_64_only() {
    for (os, arch, accepted) in [
        ("linux", "x86_64", true),
        ("linux", "aarch64", false),
        ("macos", "x86_64", false),
        ("darwin", "aarch64", false),
        ("windows", "x86_64", false),
    ] {
        let facts = gripsack_ir::HostFacts {
            os: os.into(),
            arch: arch.into(),
            ..Default::default()
        };
        assert_eq!(
            HostTarget::from_facts(&facts).is_ok(),
            accepted,
            "{os}/{arch}"
        );
    }
}

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
        archive_root: None,
        selected_root: None,
        identity: serde_json::from_str(&format!("\"{}\"", "0".repeat(64))).unwrap(),
        root: payload.clone(),
        selected_payload: None,
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
    // Path tags alone do not activate a loader, but the static shortcut must
    // still reject incompatible headers and all non-DT_NEEDED loader inputs.
    let static_image = ExecutableMetadata {
        interpreter: None,
        needed_libraries: Vec::new(),
        runpaths: vec!["/unadmitted/static-path".into()],
        ..metadata.clone()
    };
    let admit_static = |image: &ExecutableMetadata| {
        elf::admit(
            image,
            &payload.join("program"),
            elf::ImageOwner {
                package: &package,
                payload: &payload,
                declared_abi: None,
            },
            &RuntimeScope::new(&package, None),
            &host,
            &span,
        )
    };
    for object in [ObjectKind::Executable, ObjectKind::SharedObject] {
        let image = ExecutableMetadata {
            object: Some(object),
            ..static_image.clone()
        };
        let runtime = admit_static(&image).unwrap();
        assert!(runtime.gnu_loader.is_none() && runtime.directories.is_empty());
    }
    for incompatible in [
        ExecutableMetadata {
            class: Some(WordClass::ThirtyTwo),
            ..static_image.clone()
        },
        ExecutableMetadata {
            arch: Some(ExecutableArch::Aarch64),
            ..static_image.clone()
        },
        ExecutableMetadata {
            endianness: Some(Endianness::Big),
            ..static_image.clone()
        },
        ExecutableMetadata {
            object: Some(ObjectKind::Relocatable),
            ..static_image.clone()
        },
        ExecutableMetadata {
            has_entry_point: false,
            ..static_image.clone()
        },
    ] {
        assert!(admit_static(&incompatible).is_err());
    }
    use gripsack_process::executable::ElfLoaderExtension;
    for extension in [
        ElfLoaderExtension::Audit("outside.so".into()),
        ElfLoaderExtension::DependencyAudit("outside.so".into()),
        ElfLoaderExtension::AuxiliaryFilter("outside.so".into()),
        ElfLoaderExtension::Filter("outside.so".into()),
    ] {
        let image = ExecutableMetadata {
            elf_loader_extensions: vec![extension],
            ..static_image.clone()
        };
        assert!(admit_static(&image).is_err());
    }
    let admit = |metadata: &ExecutableMetadata| {
        elf::admit(
            metadata,
            &payload.join("program"),
            elf::ImageOwner {
                package: &package,
                payload: &payload,
                declared_abi: Some(BinaryAbi::Gnu),
            },
            &RuntimeScope::new(&package, None),
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

#[test]
fn imported_environment_values_are_command_scoped_not_profile_exports() {
    let mut plan = EnvironmentPlan::empty();
    plan.entries = vec![
        (
            OsString::from("IMPORTED_ONLY"),
            OsString::from("literal $value"),
        ),
        (OsString::from("SHARED"), OsString::from("publisher")),
        (OsString::from("SHARED"), OsString::from("recipient")),
    ];
    plan.profile_entries_start = 2;
    let contributions = serde_json::to_value(plan.profile_env().unwrap()).unwrap();
    let encoded = serde_json::to_string(&contributions).unwrap();
    assert!(!encoded.contains("IMPORTED_ONLY"));
    assert!(!encoded.contains("publisher"));
    assert!(encoded.contains("recipient"));
    let overlay = EnvironmentOverlay::admit(plan.entries.iter().cloned(), [], []).unwrap();
    assert_eq!(
        overlay.entries()[std::ffi::OsStr::new("SHARED")],
        "recipient"
    );
    assert_eq!(
        overlay.entries()[std::ffi::OsStr::new("IMPORTED_ONLY")],
        "literal $value"
    );
}

fn gnu_host() -> HostTarget {
    HostTarget {
        requirement: TargetRequirement {
            os: TargetOs::Linux,
            arch: TargetArch::X86_64,
            abi: Some(BinaryAbi::Gnu),
            minimum_os: None,
        },
        os: TargetOs::Linux,
        arch: TargetArch::X86_64,
        abi: Some(BinaryAbi::Gnu),
    }
}

fn span() -> Span {
    Span {
        file: "workspace.ts".into(),
        line: 7,
        col: None,
    }
}

fn platform(abi: Option<PlatformAbi>) -> WorkspacePlatform {
    WorkspacePlatform {
        os: PlatformOs::Linux,
        arch: PlatformArch::X86_64,
        abi,
        minimum_os: None,
    }
}

/// plan/0059 step 1: the ABI is a property of the executable's bytes. A
/// declared ABI never compares against the host libc at target admission;
/// ELF admission reads the loader and checks any declaration there.
#[test]
fn declared_abi_is_not_a_host_libc_gate() {
    for declared in [None, Some(PlatformAbi::Gnu), Some(PlatformAbi::Musl)] {
        assert!(
            admit_target(&platform(declared), &gnu_host(), &span(), "environment").is_ok(),
            "{declared:?}"
        );
    }
    let unknown_libc = HostTarget {
        requirement: TargetRequirement {
            abi: None,
            ..gnu_host().requirement
        },
        abi: None,
        ..gnu_host()
    };
    assert!(
        admit_target(
            &platform(Some(PlatformAbi::Musl)),
            &unknown_libc,
            &span(),
            "package"
        )
        .is_ok()
    );
}

#[test]
fn elf_admission_infers_the_abi_from_the_loader() {
    use crate::workspace::artifact::Artifact;
    use gripsack_process::executable::ExecutableMetadata;
    use std::sync::Arc;

    let directory = tempfile::tempdir().unwrap();
    let payload = directory.path().canonicalize().unwrap();
    let package = Package {
        archive_root: None,
        selected_root: None,
        identity: serde_json::from_str(&format!("\"{}\"", "0".repeat(64))).unwrap(),
        root: payload.clone(),
        selected_payload: None,
        producer: Arc::new(Artifact {
            root: payload.clone(),
            payload: payload.clone(),
            tree: serde_json::from_str(&format!("\"{}\"", "0".repeat(64))).unwrap(),
            retention: BTreeSet::new(),
        }),
        commands: BTreeMap::new(),
        runtime: Vec::new(),
        target: platform(None),
        layout: CatalogPackageLayout::Relocatable,
        conda: None,
        host_runtime: None,
    };
    let image = |loader: Option<&str>| ExecutableMetadata {
        format: Some(ExecutableFormat::Elf),
        class: Some(WordClass::SixtyFour),
        arch: Some(ExecutableArch::X86_64),
        endianness: Some(Endianness::Little),
        object: Some(ObjectKind::Executable),
        has_entry_point: true,
        interpreter: loader.map(|loader| Interpreter::Loader(loader.into())),
        ..ExecutableMetadata::default()
    };
    let admit = |metadata: &ExecutableMetadata, declared_abi: Option<BinaryAbi>| {
        elf::admit(
            metadata,
            &payload.join("program"),
            elf::ImageOwner {
                package: &package,
                payload: &payload,
                declared_abi,
            },
            &RuntimeScope::new(&package, None),
            &gnu_host(),
            &span(),
        )
    };
    let gnu = image(Some("/lib64/ld-linux-x86-64.so.2"));
    let musl = image(Some("/lib/ld-musl-x86_64.so.1"));
    let unknown = image(Some("/opt/loader.so"));
    let static_image = image(None);

    // Undeclared: the loader names the ABI; GNU selects the platform loader.
    let runtime = admit(&gnu, None).unwrap();
    assert_eq!(runtime.gnu_loader, Some("/lib64/ld-linux-x86-64.so.2"));
    // A declaration is an assertion checked against the bytes.
    assert!(admit(&gnu, Some(BinaryAbi::Gnu)).is_ok());
    assert!(
        admit(&gnu, Some(BinaryAbi::Musl)).is_err(),
        "a declared ABI that contradicts PT_INTERP is refused"
    );
    // A dynamic musl image needs a musl host loader whatever is declared.
    assert!(admit(&musl, None).is_err());
    assert!(admit(&musl, Some(BinaryAbi::Musl)).is_err());
    assert!(admit(&unknown, None).is_err());
    // Static images carry no loader ABI and run on any host.
    for declared in [None, Some(BinaryAbi::Gnu), Some(BinaryAbi::Musl)] {
        let runtime = admit(&static_image, declared).unwrap();
        assert!(runtime.gnu_loader.is_none() && runtime.directories.is_empty());
    }
}

/// plan/0059 step 1: conda-forge leaves its CDT platform libraries (X11,
/// OpenGL, ALSA) to the host; a prefix needing one is admitted by default.
#[test]
fn conda_platform_libraries_are_host_provided_by_default() {
    use gripsack_ir::workspace_model::PlatformLibraries;
    let system = crate::workspace::conda::SystemRuntime {
        libraries: vec!["libX11.so.6".into(), "libGL.so.1".into()],
        loaders: Vec::new(),
    };
    let policy = PlatformLibraries::HostProvided;
    assert!(platform::admit(&system, policy, &gnu_host(), &span()).is_ok());
    assert_eq!(
        platform::host_provided(&system, policy).collect::<Vec<_>>(),
        ["libX11.so.6", "libGL.so.1"]
    );
    // strict: true restores the closed check.
    let closed = PlatformLibraries::Closed;
    let refused = platform::admit(&system, closed, &gnu_host(), &span()).unwrap_err();
    assert!(refused.to_string().contains("strict"), "{refused}");
    assert_eq!(platform::host_provided(&system, closed).count(), 0);
    // Anything outside the explicit CDT list stays a closure requirement.
    let ambient = crate::workspace::conda::SystemRuntime {
        libraries: vec!["libstdc++.so.6".into()],
        loaders: Vec::new(),
    };
    assert!(platform::admit(&ambient, policy, &gnu_host(), &span()).is_err());
}
