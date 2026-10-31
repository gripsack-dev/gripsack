//! Host-bound, explicit execution of captured declarations. No evaluator or
//! alternate planner lives here: admission precedes the existing held session.
mod observations;
#[cfg(test)]
mod tests;

use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_store as store;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const FORMAT_VERSION: u32 = 1;
const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    digest: String,
    binding: Binding,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    repository: PathBuf,
    home: PathBuf,
    source: String,
    policy: String,
    evaluation: store::trust::evaluation::EvaluationId,
    host: String,
    selected: Vec<String>,
    frontend: PathBuf,
    pinned_frontend: Option<PathBuf>,
    exclusions: Vec<String>,
    /// Exact admitted declarations, not a promise of unknown realized bytes.
    ir: String,
    facts: serde_json::Value,
    probes: BTreeMap<String, bool>,
    state: observations::State,
}

/// Only a bounded, digest-checked envelope constructs this value.
pub struct SavedPlan {
    binding: Binding,
}

fn invalid(message: impl Into<String>) -> ExecError {
    io::Error::new(io::ErrorKind::InvalidData, message.into()).into()
}
fn digest(bytes: &[u8]) -> String {
    gripsack_process::Sha256Digest::of(bytes).to_string()
}
fn require_equal<T: PartialEq>(before: &T, after: &T, subject: &str) -> Result<(), ExecError> {
    if before != after {
        return Err(invalid(format!(
            "stale saved plan: {subject} changed; create a new plan"
        )));
    }
    Ok(())
}

impl SavedPlan {
    /// Explicit live planning only. Caller supplies the completed evaluation;
    /// preview operations are obtained from the production offline planner.
    pub fn capture(ir_json: &str, ctx: &Ctx) -> Result<Self, ExecError> {
        if ctx.take_over || ctx.take_over_entries.is_some() {
            return Err(invalid("saved plans cannot retain take-over authority"));
        }
        let crate::Repository::Evaluated {
            sources, receipt, ..
        } = &ctx.repository
        else {
            return Err(invalid(
                "saving requires explicit live planning with approved captured source",
            ));
        };
        let receipt_record = store::trust::evaluation::read(&ctx.home, *receipt)?;
        let mut ir = admit_ir(ir_json)?;
        crate::rebind_captured_ir(&mut ir, sources)?;
        let ir_json = serde_json::to_string(&ir)?;
        let ir = admit_ir(&ir_json)?;
        let inputs = ctx
            .repository
            .evaluation_inputs()
            .ok_or_else(|| invalid("saving requires the retained trusted evaluation inputs"))?;
        let _session = LifecycleSession::acquire(&ctx.home)?;
        let binding = Binding {
            repository: sources.repository_identity().to_path_buf(),
            home: std::path::absolute(&ctx.home)?,
            source: sources.digest().to_string(),
            policy: receipt_record.policy_digest.to_string(),
            evaluation: *receipt,
            host: ctx.host.to_string(),
            frontend: sources
                .original_root(store::source_bundle::SourceRootKind::Frontend)
                .ok_or_else(|| invalid("captured frontend identity is missing"))?
                .to_path_buf(),
            pinned_frontend: sources
                .original_root(store::source_bundle::SourceRootKind::PinnedFrontend)
                .map(Path::to_path_buf),
            exclusions: sources.capture_policy().exclusions().to_vec(),
            selected: ctx.only.clone(),
            ir: ir_json,
            facts: serde_json::to_value(inputs.facts())?,
            probes: inputs.probes().clone(),
            state: observations::State::capture(&ir, ctx)?,
        };
        Ok(Self { binding })
    }

    pub fn load(path: &Path) -> Result<Self, ExecError> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no preconditions and returns the caller identity.
        let owner = unsafe { libc::geteuid() };
        if !metadata.is_file()
            || metadata.uid() != owner
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(invalid("saved plans must have private (0600) permissions"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("saved plan exceeds byte limit"));
        }
        let plan = Self::decode(&bytes)?;
        let directory = gripsack_fs::open(&plan.binding.home.join("saved-plans"))?;
        let name = digest(&serde_json::to_vec(&plan.binding)?);
        let mut retained = Vec::new();
        directory
            .open(Path::new(&name))?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut retained)?;
        require_equal(&retained, &bytes, "private local admission record")?;
        Ok(plan)
    }

    fn decode(bytes: &[u8]) -> Result<Self, ExecError> {
        let value = crate::semantic::admission::parse_json(bytes)
            .map_err(|error| invalid(error.to_string()))?;
        let envelope: Envelope = serde_json::from_value(value)?;
        if envelope.version != FORMAT_VERSION {
            return Err(invalid("unsupported saved-plan version"));
        }
        require_equal(
            &envelope.digest,
            &digest(&serde_json::to_vec(&envelope.binding)?),
            "envelope digest",
        )?;
        admit_ir(&envelope.binding.ir)?;
        Ok(Self {
            binding: envelope.binding,
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), ExecError> {
        use std::os::unix::fs::OpenOptionsExt;
        let binding_bytes = serde_json::to_vec(&self.binding)?;
        #[derive(Serialize)]
        struct Wire<'a> {
            version: u32,
            digest: String,
            binding: &'a Binding,
        }
        let bytes = serde_json::to_vec(&Wire {
            version: FORMAT_VERSION,
            digest: digest(&binding_bytes),
            binding: &self.binding,
        })?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("saved plan exceeds byte limit"));
        }
        // Never follow an existing symlink or overwrite another review artifact.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        let home = gripsack_fs::open_or_create(&self.binding.home)?;
        match home.create_dir("saved-plans") {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let directory = gripsack_fs::open_dir_nofollow(&home, Path::new("saved-plans"))?;
        use gripsack_fs::cap_std::fs::PermissionsExt;
        directory.set_permissions(".", gripsack_fs::cap_std::fs::Permissions::from_mode(0o700))?;
        gripsack_fs::atomic_write_with_mode(
            &directory,
            Path::new(&digest(&binding_bytes)),
            &bytes,
            0o600,
        )?;
        Ok(())
    }

    pub fn host(&self) -> &str {
        &self.binding.host
    }
    pub fn selected(&self) -> &[String] {
        &self.binding.selected
    }
    pub fn ir_json(&self) -> &str {
        &self.binding.ir
    }
    pub fn probes(&self) -> &BTreeMap<String, bool> {
        &self.binding.probes
    }
    pub fn evaluation(&self) -> store::trust::evaluation::EvaluationId {
        self.binding.evaluation
    }
    pub fn existing_runtime(home: &Path) -> io::Result<PathBuf> {
        crate::frontend::existing_deno(home)
    }

    /// Reject changed source/locks before runtime preparation can provision tools.
    pub fn preflight(&self, repo: &Path, home: &Path) -> Result<(), ExecError> {
        require_equal(
            &self.binding.repository,
            &repo.canonicalize()?,
            "repository",
        )?;
        require_equal(
            &self.binding.home,
            &std::path::absolute(home)?,
            "runtime home",
        )?;
        let policy =
            store::source_bundle::SourceCapturePolicy::new(self.binding.exclusions.clone())?;
        let sources = store::source_bundle::SourceBundle::capture(
            repo,
            &self.binding.frontend,
            self.binding.pinned_frontend.as_deref(),
            home,
            policy,
        )?;
        require_equal(
            &self.binding.source,
            &sources.digest().to_string(),
            "source or lock",
        )
    }
    pub fn revalidate(
        &self,
        ctx: &Ctx,
        policy: &str,
        facts: &serde_json::Value,
        probes: &BTreeMap<String, bool>,
    ) -> Result<(), ExecError> {
        let crate::Repository::Evaluated { sources, .. } = &ctx.repository else {
            return Err(invalid("saved execution requires captured sources"));
        };
        require_equal(
            &self.binding.repository,
            &sources.repository_identity().to_path_buf(),
            "repository",
        )?;
        require_equal(
            &self.binding.home,
            &std::path::absolute(&ctx.home)?,
            "runtime home",
        )?;
        require_equal(
            &self.binding.source,
            &sources.digest().to_string(),
            "source or lock",
        )?;
        require_equal(
            &self.binding.policy,
            &policy.to_owned(),
            "evaluation policy",
        )?;
        require_equal(&self.binding.host, &ctx.host.to_string(), "host")?;
        require_equal(&self.binding.selected, &ctx.only, "selected scope")?;
        require_equal(&self.binding.facts, facts, "host facts")?;
        require_equal(&self.binding.probes, probes, "recorded probes")?;
        if ctx.take_over || ctx.take_over_entries.is_some() {
            return Err(invalid("saved execution has no take-over grant"));
        }
        let ir = admit_ir(&self.binding.ir)?;
        let receipt = store::trust::evaluation::read(&ctx.home, self.binding.evaluation)?;
        if store::trust::status(&ctx.home, sources, &receipt.policy)?
            != store::trust::ApprovalStatus::Approved
        {
            return Err(invalid(
                "stale saved plan: source approval is no longer current",
            ));
        }
        self.binding.state.revalidate(&ir, ctx)
    }

    pub fn apply(
        &self,
        ctx: &Ctx,
        policy: &str,
        facts: &serde_json::Value,
        probes: &BTreeMap<String, bool>,
        prepare: impl FnOnce() -> Result<(), ExecError>,
    ) -> Result<crate::report::ApplyResult, ExecError> {
        let session = LifecycleSession::acquire(&ctx.home)?;
        self.preflight(ctx.repository.identity(), &ctx.home)?;
        self.revalidate(ctx, policy, facts, probes)?;
        prepare()?;
        let ir = admit_ir(&self.binding.ir)?;
        crate::apply::apply_in_session(&ir, ctx, session)
    }
}

fn admit_ir(json: &str) -> Result<gripsack_ir::Ir, ExecError> {
    crate::semantic::admission::parse_json(json.as_bytes())
        .map_err(|error| invalid(error.to_string()))?;
    gripsack_ir::check(json).map_err(|_| invalid("saved declarations failed current IR admission"))
}
