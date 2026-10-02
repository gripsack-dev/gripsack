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

pub(super) fn verify(
    expected: &BTreeMap<String, ExpectedFile>,
    directories: &BTreeSet<String>,
    prefix: &str,
) -> Result<(), String> {
    for (path, file) in expected {
        if !matches!(file.content, Content::Symlink(_)) {
            continue;
        }
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
            match expected.get(&candidate).map(|file| &file.content) {
                Some(Content::Symlink(target)) => {
                    traversals += 1;
                    // Match the Unix kernel's bounded link traversal, rejecting cycles.
                    if traversals > 40 {
                        return Err(format!("symlink {path:?} cycles or exceeds 40 traversals"));
                    }
                    resolved.pop();
                    let target = if target.starts_with('/') {
                        resolved.clear();
                        target
                            .strip_prefix(prefix)
                            .and_then(|rest| rest.strip_prefix('/'))
                            .ok_or_else(|| format!("symlink {path:?} escapes its final prefix"))?
                    } else {
                        target.as_str()
                    };
                    for part in target.split('/').rev() {
                        pending.push_front(part.into());
                    }
                }
                Some(Content::Directory) => {}
                Some(_) if pending.is_empty() => {}
                None if directories.contains(&candidate) => {}
                _ => {
                    return Err(format!(
                        "symlink {path:?} resolves through missing/non-directory {candidate:?}"
                    ));
                }
            }
        }
    }
    Ok(())
}
