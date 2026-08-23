//! Verify: smoke contracts, pre-flip (0007 §verify).

use crate::ctx::ExecError;
use crate::deploy::expand_home;
use gripsack_ir::Verify;
use std::path::Path;

pub(crate) fn run_verify(
    name: &str,
    verify: &Verify,
    store_path: &Path,
    version: Option<&str>,
) -> Result<(), ExecError> {
    let fail = |detail: String| ExecError::Verify {
        module: name.to_string(),
        detail,
    };
    let subst = |p: &String| match version {
        Some(v) => p.replace("{version}", v),
        None => p.clone(),
    };
    match verify {
        Verify::BinaryRuns { path, args } => {
            let bin = store_path.join(subst(path));
            let status = std::process::Command::new(&bin)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|e| fail(format!("cannot run {}: {e}", bin.display())))?;
            if !status.success() {
                return Err(fail(format!("{} exited {status}", bin.display())));
            }
        }
        Verify::FileExists { path } => {
            if !store_path.join(subst(path)).exists() {
                return Err(fail(format!("{} missing in payload", path)));
            }
        }
        Verify::FileDeployed { path } => {
            if !expand_home(path).exists() {
                return Err(fail(format!("{} not deployed", path)));
            }
        }
        Verify::Shell { script } => run_shell(script, store_path).map_err(fail)?,
    }
    Ok(())
}

pub(crate) fn run_shell(script: &str, cwd: &Path) -> Result<(), String> {
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("sh -c exited {status}"))
    }
}
