//! Independent Conda relocation. Binary suffixes shift inside each C string;
//! zero padding belongs at its end. Expected bytes stream into a comparator or
//! a private native-signing reconstruction, never a patched cache hardlink.
use memchr::{memchr, memmem::Finder};
use std::borrow::Cow;
use std::io::{self, Read, Write};

fn text(
    mut source: &[u8],
    output: &mut impl Write,
    finder: &Finder<'_>,
    old: &[u8],
    new: &[u8],
) -> io::Result<usize> {
    let mut written = 0;
    while let Some(index) = finder.find(source) {
        output.write_all(&source[..index])?;
        output.write_all(new)?;
        written += index + new.len();
        source = &source[index + old.len()..];
    }
    output.write_all(source)?;
    Ok(written + source.len())
}

fn interpreter(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("#!")?.trim_start_matches(' ');
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        if bytes[end..].starts_with(b"\\ ") {
            end += 2;
        } else if matches!(bytes[end], b' ' | b'\n' | b'\r' | b'\t') {
            break;
        } else {
            end += 1;
        }
    }
    Some((&rest[..end], &rest[end..]))
}

fn python(name: &str) -> bool {
    let Some(version) = name.strip_prefix("python") else {
        return false;
    };
    if version.is_empty() {
        return true;
    }
    let mut components = version.split('.');
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    components.next().is_some_and(digits)
        && components.next().is_none_or(digits)
        && components.next().is_none()
}

fn shebang_wrapper(line: &str) -> Result<Cow<'_, str>, String> {
    let Some((path, arguments)) = interpreter(line) else {
        return Ok(Cow::Borrowed(line));
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    if python(name) {
        Ok(Cow::Owned(format!(
            "#!/bin/sh\n'''exec' \"{path}\"{arguments} \"$0\" \"$@\" #'''"
        )))
    } else if !arguments.trim().is_empty() {
        Err(format!(
            "non-Python interpreter {name:?} has arguments that cannot be preserved by env shebang relocation"
        ))
    } else {
        Ok(Cow::Owned(format!("#!/usr/bin/env {name}")))
    }
}

fn shebang<'a>(
    line: &'a str,
    old: &str,
    new: &str,
    platform: &str,
) -> Result<Cow<'a, str>, String> {
    // The pinned installer uses these Unix shebang limits, including its
    // conservative Linux limit even on kernels that accept longer headers.
    let limit = match platform {
        "linux-64" | "linux-aarch64" => 127,
        "osx-64" | "osx-arm64" => 512,
        _ => return Err("unsupported Conda relocation platform".into()),
    };
    if new.contains(' ') {
        return Ok(if line.contains(old) {
            Cow::Owned(shebang_wrapper(line)?.replace(old, new))
        } else {
            Cow::Borrowed(line)
        });
    }
    let replaced = if line.contains(old) {
        Cow::Owned(line.replace(old, new))
    } else {
        Cow::Borrowed(line)
    };
    if replaced.len() <= limit {
        return Ok(replaced);
    }
    Ok(match shebang_wrapper(&replaced)? {
        Cow::Owned(wrapper) => Cow::Owned(wrapper),
        Cow::Borrowed(_) => replaced,
    })
}

pub(super) fn emit(
    mut source: &[u8],
    output: &mut impl Write,
    placeholder: &str,
    prefix: &str,
    binary: bool,
    platform: &str,
) -> Result<(), String> {
    let old = placeholder.as_bytes();
    let new = prefix.as_bytes();
    if old.is_empty() || old.contains(&0) {
        return Err("invalid prefix placeholder".into());
    }
    let finder = Finder::new(old);
    let io_error = |error: io::Error| error.to_string();
    if binary {
        if new.len() > old.len() {
            return Err("binary prefix exceeds its placeholder".into());
        }
        while let Some(start) = finder.find(source) {
            output.write_all(&source[..start]).map_err(io_error)?;
            let after_prefix = start + old.len();
            let end = after_prefix
                + memchr(0, &source[after_prefix..]).unwrap_or(source.len() - after_prefix);
            let written = text(&source[start..end], output, &finder, old, new).map_err(io_error)?;
            let mut padding = end - start - written;
            let zeros = [0; 4096];
            while padding != 0 {
                let count = padding.min(zeros.len());
                output.write_all(&zeros[..count]).map_err(io_error)?;
                padding -= count;
            }
            source = &source[end..];
        }
        output.write_all(source).map_err(io_error)?;
    } else {
        if source.starts_with(b"#!") {
            let end = memchr(b'\n', source)
                .ok_or_else(|| "unterminated interpreter header".to_string())?;
            // Bound the only formatted part before replacement can allocate.
            // Payload text after the interpreter line remains fully streaming.
            if end > super::archive::MAX_PATH_BYTES {
                return Err("interpreter header exceeds its byte bound".into());
            }
            let header = std::str::from_utf8(&source[..end])
                .map_err(|_| "interpreter header is not UTF-8".to_string())?;
            output
                .write_all(shebang(header, placeholder, prefix, platform)?.as_bytes())
                .map_err(io_error)?;
            source = &source[end..];
        }
        text(source, output, &finder, old, new).map_err(io_error)?;
    }
    Ok(())
}

pub(super) struct Comparison<R>(pub R);
impl<R: Read> Write for Comparison<R> {
    fn write(&mut self, expected: &[u8]) -> io::Result<usize> {
        let mut buffer = [0; 64 * 1024];
        for chunk in expected.chunks(buffer.len()) {
            self.0.read_exact(&mut buffer[..chunk.len()])?;
            if buffer[..chunk.len()] != *chunk {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bytes differ from the independently relocated original",
                ));
            }
        }
        Ok(expected.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn verify(
    source: &[u8],
    actual: impl Read,
    placeholder: &str,
    prefix: &str,
    binary: bool,
    platform: &str,
) -> Result<(), String> {
    let mut comparison = Comparison(actual);
    emit(
        source,
        &mut comparison,
        placeholder,
        prefix,
        binary,
        platform,
    )?;
    let mut extra = [0];
    if comparison
        .0
        .read(&mut extra)
        .map_err(|error| error.to_string())?
        == 0
    {
        Ok(())
    } else {
        Err("relocated file has extra bytes".into())
    }
}

#[cfg(test)]
mod tests {
    use super::verify;

    #[test]
    fn binary_relocation_preserves_suffixes_and_pads_after_the_whole_cstring() {
        let original = b"header\0/long-prefix/lib:/long-prefix/bin\0tail";
        let mut relocated = b"header\0/x/lib:/x/bin".to_vec();
        relocated.resize(original.len() - b"\0tail".len(), 0);
        relocated.extend_from_slice(b"\0tail");
        verify(
            original,
            relocated.as_slice(),
            "/long-prefix",
            "/x",
            true,
            "linux-64",
        )
        .unwrap();
        let gap = [0; "/long-prefix".len() - "/x".len()];
        let prematurely_padded = [
            b"header\0/x".as_slice(),
            &gap,
            b"/lib:/x",
            &gap,
            b"/bin\0tail",
        ]
        .concat();
        assert!(
            verify(
                original,
                prematurely_padded.as_slice(),
                "/long-prefix",
                "/x",
                true,
                "linux-64"
            )
            .is_err()
        );
        let mut corrupt = relocated;
        let padding = corrupt.iter().rposition(|byte| *byte == 0).unwrap() - 1;
        corrupt[padding] = b'x';
        assert!(
            verify(
                original,
                corrupt.as_slice(),
                "/long-prefix",
                "/x",
                true,
                "linux-64"
            )
            .is_err()
        );
        assert!(
            verify(
                original,
                original.as_slice(),
                "/long-prefix",
                "/prefix-that-does-not-fit",
                true,
                "linux-64"
            )
            .is_err()
        );
    }

    #[test]
    fn long_or_spaced_python_headers_keep_the_locked_interpreter_path() {
        let prefix = format!("/{}", "a".repeat(140));
        let original = b"#!/placeholder/bin/python3.12 -s\nprint('ok')\n";
        for prefix in [prefix.as_str(), "/with space"] {
            let relocated = format!(
                "#!/bin/sh\n'''exec' \"{prefix}/bin/python3.12\" -s \"$0\" \"$@\" #'''\nprint('ok')\n"
            );
            verify(
                original,
                relocated.as_bytes(),
                "/placeholder",
                prefix,
                false,
                "linux-64",
            )
            .unwrap();
            let substituted = relocated.replace("/bin/python3.12", "/bin/other");
            assert!(
                verify(
                    original,
                    substituted.as_bytes(),
                    "/placeholder",
                    prefix,
                    false,
                    "linux-64"
                )
                .is_err()
            );
        }
    }

    #[test]
    fn non_python_interpreter_arguments_cannot_become_one_env_token() {
        let long_prefix = format!("/{}", "a".repeat(140));
        for (prefix, platform) in [
            (long_prefix.as_str(), "linux-64"),
            ("/with space", "osx-arm64"),
        ] {
            let mut output = Vec::new();
            assert!(
                super::emit(
                    b"#!/placeholder/bin/perl -w\nprint 'ok';\n",
                    &mut output,
                    "/placeholder",
                    prefix,
                    false,
                    platform,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn oversized_interpreter_header_is_refused_before_formatted_replacement() {
        let source = format!(
            "#!/{}/python\n",
            "x".repeat(super::super::archive::MAX_PATH_BYTES)
        );
        assert!(
            super::emit(
                source.as_bytes(),
                &mut std::io::sink(),
                "/x",
                "/final prefix",
                false,
                "linux-64"
            )
            .is_err()
        );
    }
}
