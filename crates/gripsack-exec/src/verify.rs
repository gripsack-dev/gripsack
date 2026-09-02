//! Verify: smoke contracts, pre-flip (0007 §verify).

use crate::ctx::ExecError;
use gripsack_ir::Verify;
use gripsack_store::expand_home;
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
    // {version} is the locked tag; the platform placeholders (0016 §D1)
    // come from this machine's facts — one substitution surface for
    // verify keys and deploy's install keys
    let subst = |p: &String| {
        let expanded = gripsack_fetch::expand_platform(p);
        match version {
            Some(v) => expanded.replace("{version}", v),
            None => expanded,
        }
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
