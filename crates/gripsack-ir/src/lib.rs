//! The workspace contract between the TypeScript frontend and Rust core.
//! JSON passes strict current-version decoding, tagged-field admission and
//! semantic validation before any realization or deployment.

mod authoring;
pub mod diagnostic;
pub mod host_name;
pub mod model;
pub mod parse;
pub mod placeholders;
pub mod sema;
pub mod span;
mod tagged;
pub mod workspace;
mod workspace_catalog;
pub mod workspace_model;

pub use diagnostic::{Diagnostic, Label, Severity, codes};
pub use host_name::HostName;
pub use model::*;
pub use parse::{IR_VERSION, parse};
pub use sema::{check, run};
pub use span::Span;
pub use workspace::*;
pub use workspace_catalog::CatalogOutput;
