//! A timer never follows a mutable core installation or an ambient shell.
//! Release runners are static ELF; dynamically linked development executables
//! cannot promise retained loader/library closure and are refused explicitly.
use crate::{Ctx, ExecError};
use gripsack_process::executable::{self, ExecutableArch, ExecutableFormat};
use std::{io, path::PathBuf};

pub(super) fn retain(ctx: &Ctx) -> Result<(PathBuf, PathBuf), ExecError> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let mut input = std::fs::File::open(&executable)?;
    let metadata = executable::classify(&mut input).map_err(io::Error::other)?;
    if metadata.format != Some(ExecutableFormat::Elf)
        || metadata.arch != Some(ExecutableArch::X86_64)
        || metadata.interpreter.is_some()
        || !metadata.needed_libraries.is_empty()
    {
        return Err(io::Error::new(io::ErrorKind::Unsupported,
            "scheduled jobs require the static Linux x86_64 release runner; a development binary with ambient loader/libraries cannot be retained safely").into());
    }
    let temporary = tempfile::tempdir()?;
    let runner = temporary.path().join("grip");
    std::fs::copy(&executable, &runner)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o500))?;
    let tree = gripsack_store::canonical_tree_hash(temporary.path())?;
    let root = gripsack_store::content_path(&ctx.home, "job-runner", tree.as_str());
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.is_dir() => {
            if gripsack_store::canonical_tree_hash(&root)? != tree {
                return Err(io::Error::other("retained scheduler runner identity differs").into());
            }
        }
        Ok(_) => {
            return Err(io::Error::other("retained scheduler runner is not a directory").into());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            crate::source::publish(ctx, "scheduled-runner", temporary.path(), &root)?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok((root.join("grip"), root))
}
