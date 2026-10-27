//! Lexical admission; realization additionally resolves captured capabilities.

pub(in crate::sema) fn admissible_repo_file(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

pub(in crate::sema) fn admissible_destination(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix('/')) else {
        return false;
    };
    !rest.is_empty()
        && !rest.ends_with('/')
        && !rest.contains('\0')
        && rest
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// HH:MM local time, 00–23 : 00–59, without slicing untrusted UTF-8.
pub(in crate::sema) fn valid_local_time(time: &str) -> bool {
    let [h1, h2, b':', m1, m2] = *time.as_bytes() else {
        return false;
    };
    h1.is_ascii_digit()
        && h2.is_ascii_digit()
        && m1.is_ascii_digit()
        && m2.is_ascii_digit()
        && (h1 - b'0') * 10 + h2 - b'0' < 24
        && (m1 - b'0') * 10 + m2 - b'0' < 60
}
