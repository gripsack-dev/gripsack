//! Current generation pointer admission and activation.
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
    match directory.read_link("current") {
        Ok(target) => {
            // the relative canonical form skips canonicalize entirely
            if let Some(n) = parse_current_relative(&target) {
                return Ok(Some(n));
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
            Ok(Some(n))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Parse the canonical relative `generations/<N>` target (0030 §H10).
/// The ONE shape flip writes; both current readers accept it without
/// ambient canonicalization.
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

/// Flip `current` to a generation — the single indivisible activation
/// operation (0001 §9.2), pinned to the home capability (plan/0021).
/// `home_path` exists only to compose the link TARGET: `current`
/// records the absolute generation dir so readers resolve it from
/// any cwd. The target must exist first: a `current` pointing at
/// nothing reads as "no generations" everywhere downstream
/// (list/current swallow that as None) while looking deployed to
/// anything that inspects the link.
pub fn flip(home: &gripsack_fs::Dir, home_path: &Path, generation: GenerationId) -> io::Result<()> {
    let rel = Path::new(crate::paths::GENERATIONS_DIR).join(generation.to_string());
    let dir = home_path.join(&rel);
    if !home.metadata(&rel).is_ok_and(|m| m.is_dir()) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "cannot activate generation {generation}: {} is missing",
                dir.display()
            ),
        ));
    }
    // the link target is the RELATIVE canonical form (0030 §H10): one
    // lexical shape means both readers validate without ambient
    // canonicalization. Pre-0.26 absolute targets still validate via
    // canonicalize in the reader.
    gripsack_fs::symlink_replace(home, Path::new("current"), &rel)
}
