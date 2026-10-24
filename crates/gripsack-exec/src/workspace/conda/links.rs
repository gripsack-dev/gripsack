//! Resolve the complete expected link graph component by component. In
//! particular `link/..` is never lexically collapsed before resolving `link`.
use super::inventory::{Content, ExpectedFile};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) fn directories(
    expected: &BTreeMap<String, ExpectedFile>,
) -> Result<BTreeSet<String>, String> {
    let mut directories = BTreeSet::from(["conda-meta".into()]);
    for (path, file) in expected {
        if matches!(file.content, Content::Directory) {
            directories.insert(path.clone());
        }
        let mut parent = path.as_str();
        while let Some((ancestor, _)) = parent.rsplit_once('/') {
            if expected
                .get(ancestor)
                .is_some_and(|file| !matches!(file.content, Content::Directory))
            {
                return Err(format!(
                    "payload {path:?} traverses a file or symlink owner {ancestor:?}"
                ));
            }
            directories.insert(ancestor.into());
            parent = ancestor;
        }
    }
    Ok(directories)
}

pub(super) enum Entry<'a> {
    Symlink(&'a str),
    Directory,
    File,
}

/// Component-wise lookup shared by installed-link validation and archive
/// lifecycle admission. `follow_final = false` models symlink_metadata:
/// the existence of a final dangling symlink still counts.
pub(super) fn resolve<'a>(
    path: &str,
    prefix: Option<&str>,
    follow_final: bool,
    lookup: impl Fn(&str) -> Option<Entry<'a>>,
) -> Result<bool, String> {
    let mut pending: VecDeque<String> = path.split('/').map(str::to_owned).collect();
    let mut resolved = Vec::new();
    let mut traversals = 0;
    while let Some(component) = pending.pop_front() {
        match component.as_str() {
            "" | "." => continue,
            ".." => {
                if resolved.pop().is_none() {
                    return Err(format!("symlink {path:?} escapes its prefix"));
                }
                continue;
            }
            _ => {}
        }
        resolved.push(component);
        let candidate = resolved.join("/");
        match lookup(&candidate) {
            Some(Entry::Symlink(_)) if pending.is_empty() && !follow_final => return Ok(true),
            Some(Entry::Symlink(target)) => {
                traversals += 1;
                if traversals > 40 {
                    return Err(format!("symlink {path:?} cycles or exceeds 40 traversals"));
                }
                resolved.pop();
                let target = if target.starts_with('/') {
                    resolved.clear();
                    prefix
                        .and_then(|prefix| target.strip_prefix(prefix))
                        .and_then(|rest| rest.strip_prefix('/'))
                        .ok_or_else(|| format!("symlink {path:?} escapes its final prefix"))?
                } else {
                    target
                };
                for part in target.split('/').rev() {
                    pending.push_front(part.into());
                }
            }
            Some(Entry::Directory) => {}
            Some(Entry::File) if pending.is_empty() => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

pub(super) fn verify(
    expected: &BTreeMap<String, ExpectedFile>,
    directories: &BTreeSet<String>,
    prefix: &str,
) -> Result<(), String> {
    for (path, file) in expected {
        if !matches!(file.content, Content::Symlink(_)) {
            continue;
        }
        let exists = resolve(path, Some(prefix), true, |candidate| {
            match expected.get(candidate).map(|file| &file.content) {
                Some(Content::Symlink(target)) => Some(Entry::Symlink(target)),
                Some(Content::Directory) => Some(Entry::Directory),
                Some(_) => Some(Entry::File),
                None if directories.contains(candidate) => Some(Entry::Directory),
                None => None,
            }
        })?;
        if !exists {
            return Err(format!(
                "symlink {path:?} resolves through missing/non-directory path"
            ));
        }
    }
    Ok(())
}
