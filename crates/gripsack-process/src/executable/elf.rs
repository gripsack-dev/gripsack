//! ELF metadata: class, byte order, machine, object kind, entry point,
//! PT_INTERP loader and the bounded DT_NEEDED/RPATH/RUNPATH inventory.
//! String-table addresses resolve through PT_LOAD vaddr windows with
//! checked arithmetic only.
use super::{
    Endian, ExecutableArch, ExecutableFormat, ExecutableMetadata, Interpreter, LayoutError,
    MAX_DYNAMIC_BYTES, MAX_PROGRAM_HEADERS, MAX_STRING_BYTES, ObjectKind, WordClass, field,
    read_exact_at, read_up_to,
};
use std::{
    ffi::OsString,
    io::{Read, Seek, SeekFrom},
    os::unix::ffi::OsStringExt,
};

fn arch(machine: u16) -> ExecutableArch {
    match machine {
        62 => ExecutableArch::X86_64,
        183 => ExecutableArch::Aarch64,
        _ => ExecutableArch::Other,
    }
}

pub(super) fn elf(
    source: &mut (impl Read + Seek),
    header: &[u8],
) -> Result<ExecutableMetadata, LayoutError> {
    if header.len() < 64 {
        return Err(LayoutError::Truncated);
    }
    let class = match header[4] {
        1 => WordClass::ThirtyTwo,
        2 => WordClass::SixtyFour,
        _ => return Err(LayoutError::Malformed("unknown ELF class")),
    };
    let endian = match header[5] {
        1 => Endian::Little,
        2 => Endian::Big,
        _ => return Err(LayoutError::Malformed("unknown ELF byte order")),
    };
    let object = match endian.u16(field(header, 16, 2)?) {
        1 => ObjectKind::Relocatable,
        2 => ObjectKind::Executable,
        3 => ObjectKind::SharedObject,
        4 => ObjectKind::Core,
        _ => ObjectKind::Other,
    };
    let machine = endian.u16(field(header, 18, 2)?);
    let entry_point = match class {
        WordClass::ThirtyTwo => u64::from(endian.u32(field(header, 0x18, 4)?)),
        WordClass::SixtyFour => endian.u64(field(header, 0x18, 8)?),
    };
    let (header_offset, entry_offset, count_offset) = match class {
        WordClass::ThirtyTwo => (0x1C, 0x2A, 0x2C),
        WordClass::SixtyFour => (0x20, 0x36, 0x38),
    };
    let table_offset = match class {
        WordClass::ThirtyTwo => u64::from(endian.u32(field(header, header_offset, 4)?)),
        WordClass::SixtyFour => endian.u64(field(header, header_offset, 8)?),
    };
    let entry_size = u64::from(endian.u16(field(header, entry_offset, 2)?));
    let count = u64::from(endian.u16(field(header, count_offset, 2)?));
    if count > MAX_PROGRAM_HEADERS {
        return Err(LayoutError::Malformed("program header count exceeds bound"));
    }
    if count > 0 && entry_size < 8 {
        return Err(LayoutError::Malformed("program header entry too small"));
    }
    let table_bytes = count
        .checked_mul(entry_size)
        .ok_or(LayoutError::OutOfBounds)?;
    let mut table = vec![0u8; usize::try_from(table_bytes).map_err(|_| LayoutError::OutOfBounds)?];
    if table_bytes > 0 {
        read_exact_at(source, table_offset, &mut table)?;
    }
    let mut metadata = ExecutableMetadata {
        format: Some(ExecutableFormat::Elf),
        class: Some(class),
        arch: Some(arch(machine)),
        endianness: Some(endian.as_endianness()),
        object: Some(object),
        has_entry_point: entry_point != 0,
        ..ExecutableMetadata::default()
    };
    let mut dynamic: Option<(u64, u64)> = None;
    // PT_LOAD virtual-address windows, for dynstr vaddr → file offset mapping.
    let mut loads: Vec<(u64, u64, u64)> = Vec::new();
    for index in 0..count {
        let start = usize::try_from(index * entry_size).map_err(|_| LayoutError::OutOfBounds)?;
        let entry = field(&table, start, entry_size as usize)?;
        let p_type = endian.u32(field(entry, 0, 4)?);
        let (p_offset, p_vaddr, p_filesz) = match class {
            WordClass::ThirtyTwo => (
                u64::from(endian.u32(field(entry, 4, 4)?)),
                u64::from(endian.u32(field(entry, 8, 4)?)),
                u64::from(endian.u32(field(entry, 16, 4)?)),
            ),
            WordClass::SixtyFour => (
                endian.u64(field(entry, 8, 8)?),
                endian.u64(field(entry, 16, 8)?),
                endian.u64(field(entry, 32, 8)?),
            ),
        };
        const PT_INTERP: u32 = 3;
        const PT_DYNAMIC: u32 = 2;
        const PT_LOAD: u32 = 1;
        match p_type {
            PT_INTERP => {
                let mut bytes = vec![
                    0u8;
                    usize::try_from(p_filesz.min(MAX_STRING_BYTES))
                        .map_err(|_| LayoutError::OutOfBounds)?
                ];
                read_exact_at(source, p_offset, &mut bytes)?;
                let end = bytes
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(bytes.len());
                metadata.interpreter = Some(Interpreter::Loader(OsString::from_vec(
                    bytes[..end].to_vec(),
                )));
            }
            PT_DYNAMIC => {
                if p_filesz > MAX_DYNAMIC_BYTES {
                    return Err(LayoutError::Malformed("dynamic segment exceeds bound"));
                }
                dynamic = Some((p_offset, p_filesz));
            }
            PT_LOAD => loads.push((p_vaddr, p_offset, p_filesz)),
            _ => {}
        }
    }
    if let Some((offset, size)) = dynamic {
        dynamic_entries(source, offset, size, class, endian, &loads, &mut metadata)?;
    }
    Ok(metadata)
}

fn dynamic_entries(
    source: &mut (impl Read + Seek),
    offset: u64,
    size: u64,
    class: WordClass,
    endian: Endian,
    loads: &[(u64, u64, u64)],
    metadata: &mut ExecutableMetadata,
) -> Result<(), LayoutError> {
    let mut segment = vec![0u8; usize::try_from(size).map_err(|_| LayoutError::OutOfBounds)?];
    read_exact_at(source, offset, &mut segment)?;
    let entry_size = match class {
        WordClass::ThirtyTwo => 8usize,
        WordClass::SixtyFour => 16usize,
    };
    const DT_NULL: i64 = 0;
    const DT_NEEDED: i64 = 1;
    const DT_STRTAB: i64 = 5;
    const DT_RPATH: i64 = 15;
    const DT_RUNPATH: i64 = 29;
    let mut strtab: Option<u64> = None;
    let mut wanted: Vec<(i64, u64)> = Vec::new();
    for chunk in segment.chunks_exact(entry_size) {
        let tag = endian.i64(field(chunk, 0, 8)?);
        let value = match class {
            WordClass::ThirtyTwo => u64::from(endian.u32(field(chunk, 4, 4)?)),
            WordClass::SixtyFour => endian.u64(field(chunk, 8, 8)?),
        };
        match tag {
            DT_NULL => break,
            DT_STRTAB => strtab = Some(value),
            DT_NEEDED | DT_RPATH | DT_RUNPATH => wanted.push((tag, value)),
            _ => {}
        }
    }
    let Some(strtab_vaddr) = strtab else {
        if wanted.is_empty() {
            return Ok(());
        }
        return Err(LayoutError::Malformed(
            "dynamic strings without a string table",
        ));
    };
    let strtab_offset = vaddr_to_offset(loads, strtab_vaddr)?;
    for (tag, name_offset) in wanted {
        let at = strtab_offset
            .checked_add(name_offset)
            .ok_or(LayoutError::OutOfBounds)?;
        let name = read_c_string(source, at)?;
        match tag {
            DT_NEEDED => metadata.needed_libraries.push(name),
            DT_RPATH => metadata.rpaths.push(name),
            _ => metadata.runpaths.push(name),
        }
    }
    Ok(())
}

fn vaddr_to_offset(loads: &[(u64, u64, u64)], vaddr: u64) -> Result<u64, LayoutError> {
    for &(segment_vaddr, segment_offset, segment_size) in loads {
        let end = segment_vaddr
            .checked_add(segment_size)
            .ok_or(LayoutError::OutOfBounds)?;
        if vaddr >= segment_vaddr && vaddr < end {
            return segment_offset
                .checked_add(vaddr - segment_vaddr)
                .ok_or(LayoutError::OutOfBounds);
        }
    }
    Err(LayoutError::OutOfBounds)
}

fn read_c_string(source: &mut (impl Read + Seek), offset: u64) -> Result<OsString, LayoutError> {
    let mut bytes = vec![0u8; MAX_STRING_BYTES as usize];
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| LayoutError::Io(error.kind()))?;
    let read = read_up_to(source, &mut bytes)?;
    let bytes = &bytes[..read];
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(LayoutError::Malformed("unterminated string table entry"))?;
    Ok(OsString::from_vec(bytes[..end].to_vec()))
}
