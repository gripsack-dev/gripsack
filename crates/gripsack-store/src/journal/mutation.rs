//! An admitted run and one owned destination capability precede mutation.
use super::{
    Entry, Intended, JournalRun, ObjectIdentity, Prior, PriorSerde, entry_name, live_identity,
    prior_identity,
};
use gripsack_fs::Dir;
use gripsack_policy::journal_protocol::{MutationAuthority, RecordRole, admit_mutation};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// A prior captured under this active run and this exact parent capability.
/// External writers can still race observation and mutation; this is not CAS.
pub struct CapturedPrior<'a> {
    run: &'a JournalRun<'a>,
    directory: Dir,
    name: PathBuf,
    destination: &'a Path,
    prior: Prior,
}

impl CapturedPrior<'_> {
    pub fn prior(&self) -> &Prior {
        &self.prior
    }
}

/// The durable marker, captured prior and entry grant one concrete mutation.
#[must_use]
pub struct MutationPermit<'a> {
    captured: CapturedPrior<'a>,
    intended: &'a Intended,
    _authority: MutationAuthority<'a>,
}

impl MutationPermit<'_> {
    pub fn execute(self, effect: impl FnOnce(&Dir, &Path) -> io::Result<()>) -> io::Result<()> {
        effect(&self.captured.directory, &self.captured.name)?;
        let live = live_identity(&self.captured.directory, &self.captured.name)?;
        let landed = match self.intended {
            Intended::Removed => live.is_none(),
            Intended::Object(identity) => live.as_ref() == Some(identity),
        };
        if !landed {
            return Err(io::Error::other(format!(
                "{} did not reach its intended state (expected {}, found {})",
                self.captured.destination.display(),
                self.intended,
                live.as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "absent".into()),
            )));
        }
        Ok(())
    }
}

pub fn capture<'a>(
    run: &'a JournalRun<'a>,
    directory: Dir,
    name: PathBuf,
    destination: &'a Path,
    expected: Option<&ObjectIdentity>,
) -> io::Result<CapturedPrior<'a>> {
    if destination.to_str().is_none() {
        return Err(io::Error::other(format!(
            "destination {} is not valid UTF-8 — refusing to journal it (the object is untouched)",
            destination.display(),
        )));
    }
    let metadata = match directory.symlink_metadata(&name) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let prior = match metadata {
        None => {
            require_expected(destination, None, expected)?;
            Prior::Absent
        }
        Some(metadata) if metadata.file_type().is_symlink() => {
            let target = directory.read_link_contents(&name)?;
            let target = target.to_str().ok_or_else(|| {
                io::Error::other(format!(
                    "{} is a symlink with a non-UTF-8 target — refusing to journal it",
                    destination.display(),
                ))
            })?;
            let observed = ObjectIdentity::Link(target.to_owned());
            require_expected(destination, Some(&observed), expected)?;
            let ObjectIdentity::Link(target) = observed else {
                unreachable!()
            };
            Prior::Symlink { target }
        }
        Some(metadata) if metadata.is_file() => {
            // Mode and bytes come from the same no-follow handle. A directory
            // descriptor protects namespace selection, not concurrent writers.
            let mut file = gripsack_fs::open_file_nofollow(&directory, &name)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "journal prior is not a regular file",
                ));
            }
            use gripsack_fs::cap_std::fs::MetadataExt;
            let mode = crate::prior::FileMode::try_from(metadata.mode() & 0o7777)?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            let observed =
                ObjectIdentity::File(crate::hash::canonical_bytes_identity(&bytes, mode.bits()));
            require_expected(destination, Some(&observed), expected)?;
            let hash = crate::prior::store_blob(run.home(), &bytes)?;
            Prior::File { hash, mode }
        }
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "journal prior is not a regular file or symlink",
            ));
        }
    };
    Ok(CapturedPrior {
        run,
        directory,
        name,
        destination,
        prior,
    })
}

fn require_expected(
    destination: &Path,
    observed: Option<&ObjectIdentity>,
    expected: Option<&ObjectIdentity>,
) -> io::Result<()> {
    if observed != expected {
        return Err(io::Error::other(format!(
            "{} changed between the drift decision and the mutation — aborting; re-run to retry",
            destination.display(),
        )));
    }
    Ok(())
}

pub fn record<'a>(
    captured: CapturedPrior<'a>,
    intended: &'a Intended,
) -> io::Result<MutationPermit<'a>> {
    let name = entry_name(captured.destination);
    let before = PriorSerde::from(&captured.prior);
    let destination = captured.destination.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "captured journal destination lost its UTF-8 identity",
        )
    })?;
    let entry = match captured.run.journal.read(&name) {
        Ok(bytes) => {
            let previous = Entry::from_wire(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            if previous.dest != destination {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "journal destination identity changed",
                ));
            }
            let observed = prior_identity(&before, captured.run.home())?;
            let expected = Intended::from_wire(&previous.after);
            let follows_previous = match (&observed, &expected) {
                (None, Intended::Removed) => true,
                (Some(observed), Intended::Object(expected)) => observed == expected,
                _ => false,
            };
            if !follows_previous {
                return Err(io::Error::other(
                    "destination changed between journaled mutations; refusing to overwrite the intervening edit",
                ));
            }
            previous.advance(before, intended.to_serde())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Entry::new(destination.to_owned(), before, intended.to_serde())
        }
        Err(error) => return Err(error),
    };
    let durable = captured.run.journal.write(
        &name,
        serde_json::to_string(&entry)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .as_bytes(),
        RecordRole::MutationEntry,
    )?;
    let authority = admit_mutation(&captured.run.marker_record, durable).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "journal publication roles do not authorize mutation",
        )
    })?;
    Ok(MutationPermit {
        captured,
        intended,
        _authority: authority,
    })
}
