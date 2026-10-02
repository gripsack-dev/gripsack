//! Current generation pointer admission and activation.
use gripsack_policy::selection::SelectionIdentity;

use crate::GenerationId;
use std::{io, path::Path};
/// The generation `current` points at, if any. Fail closed
/// (0026 §8): only NotFound means "no generations" — a permission
/// error, I/O failure, or a `current` link that parses to no
/// generation number are real errors, not absence (apply allocates
/// from this, gc protects it; misreading either is corruption).
pub fn current(home: &Path) -> io::Result<Option<GenerationId>> {
    let directory = match gripsack_fs::open(home) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    current_in(home, &directory)
}

/// Read the pointer through an already pinned home, as used by GC.
pub fn current_in(home: &Path, directory: &gripsack_fs::Dir) -> io::Result<Option<GenerationId>> {
    Ok(current_selection_in(home, directory)?.map(|selection| selection.generation()))
}

/// The exact selection observed through the same pinned home as recovery.
/// Reserved transaction names cannot be reinterpreted as legacy generations.
pub fn current_selection_in(
    home: &Path,
    directory: &gripsack_fs::Dir,
) -> io::Result<Option<SelectionIdentity>> {
    match directory.read_link("current") {
        Ok(target) => {
            if let Some(selection) = super::parse_selection(directory, &target)? {
                return Ok(Some(selection));
            }
            // the relative canonical form skips canonicalize entirely
            if let Some(n) = parse_current_relative(&target) {
                return Ok(Some(SelectionIdentity::legacy(n)));
            }
            let n: GenerationId = target
                .file_name()
                .and_then(|n| n.to_string_lossy().parse().ok())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "{} does not point at a generation",
                            crate::paths::current_link(home).display()
                        ),
                    )
                })?;
            // the control plane and data plane must agree (0029 §10):
            // the link resolves to THIS home's generations/<n> — a
            // `current -> /tmp/42` is corruption, not generation 42
            let resolved = std::fs::canonicalize(crate::paths::current_link(home))?;
            let home_canon = std::fs::canonicalize(home)?;
            let expected = home_canon
                .join(crate::paths::GENERATIONS_DIR)
                .join(n.to_string());
            if resolved != expected {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "current resolves to {} — outside $GRIPSACK_HOME/generations",
                        resolved.display()
                    ),
                ));
            }
            Ok(Some(SelectionIdentity::legacy(n)))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Read the historical relative `generations/<N>` spelling without ambient
/// canonicalization. New writes always carry transaction identity instead.
fn parse_current_relative(target: &Path) -> Option<GenerationId> {
    let mut parts = target.components();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(head)), Some(std::path::Component::Normal(n)), None)
            if head == crate::paths::GENERATIONS_DIR =>
        {
            n.to_string_lossy().parse().ok()
        }
        _ => None,
    }
}

/// Exact selection whose current-pointer publication completed its barrier.
/// Only `flip` constructs this cleanup authority.
#[derive(Debug)]
pub struct CommittedSelection<'a> {
    run: crate::journal::JournalRun<'a>,
}

impl CommittedSelection<'_> {
    pub(crate) fn journal_directory(&self) -> &gripsack_fs::Dir {
        self.run.journal_directory()
    }
}

/// Activate exactly the target whose transaction marker was durably recorded.
/// Both the generation and the reserved selection path are admitted before
/// the single current-pointer rename; no generation-only write path remains.
pub fn flip(run: crate::journal::JournalRun<'_>) -> io::Result<CommittedSelection<'_>> {
    let home = run.home();
    let home_path = run.home_path();
    let generation = run.identity().generation();
    super::admit_manifest_at(home, home_path, generation)?;
    if super::parse_selection(home, run.target())?.as_ref() != Some(run.identity()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "reserved selection identity changed before activation",
        ));
    }
    gripsack_fs::symlink_replace(home, Path::new("current"), run.target())?;
    Ok(CommittedSelection { run })
}
