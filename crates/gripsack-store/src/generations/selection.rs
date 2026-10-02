//! Durable transaction-specific names for immutable generation selections.
//! `current` still resolves to the generation tree, but its link bytes also
//! identify the lifecycle that performed the flip. Reservations are never
//! reused; they are metadata, not extra roots for every historical payload.
use crate::{
    GenerationId, private_state,
    selection_wire::{TransactionText, parse_transaction},
};
use gripsack_fs::{Dir, open_dir_nofollow};
use gripsack_policy::selection::{SelectionIdentity, TransactionId};
use std::{
    io,
    path::{Component, Path, PathBuf},
};

const SELECTIONS: &str = ".selections";

#[derive(Debug)]
pub(crate) struct SelectionReservation {
    identity: SelectionIdentity,
    target: PathBuf,
}

impl SelectionReservation {
    pub(crate) fn identity(&self) -> &SelectionIdentity {
        &self.identity
    }
    pub(crate) fn target(&self) -> &Path {
        &self.target
    }
}

pub(crate) fn reserve(home: &Dir, generation: GenerationId) -> io::Result<SelectionReservation> {
    gripsack_fs::create_dir_all(home, Path::new(crate::GENERATIONS_DIR))?;
    let generations = open_dir_nofollow(home, Path::new(crate::GENERATIONS_DIR))?;
    let selections = private_state::ensure_directory(&generations, Path::new(SELECTIONS))?;
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| io::Error::other(format!("transaction identity entropy: {error}")))?;
    let transaction = TransactionId::from_bytes(bytes);
    let name = TransactionText(&transaction).to_string();
    // create_dir, not create_dir_all: an existing identity cannot be reused or
    // overwritten, even if a previous operation left only a reservation.
    selections.create_dir(&name)?;
    let directory = open_dir_nofollow(&selections, Path::new(&name))?;
    private_state::restrict_directory(&directory, Path::new(&name))?;
    let number = generation.to_string();
    let generation_target = Path::new("..").join("..").join(&number);
    gripsack_fs::symlink_replace(&directory, Path::new(&number), &generation_target)?;
    gripsack_fs::fsync_dir(&selections, Path::new("."))?;
    Ok(SelectionReservation {
        identity: SelectionIdentity::transaction(generation, transaction),
        target: Path::new(crate::GENERATIONS_DIR)
            .join(SELECTIONS)
            .join(name)
            .join(number),
    })
}

/// Recognize the reserved new shape before the legacy reader. Malformed names
/// inside this namespace never fall back to generation-only commitment.
pub(crate) fn parse(home: &Dir, target: &Path) -> io::Result<Option<SelectionIdentity>> {
    let mut components = target.components();
    if components.next() != Some(Component::Normal(crate::GENERATIONS_DIR.as_ref()))
        || components.next() != Some(Component::Normal(SELECTIONS.as_ref()))
    {
        return Ok(None);
    }
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "current has an invalid transaction selection",
        )
    };
    let (Some(Component::Normal(transaction)), Some(Component::Normal(number)), None) =
        (components.next(), components.next(), components.next())
    else {
        return Err(invalid());
    };
    let transaction_text = transaction.to_str().ok_or_else(invalid)?;
    let transaction = parse_transaction(transaction_text)?;
    let number_text = number.to_str().ok_or_else(invalid)?;
    let generation: GenerationId = number_text.parse().map_err(|_| invalid())?;
    if number_text.starts_with('+') || (number_text.len() > 1 && number_text.starts_with('0')) {
        return Err(invalid());
    }
    let generations = open_dir_nofollow(home, Path::new(crate::GENERATIONS_DIR))?;
    let selections = open_dir_nofollow(&generations, Path::new(SELECTIONS))?;
    let directory = open_dir_nofollow(&selections, Path::new(transaction_text))?;
    let expected = Path::new("..").join("..").join(number);
    if directory.read_link(number)? != expected {
        return Err(invalid());
    }
    Ok(Some(SelectionIdentity::transaction(
        generation,
        transaction,
    )))
}
