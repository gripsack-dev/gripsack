//! Retained subject evidence is relative to its immutable profile, never staging.
use crate::{ExecError, workspace::file_failure};
use gripsack_ir::{
    Span,
    workspace_model::{FileCheckStage, FileCheckSubject},
};
use gripsack_store::{self as store, hash::PayloadHash};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Component, Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::workspace) struct FileCheckInputs {
    pub check: String,
    pub subject: FileCheckSubject,
    pub stage: FileCheckStage,
    pub span: Span,
    pub file_span: Span,
    pub command_span: Span,
    pub captured: PathBuf,
    pub captured_digest: PayloadHash,
    pub destination: PathBuf,
}

pub(in crate::workspace) struct Observation {
    path: PathBuf,
    resolved: PathBuf,
    entry: PayloadHash,
    target_content: Option<PayloadHash>,
}

impl FileCheckInputs {
    fn captured_path(&self, root: &Path) -> Result<PathBuf, ExecError> {
        if self.captured.as_os_str().is_empty()
            || self
                .captured
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !self.destination.is_absolute()
        {
            return Err(file_failure(
                &self.span,
                "invalid retained file-check subject path",
            ));
        }
        Ok(root.join(&self.captured))
    }

    pub(in crate::workspace) fn path(&self, root: &Path) -> Result<PathBuf, ExecError> {
        let captured = self.captured_path(root)?;
        if !std::fs::symlink_metadata(&captured)?.is_file()
            || store::canonical_file_hash(&captured)? != self.captured_digest
        {
            return Err(file_failure(
                &self.span,
                "captured file-check subject bytes or mode changed",
            ));
        }
        Ok(if self.subject == FileCheckSubject::Deployed {
            self.destination.clone()
        } else {
            captured
        })
    }

    pub(in crate::workspace) fn observe(&self, root: &Path) -> Result<Observation, ExecError> {
        let path = if self.subject == FileCheckSubject::Deployed {
            self.path(root)?
        } else {
            self.captured_path(root)?
        };
        let observation =
            Observation::capture(path).map_err(|error| file_failure(&self.span, error))?;
        if self.subject != FileCheckSubject::Deployed
            && (observation.target_content.is_some() || observation.entry != self.captured_digest)
        {
            return Err(file_failure(
                &self.span,
                "captured file-check subject bytes or mode changed",
            ));
        }
        Ok(observation)
    }
}

impl Observation {
    fn capture(path: PathBuf) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("file-check subject has no parent"))?;
        if parent.canonicalize()? != parent {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file-check destination parent changed its canonical identity",
            ));
        }
        let resolved = path.canonicalize()?;
        if !std::fs::symlink_metadata(&resolved)?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file-check subject is not a regular file",
            ));
        }
        // Both the destination entry (including symlink spelling) and the
        // dereferenced bytes/mode participate. Shared managed-block destinations
        // deliberately observe the whole live file, never a cached merged body.
        let entry = store::canonical_file_hash(&path)?;
        let target_content = if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            Some(store::canonical_file_hash(&resolved)?)
        } else {
            None
        };
        Ok(Self {
            path,
            resolved,
            entry,
            target_content,
        })
    }

    pub(in crate::workspace) fn path(&self) -> &Path {
        &self.path
    }

    pub(in crate::workspace) fn unchanged(&self) -> io::Result<()> {
        let after = Self::capture(self.path.clone())?;
        if after.resolved != self.resolved
            || after.entry != self.entry
            || after.target_content != self.target_content
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file-check subject changed during validation",
            ));
        }
        Ok(())
    }
}
