//! Bottle relocation with a fixed scan buffer, not a whole-binary allocation.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

#[cfg(unix)]
pub(crate) fn pour(root: &Path) -> io::Result<()> {
    const PLACEHOLDER: &[u8] = b"@@HOMEBREW_PREFIX@@/lib/ld.so";
    let loader: &[u8] = if cfg!(target_arch = "aarch64") {
        b"/lib/ld-linux-aarch64.so.1"
    } else {
        b"/lib64/ld-linux-x86-64.so.2"
    };
    let mut replacement = [0; PLACEHOLDER.len()];
    replacement[..loader.len()].copy_from_slice(loader);
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                patch(&entry.path(), PLACEHOLDER, &replacement)?;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn patch(path: &Path, needle: &[u8], replacement: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut input = std::fs::File::open(path)?;
    let original_mode = input.metadata()?.permissions().mode();
    let mut output = None;
    let mut buffer = [0; 64 * 1024 + 64];
    if needle.len() > 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bottle placeholder too long",
        ));
    }
    let mut retained = 0;
    let mut start = 0u64;
    loop {
        let read = match input.read(&mut buffer[retained..]) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        let available = retained + read;
        if available >= needle.len() {
            for offset in 0..=available - needle.len() {
                if &buffer[offset..offset + needle.len()] != needle {
                    continue;
                }
                if output.is_none() {
                    std::fs::set_permissions(
                        path,
                        std::fs::Permissions::from_mode(original_mode | 0o200),
                    )?;
                    output = Some(std::fs::OpenOptions::new().write(true).open(path)?);
                }
                let output = output.as_mut().expect("opened for patch");
                output.seek(SeekFrom::Start(start + offset as u64))?;
                output.write_all(replacement)?;
            }
        }
        if read == 0 {
            break;
        }
        retained = available.min(needle.len() - 1);
        buffer.copy_within(available - retained..available, 0);
        start += (available - retained) as u64;
    }
    if output.is_some() {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(original_mode))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn pour(_root: &Path) -> io::Result<()> {
    Ok(())
}
