//! Bounded GNU/PAX metadata admission. tar's default iterator buffers arbitrary
//! long-name/PAX entries; the archive reader uses raw entries and owns these caps.
use super::{invalid, invalid_error};
use std::io::{self, Read};

const MAX_EXTENSION_BYTES: u64 = 32 * 1024;
#[derive(Default)]
pub(super) struct ExtendedHeader {
    pub path: Option<String>,
    pub link: Option<String>,
    size: Option<u64>,
    pending: bool,
}
impl ExtendedHeader {
    pub fn read<R: Read>(&mut self, member: &mut tar::Entry<'_, R>) -> io::Result<bool> {
        let kind = member.header().entry_type();
        if !kind.is_gnu_longname() && !kind.is_gnu_longlink() && !kind.is_pax_local_extensions() {
            if kind.is_pax_global_extensions() {
                return Err(invalid("global PAX metadata is unsupported"));
            }
            if self.size.is_some_and(|size| size != member.size()) {
                return Err(invalid(
                    "PAX size differs from tar header (unsupported semantics)",
                ));
            }
            return Ok(false);
        }
        if member.size() > MAX_EXTENSION_BYTES {
            return Err(invalid("tar extension exceeds its byte bound"));
        }
        let mut bytes = Vec::new();
        member
            .take(MAX_EXTENSION_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_EXTENSION_BYTES {
            return Err(invalid("oversized tar extension"));
        }
        let text = std::str::from_utf8(&bytes).map_err(invalid_error)?;
        self.pending = true;
        if kind.is_gnu_longname() {
            set(&mut self.path, text.trim_end_matches('\0'))?;
        } else if kind.is_gnu_longlink() {
            set(&mut self.link, text.trim_end_matches('\0'))?;
        } else {
            let mut remaining = text;
            while !remaining.is_empty() {
                let (length, _) = remaining
                    .split_once(' ')
                    .ok_or_else(|| invalid("invalid PAX record"))?;
                let length: usize = length.parse().map_err(invalid_error)?;
                let record = remaining
                    .get(..length)
                    .ok_or_else(|| invalid("truncated PAX record"))?;
                let (_, field) = record
                    .split_once(' ')
                    .ok_or_else(|| invalid("invalid PAX length"))?;
                let field = field
                    .strip_suffix('\n')
                    .ok_or_else(|| invalid("unterminated PAX record"))?;
                let (key, value) = field
                    .split_once('=')
                    .ok_or_else(|| invalid("invalid PAX field"))?;
                match key {
                    "path" => set(&mut self.path, value)?,
                    "linkpath" => set(&mut self.link, value)?,
                    "size" if self.size.is_none() => {
                        self.size = Some(value.parse().map_err(invalid_error)?)
                    }
                    // Times and ownership are not install authority: the prefix
                    // is privately owned and normalized independently.
                    "mtime" | "atime" | "ctime" | "uid" | "gid" | "uname" | "gname" => {}
                    _ => return Err(invalid(format!("unsupported PAX metadata {key:?}"))),
                }
                remaining = &remaining[length..];
            }
        }
        Ok(true)
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
}
fn set(slot: &mut Option<String>, value: &str) -> io::Result<()> {
    if slot.replace(value.into()).is_some() {
        return Err(invalid("duplicate extended tar name"));
    }
    Ok(())
}
