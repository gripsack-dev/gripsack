//! Bounded executable metadata classification shared by native consumer
//! admission and image-runtime inventory. This reader is pure metadata: no
//! host path resolution, no platform policy and no layout admission decision
//! lives here. ELF read regions and indices are bounded and checked; malformed
//! or truncated ELF fails closed with a typed error, never a panic. Foreign
//! Mach-O magic is identified only so admission can reject it.
mod elf;
mod shebang;

use std::io::{self, Read, Seek};

pub(super) const HEADER_BYTES: u64 = 4096;
pub(super) const MAX_PROGRAM_HEADERS: u64 = 1024;
pub(super) const MAX_DYNAMIC_BYTES: u64 = 64 * 1024;
pub(super) const MAX_STRING_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordClass {
    ThirtyTwo,
    SixtyFour,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutableArch {
    X86_64,
    Aarch64,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutableFormat {
    Elf,
    /// Foreign magic only, retained so Linux inventory rejects these images.
    /// No Mach-O execution metadata or loader plan is supported.
    MachO,
    /// A text executable opened through a `#!` interpreter line.
    Script,
}

/// Byte order of a binary object. The supported native and image targets
/// are little-endian; big-endian objects are reported, never assumed away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endianness {
    Little,
    Big,
}

/// What kind of object the bytes are. An `Executable` or a `SharedObject`
/// with an entry point may launch; libraries without one are only valid as
/// dependency objects, and relocatable/core images launch nowhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Executable,
    SharedObject,
    Relocatable,
    Core,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interpreter {
    /// ELF PT_INTERP — the absolute platform loader.
    Loader(OsString),
    /// A `#!` line: program plus its optional single argument.
    Shebang {
        program: OsString,
        argument: Option<OsString>,
    },
}

/// ELF loader inputs outside the ordinary DT_NEEDED dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfLoaderExtension {
    Audit(OsString),
    DependencyAudit(OsString),
    AuxiliaryFilter(OsString),
    Filter(OsString),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutableMetadata {
    pub format: Option<ExecutableFormat>,
    pub class: Option<WordClass>,
    pub arch: Option<ExecutableArch>,
    /// ELF byte order; `None` for scripts and foreign formats.
    pub endianness: Option<Endianness>,
    /// ELF e_type classification; `None` for scripts and foreign formats.
    pub object: Option<ObjectKind>,
    /// The ELF object carries a program entry point (e_entry != 0).
    /// A shared object without one is a dependency only.
    pub has_entry_point: bool,
    pub interpreter: Option<Interpreter>,
    /// ELF DT_NEEDED names, in link order.
    pub needed_libraries: Vec<OsString>,
    pub rpaths: Vec<OsString>,
    pub runpaths: Vec<OsString>,
    pub elf_loader_extensions: Vec<ElfLoaderExtension>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    UnsupportedFormat,
    Truncated,
    OutOfBounds,
    Malformed(&'static str),
    Io(io::ErrorKind),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat => write!(output, "unrecognized executable format"),
            Self::Truncated => write!(output, "executable is truncated"),
            Self::OutOfBounds => write!(output, "executable structure points outside its bytes"),
            Self::Malformed(detail) => write!(output, "malformed executable: {detail}"),
            Self::Io(kind) => write!(output, "executable read failed ({kind})"),
        }
    }
}
impl std::error::Error for LayoutError {}

/// Classify one executable stream. Reads are incremental and bounded; the
/// stream position at return is unspecified.
pub fn classify(source: &mut (impl Read + Seek)) -> Result<ExecutableMetadata, LayoutError> {
    let mut header = [0u8; HEADER_BYTES as usize];
    let read = read_up_to(source, &mut header)?;
    let header = &header[..read];
    if header.starts_with(b"#!") {
        return shebang::shebang(header);
    }
    if header.starts_with(b"\x7fELF") {
        return elf::elf(source, header);
    }
    // Recognize foreign magic without parsing retired load commands. Keeping
    // the format marker lets Linux image admission reject executable Mach-O
    // files rather than treating them as unrelated unclassified data.
    if [
        [0xfe, 0xed, 0xfa, 0xce],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xcf, 0xfa, 0xed, 0xfe],
    ]
    .iter()
    .any(|magic| header.starts_with(magic))
    {
        return Ok(ExecutableMetadata {
            format: Some(ExecutableFormat::MachO),
            ..ExecutableMetadata::default()
        });
    }
    Err(LayoutError::UnsupportedFormat)
}

pub(super) fn read_up_to(
    source: &mut (impl Read + Seek),
    buffer: &mut [u8],
) -> Result<usize, LayoutError> {
    let mut filled = 0;
    while filled < buffer.len() {
        let count = source
            .read(&mut buffer[filled..])
            .map_err(|error| LayoutError::Io(error.kind()))?;
        if count == 0 {
            break;
        }
        filled += count;
    }
    Ok(filled)
}

pub(super) fn read_exact_at(
    source: &mut (impl Read + Seek),
    offset: u64,
    buffer: &mut [u8],
) -> Result<(), LayoutError> {
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| LayoutError::Io(error.kind()))?;
    let read = read_up_to(source, buffer)?;
    if read != buffer.len() {
        return Err(LayoutError::Truncated);
    }
    Ok(())
}

/// Slice `width` bytes at `offset`, failing closed instead of panicking.
pub(super) fn field(table: &[u8], offset: usize, width: usize) -> Result<&[u8], LayoutError> {
    let end = offset.checked_add(width).ok_or(LayoutError::OutOfBounds)?;
    table.get(offset..end).ok_or(LayoutError::Truncated)
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Endian {
    Little,
    Big,
}
impl Endian {
    pub fn as_endianness(self) -> Endianness {
        match self {
            Self::Little => Endianness::Little,
            Self::Big => Endianness::Big,
        }
    }
    pub fn u16(self, bytes: &[u8]) -> u16 {
        let pair: [u8; 2] = bytes[..2].try_into().expect("sliced u16");
        match self {
            Self::Little => u16::from_le_bytes(pair),
            Self::Big => u16::from_be_bytes(pair),
        }
    }
    pub fn u32(self, bytes: &[u8]) -> u32 {
        let quad: [u8; 4] = bytes[..4].try_into().expect("sliced u32");
        match self {
            Self::Little => u32::from_le_bytes(quad),
            Self::Big => u32::from_be_bytes(quad),
        }
    }
    pub fn u64(self, bytes: &[u8]) -> u64 {
        let oct: [u8; 8] = bytes[..8].try_into().expect("sliced u64");
        match self {
            Self::Little => u64::from_le_bytes(oct),
            Self::Big => u64::from_be_bytes(oct),
        }
    }
    pub fn i64(self, bytes: &[u8]) -> i64 {
        self.u64(bytes) as i64
    }
}

use std::ffi::OsString;
use std::io::SeekFrom;

#[cfg(test)]
mod tests;
