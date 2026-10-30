//! Historical Mach-O relocation detection. Native signing is retired; callers
//! reject artifacts requiring it instead of executing a platform-specific tool.
use super::inventory::PrefixPatch;

pub(super) fn is_macho(source: &[u8]) -> bool {
    source.get(..4).is_some_and(|magic| {
        matches!(
            magic,
            [0xca, 0xfe, 0xba, 0xbe]
                | [0xbe, 0xba, 0xfe, 0xca]
                | [0xfe, 0xed, 0xfa, 0xce]
                | [0xce, 0xfa, 0xed, 0xfe]
                | [0xfe, 0xed, 0xfa, 0xcf]
                | [0xcf, 0xfa, 0xed, 0xfe]
        )
    })
}

pub(super) fn required(source: &[u8], mode: u32, patch: &PrefixPatch, platform: &str) -> bool {
    platform.starts_with("osx-")
        && patch.binary
        && (is_macho(source) || mode & 0o111 != 0)
        && memchr::memmem::find(source, patch.placeholder.as_bytes()).is_some()
}
