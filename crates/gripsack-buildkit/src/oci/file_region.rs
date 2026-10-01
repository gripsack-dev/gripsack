//! Executable parsers see one tar member, not the surrounding layer. Positioned
//! reads leave the tar iterator's shared file offset untouched.
use std::{fs::File, io::{self, Read, Seek, SeekFrom}, os::unix::fs::FileExt};

pub(super) struct FileRegion<'a> {
    file: &'a File,
    start: u64,
    size: u64,
    position: u64,
}
impl<'a> FileRegion<'a> {
    pub fn new(file: &'a File, start: u64, size: u64) -> io::Result<Self> {
        let length = file.metadata()?.len();
        if start.checked_add(size).is_none_or(|end| end > length) {
            return Err(io::Error::new(io::ErrorKind::InvalidData,"executable member exceeds its layer"));
        }
        Ok(Self { file,start,size,position:0 })
    }
}
impl Read for FileRegion<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.size - self.position;
        let count = usize::try_from(remaining.min(buffer.len() as u64)).expect("bounded by buffer length");
        let count = self.file.read_at(&mut buffer[..count],self.start + self.position)?;
        self.position += count as u64;
        Ok(count)
    }
}
impl Seek for FileRegion<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = match position {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.size) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };
        if next < 0 || next > i128::from(self.size) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,"executable seek escapes its member"));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}
