use super::*;
use std::ffi::{OsStr, OsString};
use std::io::Cursor;

fn classify_bytes(bytes: &[u8]) -> Result<ExecutableMetadata, LayoutError> {
    classify(&mut Cursor::new(bytes.to_vec()))
}

fn elf64(phdrs: &[(u32, u64, u64, u64)], dynamic: Option<&[u8]>) -> Vec<u8> {
    // (type, offset, vaddr, filesz) program headers at 0x40; optional
    // dynamic bytes appended after the table.
    let mut bytes = vec![0u8; 0x40];
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2; // 64-bit
    bytes[5] = 1; // little-endian
    bytes[16..18].copy_from_slice(&3u16.to_le_bytes()); // ET_DYN
    bytes[18..20].copy_from_slice(&62u16.to_le_bytes()); // x86_64
    bytes[0x18..0x20].copy_from_slice(&0x400u64.to_le_bytes()); // e_entry
    bytes[0x20..0x28].copy_from_slice(&0x40u64.to_le_bytes());
    bytes[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
    bytes[0x38..0x3A].copy_from_slice(&(phdrs.len() as u16).to_le_bytes());
    for (kind, offset, vaddr, size) in phdrs {
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&vaddr.to_le_bytes());
        bytes.extend_from_slice(&vaddr.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
    }
    if let Some(segment) = dynamic {
        bytes.extend_from_slice(segment);
    }
    bytes
}

#[test]
fn static_elf_has_no_loader_or_libraries() {
    let bytes = elf64(&[(1, 0, 0x1000, 0x40)], None);
    let metadata = classify_bytes(&bytes).unwrap();
    assert_eq!(metadata.format, Some(ExecutableFormat::Elf));
    assert_eq!(metadata.arch, Some(ExecutableArch::X86_64));
    assert_eq!(metadata.endianness, Some(Endianness::Little));
    assert_eq!(metadata.object, Some(ObjectKind::SharedObject));
    assert!(metadata.has_entry_point);
    assert_eq!(metadata.interpreter, None);
    assert!(metadata.needed_libraries.is_empty());
}

#[test]
fn dynamic_elf_reports_loader_and_needed_libraries() {
    // PT_LOAD covers vaddr 0..0x1000 at offset 0; dynamic segment after it.
    // strtab at vaddr 0x800 → offset 0x800.
    let mut dynamic = Vec::new();
    dynamic.extend_from_slice(&5i64.to_le_bytes());
    dynamic.extend_from_slice(&0x800u64.to_le_bytes()); // DT_STRTAB
    dynamic.extend_from_slice(&1i64.to_le_bytes());
    dynamic.extend_from_slice(&1u64.to_le_bytes()); // DT_NEEDED offset 1
    dynamic.extend_from_slice(&15i64.to_le_bytes());
    dynamic.extend_from_slice(&11u64.to_le_bytes()); // DT_RPATH offset 11
    dynamic.extend_from_slice(&0i64.to_le_bytes());
    dynamic.extend_from_slice(&0u64.to_le_bytes()); // DT_NULL
    let dynamic_offset = 0x40 + 56 * 3;
    let mut bytes = elf64(
        &[
            (1, 0, 0, 0x1000),
            (3, 0x900, 0, 28),
            (2, dynamic_offset as u64, 0, dynamic.len() as u64),
        ],
        Some(&dynamic),
    );
    bytes.resize(0x800, 0);
    bytes.extend_from_slice(b"\0libc.so.6\0/opt/lib\0");
    bytes.resize(0x900, 0);
    bytes.extend_from_slice(b"/lib64/ld-linux-x86-64.so.2\0");
    let metadata = classify_bytes(&bytes).unwrap();
    assert_eq!(metadata.needed_libraries, vec![OsString::from("libc.so.6")]);
    assert_eq!(metadata.rpaths, vec![OsString::from("/opt/lib")]);
    assert!(metadata.runpaths.is_empty());
    let Some(Interpreter::Loader(loader)) = &metadata.interpreter else {
        panic!("expected loader: {:?}", metadata.interpreter);
    };
    assert_eq!(loader, &OsString::from("/lib64/ld-linux-x86-64.so.2"));
}

#[test]
fn elf_audit_and_filter_objects_cannot_escape_needed_inventory() {
    let name = OsString::from("outside.so");
    for (tag, expected) in [
        (
            0x6fff_fefb_i64,
            ElfLoaderExtension::DependencyAudit(name.clone()),
        ),
        (0x6fff_fefc, ElfLoaderExtension::Audit(name.clone())),
        (
            0x7fff_fffd,
            ElfLoaderExtension::AuxiliaryFilter(name.clone()),
        ),
        (0x7fff_ffff, ElfLoaderExtension::Filter(name.clone())),
    ] {
        let mut dynamic = Vec::new();
        for (tag, value) in [(5i64, 0x800u64), (tag, 1), (0, 0)] {
            dynamic.extend_from_slice(&tag.to_le_bytes());
            dynamic.extend_from_slice(&value.to_le_bytes());
        }
        let mut bytes = elf64(
            &[
                (1, 0, 0, 0x900),
                (2, 0x40 + 56 * 2, 0, dynamic.len() as u64),
            ],
            Some(&dynamic),
        );
        bytes.resize(0x800, 0);
        bytes.extend_from_slice(b"\0outside.so\0");
        let metadata = classify_bytes(&bytes).unwrap();
        assert!(metadata.needed_libraries.is_empty());
        assert_eq!(metadata.elf_loader_extensions, vec![expected]);
    }
}

#[test]
fn big_endian_and_non_entry_objects_are_reported() {
    let mut bytes = elf64(&[], None);
    bytes[5] = 2; // big-endian
    bytes[16..18].copy_from_slice(&1u16.to_be_bytes()); // ET_REL
    bytes[0x18..0x20].copy_from_slice(&0u64.to_be_bytes());
    let metadata = classify_bytes(&bytes).unwrap();
    assert_eq!(metadata.endianness, Some(Endianness::Big));
    assert_eq!(metadata.object, Some(ObjectKind::Relocatable));
    assert!(!metadata.has_entry_point);
}

#[test]
fn shebang_reports_program_and_argument() {
    let metadata = classify_bytes(b"#!/bin/sh -e\necho hi\n").unwrap();
    assert_eq!(metadata.format, Some(ExecutableFormat::Script));
    let Some(Interpreter::Shebang { program, argument }) = &metadata.interpreter else {
        panic!("expected shebang: {:?}", metadata.interpreter);
    };
    assert_eq!(program, &OsString::from("/bin/sh"));
    assert_eq!(argument.as_deref(), Some(OsStr::new("-e")));
}

#[test]
fn shebang_spacing_matches_kernel_interpreter_and_single_argument_rules() {
    let metadata = classify_bytes(b"#! \t/bin/sh \t-e  argument with spaces \t\n").unwrap();
    assert_eq!(
        metadata.interpreter,
        Some(Interpreter::Shebang {
            program: "/bin/sh".into(),
            argument: Some("-e  argument with spaces".into()),
        })
    );
    // CR is part of the interpreter name on Linux, not whitespace to erase.
    let metadata = classify_bytes(b"#!/bin/sh\r\n").unwrap();
    assert_eq!(
        metadata.interpreter,
        Some(Interpreter::Shebang {
            program: "/bin/sh\r".into(),
            argument: None
        })
    );
}

#[test]
fn truncated_and_unknown_inputs_fail_closed() {
    assert_eq!(classify_bytes(b"\x7fELF\x02"), Err(LayoutError::Truncated));
    assert_eq!(
        classify_bytes(b"plain text"),
        Err(LayoutError::UnsupportedFormat)
    );
    assert!(matches!(
        classify_bytes(b"#!"),
        Err(LayoutError::Malformed(_))
    ));
}
