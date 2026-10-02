//! Mach-O metadata: single-architecture images only (a fat/universal
//! archive is an explicitly unsupported shape), CPU type, filetype and the
//! bounded LC_LOAD_DYLIB/LC_RPATH install-name inventory.
use super::{
    Endian, ExecutableArch, ExecutableFormat, ExecutableMetadata, Interpreter, LayoutError,
    MAX_DYNAMIC_BYTES, MAX_LOAD_COMMANDS, ObjectKind, WordClass, field, read_exact_at,
};
use std::{
    ffi::OsString,
    io::{Read, Seek, SeekFrom},
    os::unix::ffi::OsStringExt,
};

pub(super) fn is_macho(header: &[u8]) -> bool {
    let magics: [[u8; 4]; 4] = [
        [0xFE, 0xED, 0xFA, 0xCE],
        [0xFE, 0xED, 0xFA, 0xCF],
        [0xCE, 0xFA, 0xED, 0xFE],
        [0xCF, 0xFA, 0xED, 0xFE],
    ];
    header.len() >= 4 && magics.iter().any(|magic| header.starts_with(magic))
}

pub(super) fn macho(
    source: &mut (impl Read + Seek),
    header: &[u8],
) -> Result<ExecutableMetadata, LayoutError> {
    let (endian, sixty_four) = match &header[..4] {
        [0xFE, 0xED, 0xFA, 0xCE] => (Endian::Big, false),
        [0xFE, 0xED, 0xFA, 0xCF] => (Endian::Big, true),
        [0xCE, 0xFA, 0xED, 0xFE] => (Endian::Little, false),
        _ => (Endian::Little, true),
    };
    let cpu = endian.u32(field(header, 4, 4)?);
    let arch = match cpu {
        0x0100_0007 => ExecutableArch::X86_64,
        0x0100_000C => ExecutableArch::Aarch64,
        _ => ExecutableArch::Other,
    };
    let object = match endian.u32(field(header, 12, 4)?) {
        0x2 => ObjectKind::Executable,
        0x6..=0x9 => ObjectKind::SharedObject,
        0x1 => ObjectKind::Relocatable,
        _ => ObjectKind::Other,
    };
    let command_count = endian.u32(field(header, 16, 4)?);
    if command_count > MAX_LOAD_COMMANDS {
        return Err(LayoutError::Malformed("load command count exceeds bound"));
    }
    let mut cursor: u64 = if sixty_four { 32 } else { 28 };
    let command_bytes = u64::from(endian.u32(field(header, 20, 4)?));
    if command_bytes > MAX_DYNAMIC_BYTES {
        return Err(LayoutError::Malformed("load command bytes exceed bound"));
    }
    let commands_end = cursor + command_bytes;
    let file_end = source
        .seek(SeekFrom::End(0))
        .map_err(|error| LayoutError::Io(error.kind()))?;
    if commands_end > file_end {
        return Err(LayoutError::Truncated);
    }
    let mut metadata = ExecutableMetadata {
        format: Some(ExecutableFormat::MachO),
        class: Some(if sixty_four {
            WordClass::SixtyFour
        } else {
            WordClass::ThirtyTwo
        }),
        arch: Some(arch),
        endianness: Some(endian.as_endianness()),
        object: Some(object),
        has_entry_point: object == ObjectKind::Executable,
        ..ExecutableMetadata::default()
    };
    const LC_LOAD_DYLIB: u32 = 0xC;
    const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
    const LC_REEXPORT_DYLIB: u32 = 0x8000_001F;
    const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
    const LC_LOAD_DYLINKER: u32 = 0xE;
    const LC_RPATH: u32 = 0x8000_001C;
    for _ in 0..command_count {
        if cursor + 8 > commands_end {
            return Err(LayoutError::Malformed(
                "load command count exceeds declared bytes",
            ));
        }
        let mut command = [0u8; 8];
        read_exact_at(source, cursor, &mut command)?;
        let kind = endian.u32(&command[0..4]);
        let size = endian.u32(&command[4..8]);
        if size < 8 || size % (if sixty_four { 8 } else { 4 }) != 0 {
            return Err(LayoutError::Malformed(
                "invalid load command size/alignment",
            ));
        }
        let body = u64::from(size) - 8;
        let next = cursor
            .checked_add(u64::from(size))
            .ok_or(LayoutError::OutOfBounds)?;
        if next > commands_end {
            return Err(LayoutError::Malformed(
                "load command exceeds declared bytes",
            ));
        }
        if matches!(
            kind,
            LC_LOAD_DYLIB
                | LC_LOAD_WEAK_DYLIB
                | LC_REEXPORT_DYLIB
                | LC_LOAD_UPWARD_DYLIB
                | LC_RPATH
                | LC_LOAD_DYLINKER
        ) {
            let mut body_bytes = vec![
                0u8;
                usize::try_from(body.min(MAX_DYNAMIC_BYTES))
                    .map_err(|_| LayoutError::OutOfBounds)?
            ];
            read_exact_at(source, cursor + 8, &mut body_bytes)?;
            // dylib_command/rpath_command: the name offset counts from the
            // start of the full command, i.e. 8 bytes into the body view.
            let name_offset = u64::from(endian.u32(field(&body_bytes, 0, 4)?));
            let minimum = if matches!(kind, LC_RPATH | LC_LOAD_DYLINKER) {
                12
            } else {
                24
            };
            if name_offset < minimum
                || name_offset >= u64::from(size)
                || name_offset - 8 >= body_bytes.len() as u64
            {
                return Err(LayoutError::Malformed("name offset outside load command"));
            }
            let start = (name_offset - 8) as usize;
            let tail = &body_bytes[start..];
            let end = tail
                .iter()
                .position(|byte| *byte == 0)
                .ok_or(LayoutError::Malformed("unterminated load command name"))?;
            let name = OsString::from_vec(tail[..end].to_vec());
            if kind == LC_RPATH {
                metadata.runpaths.push(name);
            } else if kind == LC_LOAD_DYLINKER {
                if metadata
                    .interpreter
                    .replace(Interpreter::Loader(name))
                    .is_some()
                {
                    return Err(LayoutError::Malformed("multiple Mach-O dynamic loaders"));
                }
            } else {
                metadata.needed_libraries.push(name);
            }
        }
        cursor = next;
    }
    if cursor != commands_end {
        return Err(LayoutError::Malformed(
            "load commands do not fill declared bytes",
        ));
    }
    Ok(metadata)
}
