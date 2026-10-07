//! Explicit literal repository subtrees, not Git ignore rules or ambient roots.
use super::{CaptureBudget, CaptureRoot, SourceRootKind, inventory::invalid};
use std::{io, path::{Path, PathBuf}};

/// Validated, sorted, non-overlapping repository-relative exclusions. The wire
/// lives in env.toml; its exact copied bytes bind this policy to approval.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SourceCapturePolicy {
    exclusions: Vec<String>,
}

impl SourceCapturePolicy {
    pub fn new(mut exclusions: Vec<String>) -> io::Result<Self> {
        let mut budget = CaptureBudget::default();
        for value in &exclusions {
            budget.entry(value)?;
            if value.is_empty()
                || value.contains(['\0', '\\', '*', '?', '[', ']'])
                || value.split('/').any(|part| part.is_empty() || matches!(part, "." | ".."))
                || value.split('/').count() + 1 > super::inventory::MAX_DEPTH
                || value.chars().any(char::is_control)
                || value.split('/').next().is_some_and(|part| part.contains(':'))
            {
                return Err(invalid(&format!(
                    "capture exclusion {value:?} must be a normalized literal repository-relative subtree"
                )));
            }
            // These root names carry configuration/authority. Reject their
            // ASCII case variants too, so a case-insensitive capture filesystem cannot
            // hide a required lock behind a differently spelled exclusion.
            let first = value.split('/').next().expect("nonempty normalized exclusion");
            if ["env.toml", "gripsack.ts", "hosts", "gripsack.lock", "locks"]
                .iter().any(|required| first.eq_ignore_ascii_case(required))
            {
                return Err(invalid(&format!(
                    "capture exclusion {value:?} intersects required configuration, lock authority or entrypoints"
                )));
            }
        }
        exclusions.sort_unstable();
        // Ancestors need not be adjacent in lexical order (a-b sorts between
        // a and a/b); query every component prefix instead of a quadratic scan.
        for (index, value) in exclusions.iter().enumerate() {
            if index > 0 && exclusions[index - 1] == *value {
                return Err(invalid("capture exclusions must not be duplicated"));
            }
            for (end, _) in value.match_indices('/') {
                if exclusions.binary_search_by(|other| other.as_str().cmp(&value[..end])).is_ok() {
                    return Err(invalid("capture exclusions must not overlap"));
                }
            }
        }
        Ok(Self { exclusions })
    }

    pub fn exclusions(&self) -> &[String] {
        &self.exclusions
    }

    pub fn excludes(&self, relative: &Path) -> bool {
        if self.exclusions.is_empty() {
            return false;
        }
        // Native selectors may spell an ordinary subtree as ./a or a//b.
        // Normalize only those spellings; the copying walker allocates nothing.
        // Parent traversal is never collapsed (callers reject it separately).
        let normalized;
        let relative = if relative.as_os_str().as_encoded_bytes().split(|byte| *byte == b'/')
            .any(|part| part.is_empty() || part == b".")
        {
            normalized = relative.components()
                .filter(|part| *part != std::path::Component::CurDir)
                .collect::<PathBuf>();
            normalized.as_path()
        } else {
            relative
        };
        relative.ancestors().filter_map(Path::to_str).any(|path| {
            self.exclusions.binary_search_by(|value| value.as_str().cmp(path)).is_ok()
        })
    }

    pub(super) fn requires_directory(&self, relative: &Path) -> bool {
        let Some(path) = relative.to_str() else { return false };
        let index = self.exclusions.partition_point(|excluded| {
            excluded.bytes().cmp(path.bytes().chain(std::iter::once(b'/'))).is_lt()
        });
        self.exclusions.get(index).is_some_and(|excluded| {
            excluded.strip_prefix(path).is_some_and(|suffix| suffix.starts_with('/'))
        })
    }

    pub(super) fn admit_roots(&self, roots: &[CaptureRoot]) -> io::Result<()> {
        let repo = &roots[0];
        for other in &roots[1..] {
            if repo.canonical.starts_with(&other.canonical) {
                return Err(invalid("an auxiliary source root must not contain the repository"));
            }
            for excluded in &self.exclusions {
                let path = repo.canonical.join(excluded);
                if path.starts_with(&other.canonical) || other.canonical.starts_with(&path) {
                    return Err(invalid(&format!(
                        "capture exclusion {excluded:?} overlaps an admitted {} root",
                        other.kind.directory()
                    )));
                }
            }
        }
        self.admit_ancestors(repo)
    }

    /// Exclusion ancestors must be real directories, not alternate spellings
    /// of a different subtree. The excluded leaf itself is never opened.
    fn admit_ancestors(&self, repo: &CaptureRoot) -> io::Result<()> {
        let mut budget = CaptureBudget::default();
        for excluded in &self.exclusions {
            let mut directory = repo.directory.try_clone()?;
            let parent = Path::new(excluded).parent().expect("relative exclusion has a parent");
            for component in parent.components() {
                budget.resolve_step()?;
                let name = Path::new(component.as_os_str());
                match gripsack_fs::open_dir_nofollow(&directory, name) {
                    Ok(child) => directory = child,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                    Err(error) => return Err(io::Error::new(error.kind(), format!(
                        "capture exclusion {excluded:?} requires real directory ancestors: {error}"
                    ))),
                }
            }
        }
        Ok(())
    }
}

pub(super) struct CaptureAdmission<'a> {
    pub policy: &'a SourceCapturePolicy,
    pub runtime_home: &'a Path,
}

impl CaptureAdmission<'_> {
    pub fn excludes(&self, root: &CaptureRoot, relative: &Path) -> bool {
        relative.components().any(|component| component.as_os_str() == ".git")
            || (root.kind == SourceRootKind::Repository && self.policy.excludes(relative))
            || (self.runtime_home != root.canonical
                && self.runtime_home.starts_with(&root.canonical)
                && root.canonical.join(relative).starts_with(self.runtime_home))
    }

    pub fn require_available(&self, root: &CaptureRoot, relative: &Path) -> io::Result<()> {
        if self.excludes(root, relative) {
            let logical: PathBuf = root.logical(relative);
            return Err(invalid(&format!(
                "source alias enters excluded subtree {}", logical.display()
            )));
        }
        Ok(())
    }
}
