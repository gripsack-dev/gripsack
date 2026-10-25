//! Explicit repository subtrees — literal paths plus bounded glob patterns,
//! not Git ignore rules or ambient roots.
use super::{CaptureBudget, CaptureRoot, SourceRootKind, inventory::invalid};
use std::{
    io,
    path::{Path, PathBuf},
};

/// Configuration, lock authority and entrypoints no exclusion may omit, in
/// any ASCII spelling a case-insensitive capture filesystem could hide.
const REQUIRED_ROOTS: [&str; 5] = ["env.toml", "gripsack.ts", "hosts", "gripsack.lock", "locks"];

/// Validated repository-relative exclusions: sorted non-overlapping literal
/// subtrees plus explicit bounded glob patterns, every spelling recorded
/// verbatim. The wire lives in env.toml; its exact copied bytes bind this
/// policy to approval.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SourceCapturePolicy {
    /// Every wire spelling, sorted; inspection shows these verbatim.
    exclusions: Vec<String>,
    /// Literal subtrees, sorted and non-overlapping. The copying walker
    /// matches them with one bounded binary search per ancestor and no
    /// allocation.
    literals: Vec<String>,
    /// Compiled globs, sorted by their wire spelling.
    patterns: Vec<ExclusionPattern>,
}

/// One validated exclusion glob. `*` matches within a single component, `?`
/// one character, and a whole `**` component zero or more components; there
/// are no character classes, escapes or alternation, so matching is a
/// bounded two-pointer alignment over components, never free backtracking.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExclusionPattern {
    /// The verbatim wire spelling, replayed in diagnostics and inspection.
    spelling: String,
    /// A leading `**` matches by suffix, so no alias can move matched bytes
    /// to an unmatched spelling. Every other glob can, and therefore pins
    /// its ancestor directories as real, unaliased directories.
    suffix_anchored: bool,
    components: Vec<String>,
}

impl ExclusionPattern {
    fn matches(&self, path: &[&str]) -> bool {
        components_match(&self.components, path)
    }
    /// Some match of this glob lies strictly below `path`.
    fn matches_descendant(&self, path: &[&str]) -> bool {
        components_descend(&self.components, path)
    }
    /// A repository alias at `path` could re-expose excluded bytes under the
    /// alias target's spelling. Suffix-anchored globs match every spelling of
    /// their suffix, so only the others need their ancestors pinned.
    fn requires_real_ancestor(&self, path: &[&str]) -> bool {
        !self.suffix_anchored && self.matches_descendant(path)
    }
}

/// Classic single-component wildcard match over characters; `*` never crosses
/// a component boundary because components are aligned separately.
fn name_matches(pattern: &str, name: &str) -> bool {
    let (pattern, name) = (
        pattern.chars().collect::<Vec<_>>(),
        name.chars().collect::<Vec<_>>(),
    );
    let (mut p, mut n) = (0, 0);
    let (mut star, mut mark) = (usize::MAX, 0);
    while n < name.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = p;
            mark = n;
            p += 1;
        } else if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Consume every component of `path` against the pattern, `**` absorbing any
/// number of components. Returns the pattern position after alignment, or
/// `None` when `path` cannot be a prefix of any match. The single-star
/// backtrack is the bounded textbook globstar alignment.
fn components_align(pattern: &[String], path: &[&str]) -> Option<usize> {
    let (mut p, mut s) = (0, 0);
    let (mut star, mut mark) = (usize::MAX, 0);
    while s < path.len() {
        if p < pattern.len() && pattern[p] == "**" {
            star = p;
            mark = s;
            p += 1;
        } else if p < pattern.len() && name_matches(&pattern[p], path[s]) {
            p += 1;
            s += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            s = mark;
        } else {
            return None;
        }
    }
    Some(p)
}

fn components_match(pattern: &[String], path: &[&str]) -> bool {
    let Some(mut p) = components_align(pattern, path) else {
        return false;
    };
    while p < pattern.len() && pattern[p] == "**" {
        p += 1;
    }
    p == pattern.len()
}

/// Some match of the pattern lies strictly below `path`.
fn components_descend(pattern: &[String], path: &[&str]) -> bool {
    components_align(pattern, path).is_some_and(|p| p < pattern.len())
}

/// A glob reaches a required root when a root-aligned spelling of it — the
/// pattern with any number of leading `**` components dropped — literally
/// names one, ignoring ASCII case. Wildcard heads are admitted here and
/// refused per walked path instead, so `**/__pycache__` stays usable while
/// `hosts/**` and `**/env.toml` never parse.
fn names_required_root(components: &[&str]) -> bool {
    let mut start = 0;
    loop {
        let Some(first) = components.get(start) else {
            return false;
        };
        if !first.contains(['*', '?'])
            && REQUIRED_ROOTS
                .iter()
                .any(|required| first.eq_ignore_ascii_case(required))
        {
            return true;
        }
        if *first == "**" {
            start += 1;
            continue;
        }
        return false;
    }
}

fn component_spellings(path: &Path) -> Option<Vec<&str>> {
    let text = path.to_str()?;
    Some(text.split('/').filter(|part| !part.is_empty()).collect())
}

impl SourceCapturePolicy {
    pub fn new(mut exclusions: Vec<String>) -> io::Result<Self> {
        let mut budget = CaptureBudget::default();
        let mut literals = Vec::new();
        let mut patterns = Vec::new();
        for value in &exclusions {
            budget.entry(value)?;
            let components: Vec<&str> = value.split('/').collect();
            if value.is_empty()
                || value.contains(['\0', '\\', '[', ']'])
                || components
                    .iter()
                    .any(|part| part.is_empty() || matches!(*part, "." | ".."))
                || components.len() + 1 > super::inventory::MAX_DEPTH
                || value.chars().any(char::is_control)
                || components[0].contains(':')
            {
                return Err(invalid(&format!(
                    "capture exclusion {value:?} must be a normalized repository-relative literal or glob"
                )));
            }
            // These root names carry configuration/authority. Reject their
            // ASCII case variants too, so a case-insensitive capture filesystem cannot
            // hide a required lock behind a differently spelled exclusion.
            if names_required_root(&components) {
                return Err(invalid(&format!(
                    "capture exclusion {value:?} intersects required configuration, lock authority or entrypoints"
                )));
            }
            if components
                .iter()
                .any(|part| part.contains('*') || part.contains('?'))
            {
                if components
                    .iter()
                    .any(|part| part.contains("**") && *part != "**")
                {
                    return Err(invalid(&format!(
                        "capture exclusion {value:?} must use ** as a whole path component"
                    )));
                }
                if components.iter().all(|part| *part == "**") {
                    return Err(invalid(&format!(
                        "capture exclusion {value:?} would exclude the entire repository"
                    )));
                }
                patterns.push(ExclusionPattern {
                    spelling: value.clone(),
                    suffix_anchored: components[0] == "**",
                    components: components.iter().map(|part| (*part).to_owned()).collect(),
                });
            } else {
                literals.push(value.clone());
            }
        }
        exclusions.sort_unstable();
        literals.sort_unstable();
        patterns.sort_unstable_by(|left, right| left.spelling.cmp(&right.spelling));
        // Ancestors need not be adjacent in lexical order (a-b sorts between
        // a and a/b); query every component prefix instead of a quadratic scan.
        for (index, value) in literals.iter().enumerate() {
            if index > 0 && literals[index - 1] == *value {
                return Err(invalid("capture exclusions must not be duplicated"));
            }
            for (end, _) in value.match_indices('/') {
                if literals
                    .binary_search_by(|other| other.as_str().cmp(&value[..end]))
                    .is_ok()
                {
                    return Err(invalid("capture exclusions must not overlap"));
                }
            }
        }
        for (index, pattern) in patterns.iter().enumerate() {
            if index > 0 && patterns[index - 1].spelling == pattern.spelling {
                return Err(invalid("capture exclusions must not be duplicated"));
            }
        }
        Ok(Self {
            exclusions,
            literals,
            patterns,
        })
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
        let relative = if relative
            .as_os_str()
            .as_encoded_bytes()
            .split(|byte| *byte == b'/')
            .any(|part| part.is_empty() || part == b".")
        {
            normalized = relative
                .components()
                .filter(|part| *part != std::path::Component::CurDir)
                .collect::<PathBuf>();
            normalized.as_path()
        } else {
            relative
        };
        if relative.ancestors().filter_map(Path::to_str).any(|path| {
            self.literals
                .binary_search_by(|value| value.as_str().cmp(path))
                .is_ok()
        }) {
            return true;
        }
        self.matches_pattern(relative)
    }

    /// Match the walked path and every ancestor against each glob, mirroring
    /// the literal subtree rule. Allocates bounded component slices only when
    /// the policy actually declares patterns.
    fn matches_pattern(&self, relative: &Path) -> bool {
        let Some(components) = component_spellings(relative) else {
            return false;
        };
        (1..=components.len()).any(|end| {
            self.patterns
                .iter()
                .any(|pattern| pattern.matches(&components[..end]))
        })
    }

    /// Literal exclusion ancestors must remain real directories; see
    /// `pattern_requires_directory` for globs.
    pub(super) fn requires_directory(&self, relative: &Path) -> bool {
        let Some(path) = relative.to_str() else {
            return false;
        };
        let index = self.literals.partition_point(|excluded| {
            excluded
                .bytes()
                .cmp(path.bytes().chain(std::iter::once(b'/')))
                .is_lt()
        });
        self.literals.get(index).is_some_and(|excluded| {
            excluded
                .strip_prefix(path)
                .is_some_and(|suffix| suffix.starts_with('/'))
        })
    }

    /// A repository alias whose path could lie strictly below a glob match
    /// can re-expose excluded bytes under the alias target's spelling, so the
    /// walk refuses it like a replaced literal ancestor.
    pub(super) fn pattern_requires_directory(&self, relative: &Path) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let Some(components) = component_spellings(relative) else {
            return false;
        };
        self.patterns
            .iter()
            .any(|pattern| pattern.requires_real_ancestor(&components))
    }

    /// Required configuration, lock authority and entrypoints stay captured.
    /// Literal spellings naming them were refused at admission; a glob that
    /// matches one refuses the walk instead of silently omitting authority.
    pub(super) fn protected_match(&self, relative: &Path) -> Option<&str> {
        if self.patterns.is_empty() {
            return None;
        }
        let components = component_spellings(relative)?;
        if !components.first().is_some_and(|head| {
            REQUIRED_ROOTS
                .iter()
                .any(|required| head.eq_ignore_ascii_case(required))
        }) {
            return None;
        }
        self.patterns
            .iter()
            .find(|pattern| pattern.matches(&components))
            .map(|pattern| pattern.spelling.as_str())
    }

    pub(super) fn admit_roots(&self, roots: &[CaptureRoot]) -> io::Result<()> {
        let repo = &roots[0];
        for other in &roots[1..] {
            if repo.canonical.starts_with(&other.canonical) {
                return Err(invalid(
                    "an auxiliary source root must not contain the repository",
                ));
            }
            for excluded in &self.literals {
                let path = repo.canonical.join(excluded);
                if path.starts_with(&other.canonical) || other.canonical.starts_with(&path) {
                    return Err(invalid(&format!(
                        "capture exclusion {excluded:?} overlaps an admitted {} root",
                        other.kind.directory()
                    )));
                }
            }
            // A glob must not carve the repo spelling of an admitted root
            // while the root itself is captured whole, exactly as literals
            // are refused there.
            if let Some(components) = other
                .canonical
                .strip_prefix(&repo.canonical)
                .ok()
                .and_then(component_spellings)
            {
                for pattern in &self.patterns {
                    if (0..=components.len()).any(|end| pattern.matches(&components[..end]))
                        || pattern.matches_descendant(&components)
                    {
                        return Err(invalid(&format!(
                            "capture exclusion {:?} overlaps an admitted {} root",
                            pattern.spelling,
                            other.kind.directory()
                        )));
                    }
                }
            }
        }
        self.admit_ancestors(repo)
    }

    /// Exclusion ancestors must be real directories, not alternate spellings
    /// of a different subtree. The excluded leaf itself is never opened.
    fn admit_ancestors(&self, repo: &CaptureRoot) -> io::Result<()> {
        let mut budget = CaptureBudget::default();
        for excluded in &self.literals {
            Self::require_real_ancestors(repo, excluded, excluded, &mut budget)?;
        }
        for pattern in &self.patterns {
            // The wildcard-free leading components are the concrete ancestors
            // every match shares; verify them exactly as literal ancestors.
            let anchor = pattern
                .components
                .iter()
                .take_while(|part| !part.contains('*') && !part.contains('?'))
                .count();
            if anchor > 0 {
                Self::require_real_ancestors(
                    repo,
                    &pattern.components[..anchor].join("/"),
                    &pattern.spelling,
                    &mut budget,
                )?;
            }
        }
        Ok(())
    }

    fn require_real_ancestors(
        repo: &CaptureRoot,
        excluded: &str,
        label: &str,
        budget: &mut CaptureBudget,
    ) -> io::Result<()> {
        let mut directory = repo.directory.try_clone()?;
        let parent = Path::new(excluded)
            .parent()
            .expect("relative exclusion has a parent");
        for component in parent.components() {
            budget.resolve_step()?;
            let name = Path::new(component.as_os_str());
            match gripsack_fs::open_dir_nofollow(&directory, name) {
                Ok(child) => directory = child,
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "capture exclusion {label:?} requires real directory ancestors: {error}"
                        ),
                    ));
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
        relative
            .components()
            .any(|component| component.as_os_str() == ".git")
            || (root.kind == SourceRootKind::Repository && self.policy.excludes(relative))
            || (self.runtime_home != root.canonical
                && self.runtime_home.starts_with(&root.canonical)
                && root.canonical.join(relative).starts_with(self.runtime_home))
    }

    pub fn require_available(&self, root: &CaptureRoot, relative: &Path) -> io::Result<()> {
        if self.excludes(root, relative) {
            let logical: PathBuf = root.logical(relative);
            return Err(invalid(&format!(
                "source alias enters excluded subtree {}",
                logical.display()
            )));
        }
        Ok(())
    }
}
