//! Everything gripsack owns on disk: store paths, generations, and the
//! atomic primitives every write goes through.
//!
//! ```text
//! $GRIPSACK_HOME                  (~/.local/share/gripsack by default)
//! ├── store/<input-hash>-<name>/  immutable, content-addressed payloads
//! ├── generations/<N>/            one complete profile tree per apply
//! ├── current -> generations/<N>  THE symlink — flipping it is activation
//! ├── locks/                      flock files for named resources (0007 §4)
//! └── trust.toml                  repo trust list — the gate before eval
//!                                 (0013 D7)
//! ```
//!
//! The modules divide the concerns:
//!
//! - [`paths`] — where things live (layout, `$GRIPSACK_HOME` resolution)
//! - [`hash`] — canonical content identity: type + exec-bit + contents
//!   (0008 §2); what makes two payloads "the same"
//! - [`fs`] — how bytes land: atomic writes, atomic symlink flips,
//!   publish-once directories (0001 §9.2)
//! - [`trust`] — which repos may be evaluated at all (0013 D7)
//!
//! `store` is concerned with content correctness; `fs` guarantees the
//! mechanics — a reader never sees a partial write, and activation is a
//! single indivisible rename.

pub mod fs;
pub mod generations;
pub mod hash;
pub mod journal;
pub mod paths;
pub mod trust;

pub use fs::{
    atomic_write, copy_dir, prior_blob_path, publish_dir, store_prior_blob, symlink_replace,
};
pub use generations::{
    DeployedEntry, Generation, ModuleState, Prior, PriorKind, current as current_generation, flip,
    list as list_generations, read_manifest, write_manifest,
};
pub use hash::{
    canonical_bytes_hash, canonical_file_hash, canonical_overlay_hash, canonical_tree_hash,
};
pub use journal::{Entry as JournalEntry, Prior as JournalPrior, reconcile, record};
pub use paths::{
    GENERATIONS_DIR, HASH_LEN, STORE_DIR, content_path, current_link, expand_home, generation_dir,
    gripsack_home, input_hash, store_path,
};
pub use trust::{TrustedRepo, ensure_trusted};
