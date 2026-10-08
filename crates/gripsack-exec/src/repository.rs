//! Mutable repository publication and immutable evaluated contents are distinct.
//! Direct IR callers choose live inputs explicitly; CLI evaluation retains its
//! captured bundle and receipt for the entire downstream operation.
use gripsack_store::{
    prior::FileMode, source_bundle::SourceBundle, trust::evaluation::EvaluationId,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub enum Repository {
    Direct(PathBuf),
    Evaluated {
        sources: Arc<SourceBundle>,
        receipt: EvaluationId,
    },
}

impl Repository {
    pub fn direct(path: PathBuf) -> Self {
        Self::Direct(path)
    }
    pub fn evaluated(sources: Arc<SourceBundle>, receipt: EvaluationId) -> Self {
        Self::Evaluated { sources, receipt }
    }

    /// Operator-owned mutable pin state/publication, held under the existing lifecycle session.
    pub fn identity(&self) -> &Path {
        match self {
            Self::Direct(path) => path,
            Self::Evaluated { sources, .. } => sources.repository_identity(),
        }
    }

    /// Content and repository configuration use these selected bytes.
    pub fn contents(&self) -> &Path {
        match self {
            Self::Direct(path) => path,
            Self::Evaluated { sources, .. } => sources.repository(),
        }
    }

    /// A repository-local file source always consumes selected repository
    /// contents, including an evaluated repository's admitted --repo alias.
    /// Explicit outside file transports keep their existing native meaning.
    pub(crate) fn source_relative<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        match self {
            Self::Evaluated { sources, .. } => sources.repository_relative(path),
            Self::Direct(root) if path.is_absolute() => path.strip_prefix(root).ok(),
            Self::Direct(_) => Some(path),
        }
    }

    pub(crate) fn materialization_path(&self, relative: &Path) -> io::Result<PathBuf> {
        match self {
            Self::Direct(path) => Ok(path.join(relative)),
            Self::Evaluated { sources, .. } => sources.materialization_path(relative),
        }
    }

    pub(crate) fn overlay_hash(
        &self,
        froms: &[String],
    ) -> io::Result<gripsack_store::hash::PayloadHash> {
        match self {
            Self::Direct(path) => gripsack_store::canonical_overlay_hash(path, froms),
            Self::Evaluated { sources, .. } => sources.overlay_hash(froms),
        }
    }

    pub fn evaluation(&self) -> Option<EvaluationId> {
        match self {
            Self::Direct(_) => None,
            Self::Evaluated { receipt, .. } => Some(*receipt),
        }
    }

    pub(crate) fn snapshot_root(&self) -> Option<&Path> {
        match self {
            Self::Direct(_) => None,
            Self::Evaluated { sources, .. } => sources.repository().parent(),
        }
    }

    pub(crate) fn original_mode(&self, relative: &Path, observed: u32) -> io::Result<FileMode> {
        match self {
            Self::Direct(_) => FileMode::try_from(observed),
            Self::Evaluated { sources, .. } => sources.original_mode(relative),
        }
    }
}
