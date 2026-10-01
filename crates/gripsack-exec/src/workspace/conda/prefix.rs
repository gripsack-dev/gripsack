//! Independent Conda relocation admission. Binary placeholders occur inside
//! C strings: suffix bytes shift left and padding belongs at the string's end,
//! never between the replaced prefix and its suffix. No patched payload copy.
use memchr::{memchr, memmem::Finder};
use std::borrow::Cow;

fn consume(actual: &mut &[u8], expected: &[u8]) -> Result<(), String> {
    *actual = actual.strip_prefix(expected)
        .ok_or_else(|| "bytes differ from the independently relocated original".to_string())?;
    Ok(())
}

fn text(mut source: &[u8], actual: &mut &[u8], finder: &Finder<'_>, old: &[u8], new: &[u8]) -> Result<(), String> {
    while let Some(index) = finder.find(source) {
        consume(actual, &source[..index])?;
        consume(actual, new)?;
        source = &source[index + old.len()..];
    }
    consume(actual, source)
}

fn interpreter(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("#!")?.trim_start_matches(' ');
    if !rest.starts_with('/') { return None; }
    let bytes = rest.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        if bytes[end..].starts_with(b"\\ ") { end += 2; }
        else if matches!(bytes[end], b' ' | b'\n' | b'\r' | b'\t') { break; }
        else { end += 1; }
    }
    Some((&rest[..end], &rest[end..]))
}

fn python(name: &str) -> bool {
    let Some(version) = name.strip_prefix("python") else { return false; };
    if version.is_empty() { return true; }
    let mut components = version.split('.');
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    components.next().is_some_and(digits)
        && components.next().is_none_or(digits)
        && components.next().is_none()
}

fn shebang_wrapper(line: &str) -> Cow<'_, str> {
    let Some((path, arguments)) = interpreter(line) else { return Cow::Borrowed(line); };
    let name = path.rsplit('/').next().unwrap_or(path);
    if python(name) {
        Cow::Owned(format!("#!/bin/sh\n'''exec' \"{path}\"{arguments} \"$0\" \"$@\" #'''"))
    } else {
        Cow::Owned(format!("#!/usr/bin/env {name}{arguments}"))
    }
}

fn shebang<'a>(line: &'a str, old: &str, new: &str, platform: &str) -> Result<Cow<'a, str>, String> {
    // The pinned installer uses these Unix shebang limits, including its
    // conservative Linux limit even on kernels that accept longer headers.
    let limit = match platform {
        "linux-64" | "linux-aarch64" => 127,
        "osx-64" | "osx-arm64" => 512,
        _ => return Err("unsupported Conda relocation platform".into()),
    };
    if new.contains(' ') {
        return Ok(if line.contains(old) {
            Cow::Owned(shebang_wrapper(line).replace(old, new))
        } else { Cow::Borrowed(line) });
    }
    let replaced = if line.contains(old) { Cow::Owned(line.replace(old, new)) } else { Cow::Borrowed(line) };
    if replaced.len() <= limit { return Ok(replaced); }
    Ok(match shebang_wrapper(&replaced) {
        Cow::Owned(wrapper) => Cow::Owned(wrapper),
        Cow::Borrowed(_) => replaced,
    })
}

pub(super) fn verify(
    mut source: &[u8],
    mut actual: &[u8],
    placeholder: &str,
    prefix: &str,
    binary: bool,
    platform: &str,
) -> Result<(), String> {
    let old = placeholder.as_bytes();
    let new = prefix.as_bytes();
    if old.is_empty() || old.contains(&0) { return Err("invalid prefix placeholder".into()); }
    let finder = Finder::new(old);
    if binary {
        if new.len() > old.len() { return Err("binary prefix exceeds its placeholder".into()); }
        while let Some(start) = finder.find(source) {
            consume(&mut actual, &source[..start])?;
            let after_prefix = start + old.len();
            let end = after_prefix + memchr(0, &source[after_prefix..]).unwrap_or(source.len() - after_prefix);
            let before = actual.len();
            text(&source[start..end], &mut actual, &finder, old, new)?;
            let padding = end - start - (before - actual.len());
            let zeros = actual.get(..padding).ok_or_else(|| "truncated binary padding".to_string())?;
            if zeros.iter().any(|byte| *byte != 0) { return Err("binary relocation padding is not zero".into()); }
            actual = &actual[padding..];
            source = &source[end..];
        }
        consume(&mut actual, source)?;
    } else {
        if source.starts_with(b"#!") {
            let end = memchr(b'\n', source).ok_or_else(|| "unterminated interpreter header".to_string())?;
            let header = std::str::from_utf8(&source[..end]).map_err(|_| "interpreter header is not UTF-8".to_string())?;
            consume(&mut actual, shebang(header, placeholder, prefix, platform)?.as_bytes())?;
            source = &source[end..];
        }
        text(source, &mut actual, &finder, old, new)?;
    }
    if actual.is_empty() { Ok(()) } else { Err("relocated file has extra bytes".into()) }
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
        verify(original, &relocated, "/long-prefix", "/x", true, "linux-64").unwrap();
        let wrong = original.to_vec().windows(1).flatten().copied().collect::<Vec<_>>();
        assert!(verify(original, &wrong, "/long-prefix", "/x", true, "linux-64").is_err());
        let mut corrupt = relocated;
        let padding = corrupt.iter().rposition(|byte| *byte == 0).unwrap() - 1;
        corrupt[padding] = b'x';
        assert!(verify(original, &corrupt, "/long-prefix", "/x", true, "linux-64").is_err());
        assert!(verify(original, original, "/long-prefix", "/prefix-that-does-not-fit", true, "linux-64").is_err());
    }

    #[test]
    fn long_or_spaced_python_headers_keep_the_locked_interpreter_path() {
        let prefix = format!("/{}", "a".repeat(140));
        let original = b"#!/placeholder/bin/python3.12 -s\nprint('ok')\n";
        for prefix in [prefix.as_str(), "/with space"] {
            let relocated = format!("#!/bin/sh\n'''exec' \"{prefix}/bin/python3.12\" -s \"$0\" \"$@\" #'''\nprint('ok')\n");
            verify(original, relocated.as_bytes(), "/placeholder", prefix, false, "linux-64").unwrap();
            let substituted = relocated.replace("/bin/python3.12", "/bin/other");
            assert!(verify(original, substituted.as_bytes(), "/placeholder", prefix, false, "linux-64").is_err());
        }
    }
}
