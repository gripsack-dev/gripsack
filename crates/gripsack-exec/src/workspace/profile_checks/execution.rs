//! Native check supervision owns a durable process lease and its private input
//! copy. Unknown cleanup keeps both, including after the preparation TempDir drops.
use super::Protection;
use crate::{
    ExecError,
    workspace::{file_failure, hooks, roots},
};
use gripsack_ir::Span;
use gripsack_process::{Control, OperatorEnvironment, ProcessDisposition, ProcessLeases};
use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    context: &hooks::snapshot::Context,
    root: &Path,
    home: &Path,
    environment: &OperatorEnvironment,
    deadline: Instant,
    span: &Span,
    protection: &Protection<'_>,
) -> Result<(), ExecError> {
    let lease = roots::ProcessLease::register(protection.session, protection.closure)?;
    let directory = gripsack_fs::open(home)?;
    let staging = match stage(context, root, home, lease.id()) {
        Ok(staging) => staging,
        Err(error) => {
            roots::remove_native_staging(&directory, lease.id())?;
            lease.release_unlaunched(protection.session)?;
            return Err(file_failure(span, error));
        }
    };
    let handle = match lease.duplicate_handle() {
        Ok(handle) => handle,
        Err(error) => {
            roots::remove_native_staging(&directory, lease.id())?;
            lease.release_unlaunched(protection.session)?;
            return Err(error.into());
        }
    };
    let outcome = hooks::run_context(
        context,
        &staging,
        home,
        None,
        environment,
        deadline,
        Some(ProcessLeases {
            worker: None,
            retention: Some(handle),
        }),
        |_| Control::Continue,
    )
    .map_err(|error| {
        file_failure(
            span,
            format!(
                "file check failed; retained process {} and exact inputs at {}: {error}",
                lease.id().as_str(),
                staging.display()
            ),
        )
    })?;
    if outcome.receipt.disposition == ProcessDisposition::CleanupFailure {
        let receipt = staging.join("process.json");
        std::fs::write(&receipt, serde_json::to_vec(&outcome.receipt)?)?;
        std::fs::File::open(&receipt)?.sync_all()?;
        gripsack_fs::fsync_pinned_dir(&gripsack_fs::open(&staging)?, &staging)?;
        return Err(file_failure(
            span,
            format!(
                "file check cleanup is unconfirmed; process {} and exact inputs at {} retained for recovery",
                lease.id().as_str(),
                staging.display()
            ),
        ));
    }
    // The shared supervisor's terminal receipt is the only cleanup authority.
    roots::remove_native_staging(&directory, lease.id())?;
    lease.release(protection.session, &outcome.receipt)?;
    if !outcome.success {
        return Err(file_failure(
            span,
            format!(
                "file check failed ({:?}, exit {:?}): {}",
                outcome.receipt.disposition,
                outcome.receipt.exit_code,
                String::from_utf8_lossy(&outcome.stderr)
            ),
        ));
    }
    Ok(())
}

fn stage(
    context: &hooks::snapshot::Context,
    root: &Path,
    home: &Path,
    id: &roots::RootId,
) -> Result<PathBuf, ExecError> {
    let inputs = context
        .file_check
        .as_ref()
        .ok_or_else(|| io::Error::other("missing file-check inputs"))?;
    // Admission checks the original captured file even for a deployed subject.
    inputs.path(root)?;
    let directory = gripsack_fs::open(home)?;
    gripsack_fs::create_dir_all(&directory, Path::new(roots::BUILD_STAGING_DIRECTORY))?;
    let parent =
        gripsack_fs::open_dir_nofollow(&directory, Path::new(roots::BUILD_STAGING_DIRECTORY))?;
    parent.create_dir(id.as_str())?;
    let staging = home.join(roots::BUILD_STAGING_DIRECTORY).join(id.as_str());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700))?;
    }
    let target = staging.join(&inputs.captured);
    let target_parent = target
        .parent()
        .ok_or_else(|| io::Error::other("file subject has no parent"))?;
    std::fs::create_dir_all(target_parent)?;
    // A copy, not a hardlink: a hostile check must not edit the candidate through
    // its validation input. The digest includes normalized executable mode.
    std::fs::copy(root.join(&inputs.captured), &target)?;
    inputs.path(&staging)?;
    std::fs::File::open(&target)?.sync_all()?;
    let context_path = staging.join("context.json");
    std::fs::write(&context_path, context.encode()?)?;
    std::fs::File::open(&context_path)?.sync_all()?;
    let mut cursor = target_parent;
    loop {
        gripsack_fs::fsync_pinned_dir(&gripsack_fs::open(cursor)?, cursor)?;
        if cursor == staging {
            break;
        }
        cursor = cursor
            .parent()
            .ok_or_else(|| io::Error::other("file subject escaped staging"))?;
    }
    gripsack_fs::fsync_pinned_dir(&parent, Path::new(roots::BUILD_STAGING_DIRECTORY))?;
    Ok(staging)
}
