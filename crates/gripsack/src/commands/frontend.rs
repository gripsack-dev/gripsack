//! The only evaluator launcher borrows approved captured source and retained
//! runtime bytes. It cannot rediscover a live worktree, pin or executable.
use super::prepared::ApprovedEvaluation;
use gripsack_ir::{Diagnostic, diagnostic::codes};
use gripsack_process::{
    Boundary, Control, Invocation, Limits, NativeInput, NativeOutcome, ProcessRole, Ruleset,
};
use std::{ffi::OsStr, io, path::Path};

pub(super) struct Frontend<'a> {
    approved: &'a ApprovedEvaluation<'a>,
}

pub(super) struct FrontendExecution {
    pub stdout: Vec<u8>,
    pub outcome: NativeOutcome,
}

pub(super) enum FrontendRunError {
    Grant(Diagnostic),
    Process(io::Error),
}

impl<'a> Frontend<'a> {
    pub(super) fn new(approved: &'a ApprovedEvaluation<'a>) -> Self {
        Self { approved }
    }

    pub(super) fn run_bounded(
        &self,
        inputs: &Path,
        deadline: std::time::Instant,
    ) -> Result<FrontendExecution, FrontendRunError> {
        let sources = self.approved.sources();
        let mut reads = String::new();
        for &kind in &self.approved.policy().read_roots {
            let path = sources.captured_root(kind).ok_or_else(|| {
                FrontendRunError::Process(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "approved source root is missing",
                ))
            })?;
            append_read_grant(&mut reads, "captured source", path)
                .map_err(FrontendRunError::Grant)?;
        }
        // Grant the immutable round FILE, never a directory containing another
        // command's facts or an earlier/later round's input envelope.
        append_read_grant(&mut reads, "host input", inputs).map_err(FrontendRunError::Grant)?;
        let reads = format!("--allow-read={reads}");
        let import_map = format!(
            "--import-map={}",
            sources.frontend().join("deno.json").display()
        );
        let driver = sources.frontend().join("src/cli.ts");
        // The OS filesystem boundary, not Deno's flags alone, decides what the
        // evaluated process tree may read: captured roots, this round's input
        // directory, the evaluator's private cache/scratch, and the selected
        // runtime's own load roots. Everything else — ambient `node_modules`
        // ancestors included — is denied by the kernel.
        let mut boundary = Boundary::new();
        for &kind in &self.approved.policy().read_roots {
            let path = sources.captured_root(kind).ok_or_else(|| {
                FrontendRunError::Process(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "approved source root is missing",
                ))
            })?;
            boundary = boundary
                .read_beneath(path)
                .map_err(FrontendRunError::Process)?;
        }
        let inputs_directory = inputs.parent().ok_or_else(|| {
            FrontendRunError::Process(io::Error::new(
                io::ErrorKind::InvalidData,
                "host input file has no parent directory",
            ))
        })?;
        boundary = boundary
            .read_beneath(inputs_directory)
            .map_err(FrontendRunError::Process)?;
        boundary = boundary
            .read_write_beneath(self.approved.evaluator_cache())
            .and_then(|boundary| boundary.read_write_beneath(self.approved.evaluator_tmp()))
            .map_err(FrontendRunError::Process)?;
        for root in self.approved.runtime_roots() {
            boundary = boundary
                .read_beneath(root)
                .map_err(FrontendRunError::Process)?;
        }
        // Executing a binary under Landlock requires reading it: an operator
        // script runtime commonly execs the pinned interpreter provisioned
        // under the runtime home, so that tree is readable by design.
        boundary = boundary
            .read_beneath_optional(&self.approved.runtime_home())
            .map_err(FrontendRunError::Process)?;
        // Executing a binary also requires reading it. The operator's PATH is
        // the executable space an operator-selected script runtime resolves
        // its real interpreter in, so those directories are readable by
        // design; locations outside every granted root fail closed.
        for directory in self.approved.environment().search_directories() {
            boundary = boundary
                .read_beneath_optional(directory)
                .map_err(FrontendRunError::Process)?;
        }
        // A `#!` runtime reads its script through this pid's own fd directory;
        // only the retained script descriptor and stdio survive exec.
        let ruleset =
            Ruleset::assemble(&boundary, self.approved.runtime().is_script()).map_err(|error| {
                FrontendRunError::Process(io::Error::new(
                    error.kind(),
                    format!("cannot confine the evaluator: {error}"),
                ))
            })?;
        let limits = Limits {
            operation_deadline: Some(deadline),
            ..Limits::default()
        };
        let invocation = Invocation::admit(
            self.approved.environment(),
            ProcessRole::Evaluator,
            self.approved.runtime(),
            sources.repository(),
            limits,
        )
        .map_err(FrontendRunError::Process)?
        .confine(ruleset);
        let arguments = [
            OsStr::new("run"),
            OsStr::new("--no-remote"),
            OsStr::new("--cached-only"),
            OsStr::new("--no-lock"),
            OsStr::new("--no-config"),
            OsStr::new("--node-modules-dir=manual"),
            OsStr::new(&import_map),
            OsStr::new(&reads),
            driver.as_os_str(),
            sources.repository().as_os_str(),
            OsStr::new("--inputs"),
            inputs.as_os_str(),
        ];
        let mut stdout = Vec::new();
        let outcome = invocation
            .run(&arguments, NativeInput::Bytes(b""), None, |bytes| {
                stdout.extend_from_slice(bytes);
                Control::Continue
            })
            .map_err(FrontendRunError::Process)?;
        Ok(FrontendExecution { stdout, outcome })
    }

    pub(super) fn map_diagnostics(&self, diagnostics: &mut [Diagnostic]) {
        self.approved.map_diagnostics(diagnostics);
    }

    pub(super) fn logical_text<'b>(&self, text: &'b str) -> std::borrow::Cow<'b, str> {
        self.approved.sources().logical_text(text)
    }
}

/// Deno's comma-separated permission syntax and UTF-8 command representation
/// must not silently split or alter a captured root or core input filename.
fn append_read_grant(output: &mut String, kind: &str, path: &Path) -> Result<(), Diagnostic> {
    let Some(path_text) = path.to_str().filter(|path| !path.contains(',')) else {
        return Err(Diagnostic::error(
            codes::UNSAFE_EVAL_READ_GRANT,
            format!(
                "{kind} path {} cannot be represented by a Deno read grant",
                path.to_string_lossy().escape_debug()
            ),
        )
        .with_help("use UTF-8 capture/input directory paths without commas"));
    };
    if !output.is_empty() {
        output.push(',');
    }
    output.push_str(path_text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn unrepresentable_read_grants_cannot_extend_the_admitted_roots() {
        for path in [
            Path::new("/private/bad,grant"),
            Path::new(OsStr::from_bytes(b"/private/non-\xff-utf8")),
        ] {
            let mut roots = String::from("/private/captured");
            let diagnostic = append_read_grant(&mut roots, "captured source", path).unwrap_err();
            assert_eq!(diagnostic.code, codes::UNSAFE_EVAL_READ_GRANT);
            assert_eq!(roots, "/private/captured");
        }
    }
}
