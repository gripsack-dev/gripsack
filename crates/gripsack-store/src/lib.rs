//! Input-addressed store paths and generations (plan/0001 §3.4) plus
//! canonical content identity (0008 §2).

pub mod hash;
pub mod paths;

pub use hash::{canonical_file_hash, canonical_tree_hash};
pub use paths::{
    current_link, generation_dir, gripsack_home, input_hash, store_path, GENERATIONS_DIR, HASH_LEN,
    STORE_DIR,
};
