//! Diagnostic capture is bounded and distinct from mandatory hook receipts.
//! Rendering occurs after the result is durable, outside child supervision.
use gripsack_process::{Control, NativeOutcome, ProcessDisposition, ProcessReceipt};
use std::io::{self, Write};

const STDOUT_BYTES: usize = 16 * 1024 * 1024;
const STDERR_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct CapturedOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

impl CapturedOutput {
    pub(super) fn stdout(&mut self, bytes: &[u8]) -> Control {
        let remaining = STDOUT_BYTES - self.stdout.len();
        let retained = remaining.min(bytes.len());
        self.stdout.extend_from_slice(&bytes[..retained]);
        if retained < bytes.len() {
            self.stdout_truncated = true;
        }
        Control::Continue
    }

    pub(super) fn result(
        &mut self,
        outcome: NativeOutcome,
        receipts: &mut Vec<ProcessReceipt>,
    ) -> bool {
        self.stdout_truncated |= outcome.receipt.disposition == ProcessDisposition::StdoutLimit;
        self.stderr_truncated |= outcome.stderr_truncated
            || outcome.receipt.disposition == ProcessDisposition::StderrLimit;
        let mut stderr = outcome.stderr;
        if stderr.len() > STDERR_BYTES {
            let start = stderr.len() - STDERR_BYTES;
            stderr.copy_within(start.., 0);
            stderr.truncate(STDERR_BYTES);
            self.stderr_truncated = true;
        }
        if self.stderr.is_empty() {
            self.stderr = stderr;
        } else {
            let keep = self.stderr.len().min(STDERR_BYTES - stderr.len());
            self.stderr_truncated |= keep != self.stderr.len();
            let start = self.stderr.len() - keep;
            self.stderr.copy_within(start.., 0);
            self.stderr.truncate(keep);
            self.stderr.extend_from_slice(&stderr);
        }
        receipts.push(outcome.receipt);
        outcome.success
    }

    pub(super) fn emit(self) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        gripsack_process::terminal::write_output(&mut stdout, &self.stdout)?;
        if self.stdout_truncated {
            stdout.write_all(b"\n[hook stdout truncated]\n")?;
        }
        let mut stderr = io::stderr().lock();
        if self.stderr_truncated {
            stderr.write_all(b"[hook stderr truncated; retained tail follows]\n")?;
        }
        gripsack_process::terminal::write_output(&mut stderr, &self.stderr)
    }
}
