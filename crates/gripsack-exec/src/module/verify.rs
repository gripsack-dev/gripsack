//! Receipt-based module verification (0035 F2, 0039). Store-only modules keep
//! payload receipts but cannot earn destination-verification receipts.

use super::ModuleRun;
use crate::ctx::ExecError;
use crate::report::{ReportKind, StepReport, describe_verify};
use crate::util::progress;
use crate::verify::run_verify;
use gripsack_ir::Verify;

impl<'a> ModuleRun<'a> {
    // Return IR-owned references, not references to the phase machine: reporting
    // a check must not require cloning its argv/script just to release a borrow.
    fn checks(&self) -> (Vec<&'a Verify>, bool) {
        let mut skipped_destination = false;
        let checks = self
            .plan
            .checks()
            .filter(|verify| {
                if self.build_only && matches!(verify, Verify::FileDeployed { .. }) {
                    skipped_destination = true;
                    false
                } else {
                    true
                }
            })
            .collect();
        (checks, skipped_destination)
    }

    pub(super) fn verify(&mut self) -> Result<(), ExecError> {
        let (checks, skipped_destination) = self.checks();
        if skipped_destination {
            self.reports.push(StepReport {
                module: self.name.to_owned(),
                summary: "destination verification omitted: build-only dependencies do not deploy"
                    .into(),
                kind: ReportKind::Satisfied,
            });
        }
        if checks.is_empty() {
            return Ok(());
        }
        let mut fingerprints: Vec<String> = checks
            .iter()
            .map(|verify| {
                let json = serde_json::to_vec(verify).expect("verify specs serialize");
                gripsack_store::hash::hex_sha256(&json)
            })
            .collect();
        fingerprints.sort();
        let receipt_holds = self.prev_module.is_some_and(|m| {
            m.store_path == self.store_path
                && m.build_only == self.build_only
                && m.verified.as_deref() == Some(fingerprints.as_slice())
                && m.entries.len() == self.deployed.len()
                && m.entries.iter().zip(&self.deployed).all(|(a, b)| {
                    // Lineage is not content; preserved drift is part of the
                    // produced state, just as it is for deploying modules.
                    a.from == b.from
                        && a.to == b.to
                        && a.mode == b.mode
                        && a.hash == b.hash
                        && a.file_mode == b.file_mode
                        && a.preserved_drift == b.preserved_drift
                })
        });
        if !receipt_holds {
            for verify in checks {
                let _step = tracing::info_span!("step", step = "verify").entered();
                progress(self.ctx, self.name, "verifying");
                run_verify(self.name, verify, &self.store_path, self.version.as_deref())?;
                self.reports.push(StepReport {
                    module: self.name.to_owned(),
                    summary: describe_verify(verify, self.version.as_deref()).map_err(|error| {
                        ExecError::Verify {
                            module: self.name.into(),
                            detail: error.to_string(),
                        }
                    })?,
                    kind: ReportKind::Verified,
                });
            }
        }
        self.verified = Some(fingerprints);
        Ok(())
    }
}
