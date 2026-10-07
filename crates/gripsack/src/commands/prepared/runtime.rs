//! Runtime authority is exact files, not the operator's executable directories.
use gripsack_process::{OperatorEnvironment, RuntimeAccess};
use gripsack_store::source_bundle::SourceBundle;
use std::{io, path::Path};

pub(super) fn access(
    sources: &SourceBundle,
    environment: &OperatorEnvironment,
    program: &Path,
    home: &Path,
) -> io::Result<RuntimeAccess> {
    let admit = |path: &Path| sources.admit_evaluator_root(path).map(drop);
    let mut access = RuntimeAccess::discover(environment, program, admit)?;
    if access.is_script() {
        // Instrumented/operator wrappers execute the real engine from PATH or
        // the standard provisioned location. Admit that named program's exact
        // files and loader dependencies, never all PATH or tools descendants.
        let engine = match environment.resolve(Path::new("deno")) {
            Ok(selected) => selected.declared().to_owned(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => home.join("tools")
                .join(format!("deno-{}", gripsack_fetch::DENO_RELEASE.version)).join("deno"),
            Err(error) => return Err(error),
        };
        access.add_program(environment, &engine, admit)?;
    }
    Ok(access)
}
