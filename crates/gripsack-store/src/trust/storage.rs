//! Private, bounded approval and inventory records. Missing authority is never
//! repaired by inventing a source digest or silently accepting a corrupt cache.
use super::{
    audit,
    wire::{LegacyTrustFile, MAX_TRUST_BYTES, TrustListing},
};
use crate::source_bundle::{SourceBundle, SourceBundleDigest, SourceInventory};
use gripsack_fs::Dir;
use std::{
    io::{self, Read},
    path::Path,
};

const INVENTORY_BYTES: usize = 16 * 1024 * 1024;
const STATE: &str = "trust";
const INVENTORIES: &str = "inventories";

pub(super) fn lock(home: &Path) -> io::Result<gripsack_fs::FlockGuard> {
    gripsack_fs::FlockGuard::acquire(&home.join("locks"), "trust")
}

pub(super) fn load(home: &Path) -> io::Result<TrustListing> {
    let directory = match gripsack_fs::open(home) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(TrustListing::default()),
        Err(error) => return Err(error),
    };
    let Some(bytes) = read(&directory, Path::new("trust.toml"), MAX_TRUST_BYTES)? else {
        return Ok(TrustListing::default());
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Document {
        Current(TrustListing),
        Legacy(LegacyTrustFile),
    }
    let document: Document = toml::from_str(text).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("cannot admit trust.toml: {error}"),
        )
    })?;
    let mut listing = match document {
        Document::Current(listing) => listing,
        Document::Legacy(legacy) => TrustListing {
            legacy: legacy.repos,
            ..TrustListing::default()
        },
    };
    listing.validate()?;
    for approval in &mut listing.approved {
        approval.provenance.remote = approval
            .provenance
            .remote
            .as_deref()
            .and_then(audit::redact_remote);
    }
    for legacy in &mut listing.legacy {
        legacy.remote = legacy.remote.as_deref().and_then(audit::redact_remote);
    }
    Ok(listing)
}

pub(super) fn save(home: &Path, listing: &TrustListing) -> io::Result<()> {
    listing.validate()?;
    let text = toml::to_string(listing).map_err(io::Error::other)?;
    if text.len() > MAX_TRUST_BYTES {
        return Err(invalid("trust document exceeds its byte budget"));
    }
    let directory = gripsack_fs::open_or_create(home)?;
    gripsack_fs::atomic_write_with_mode(&directory, Path::new("trust.toml"), text.as_bytes(), 0o600)
}

pub(super) fn save_inventory(home: &Path, bundle: &SourceBundle) -> io::Result<()> {
    let directory = gripsack_fs::open_or_create(home)?;
    let state = crate::private_state::ensure_directory(&directory, Path::new(STATE))?;
    let inventories = crate::private_state::ensure_directory(&state, Path::new(INVENTORIES))?;
    let name = format!("{}.json", bundle.digest());
    if let Some(bytes) = read(&inventories, Path::new(&name), INVENTORY_BYTES)? {
        if bytes != bundle.inventory_bytes() {
            return Err(invalid(
                "cached source inventory differs from the captured bytes",
            ));
        }
        return Ok(());
    }
    gripsack_fs::atomic_write_with_mode(
        &inventories,
        Path::new(&name),
        bundle.inventory_bytes(),
        0o600,
    )
}

pub(super) fn inventory(home: &Path, digest: SourceBundleDigest) -> io::Result<SourceInventory> {
    let directory = gripsack_fs::open(home)?;
    let state = gripsack_fs::open_dir_nofollow(&directory, Path::new(STATE))?;
    let inventories = gripsack_fs::open_dir_nofollow(&state, Path::new(INVENTORIES))?;
    let name = format!("{digest}.json");
    let bytes = read(&inventories, Path::new(&name), INVENTORY_BYTES)?.ok_or_else(|| {
        invalid("approved source inventory is missing; inspect and renew approval")
    })?;
    SourceInventory::decode(&bytes, digest)
}

pub(super) fn admit_inventory(home: &Path, bundle: &SourceBundle) -> io::Result<()> {
    let directory = gripsack_fs::open(home)?;
    let state = gripsack_fs::open_dir_nofollow(&directory, Path::new(STATE))?;
    let inventories = gripsack_fs::open_dir_nofollow(&state, Path::new(INVENTORIES))?;
    let name = format!("{}.json", bundle.digest());
    let bytes = read(&inventories, Path::new(&name), INVENTORY_BYTES)?.ok_or_else(|| {
        invalid("approved source inventory is missing; inspect and renew approval")
    })?;
    if bytes != bundle.inventory_bytes() {
        return Err(invalid(
            "approved source inventory differs from the captured bytes",
        ));
    }
    Ok(())
}

fn read(directory: &Dir, name: &Path, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut file = match gripsack_fs::open_file_nofollow(directory, name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    crate::private_state::restrict_file(&file, name)?;
    let length = file.metadata()?.len();
    if length > limit as u64 {
        return Err(invalid("trust record exceeds its byte budget"));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid("trust record exceeds its byte budget"));
    }
    Ok(Some(bytes))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
