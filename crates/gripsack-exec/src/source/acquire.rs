//! Source acquisition and pin construction shared by update and apply.

use crate::{
    ctx::{Ctx, ExecError},
    lockfile::{LockEntry, Resolved},
};
use gripsack_ir::FetchSpec;
use std::path::Path;

pub(crate) struct FetchInputs<'a> {
    pub name: &'a str,
    pub spec: &'a FetchSpec,
    pub locked: Option<&'a LockEntry>,
    pub dest: &'a Path,
}

pub(crate) fn fetch(ctx: &Ctx, inputs: FetchInputs<'_>) -> Result<LockEntry, ExecError> {
    let FetchInputs {
        name,
        spec,
        locked,
        dest,
    } = inputs;
    let locked = locked.filter(|entry| entry.fetch == *spec);
    let (concrete, meta) = crate::resolve::resolve_spec(name, spec, locked, &ctx.fetch)?;
    let previous = locked.and_then(|entry| entry.resolved.as_ref());
    let locked_json = previous.map(serde_json::to_value).transpose()?;
    let outcome = ctx.fetch.fetch(&concrete, dest, locked_json.as_ref())?;
    if let Some(expected) = previous.and_then(|pin| pin.sha256.as_ref())
        && expected != outcome.identity.as_str()
    {
        return Err(ExecError::Fetch(gripsack_fetch::FetchError::HashMismatch {
            url: format!("{name} payload"),
            expected: expected.clone(),
            actual: outcome.identity.into(),
        }));
    }
    let pin = Resolved {
        url: meta
            .as_ref()
            .map(|meta| meta.url.clone())
            .or(outcome.url)
            .or_else(|| previous.and_then(|pin| pin.url.clone())),
        version: meta
            .as_ref()
            .map(|meta| meta.version.clone())
            .or(outcome.version)
            .or_else(|| match &concrete {
                FetchSpec::Git { rev, .. } => rev.clone(),
                _ => None,
            })
            .or_else(|| previous.and_then(|pin| pin.version.clone())),
        sha256: Some(outcome.identity.into()),
        api_url: meta
            .as_ref()
            .and_then(|meta| meta.api_url.clone())
            .or_else(|| previous.and_then(|pin| pin.api_url.clone())),
        repo256: None, // finalized from the actual captured overlay, not a later repo read
        // Source-only tree identity is filled after the same overlay merge in
        // both commands. Build recipes deliberately have no output-tree pin.
        tree256: None,
    };
    Ok(LockEntry {
        fetch: spec.clone(),
        resolved: Some(pin),
    })
}

pub(crate) fn publish(
    ctx: &Ctx,
    name: &str,
    stage: &Path,
    destination: &Path,
) -> Result<(), ExecError> {
    let relative = destination
        .strip_prefix(&ctx.home)
        .map_err(|_| ExecError::Step {
            module: name.into(),
            step: "publish".into(),
            detail: format!(
                "store path {} is not under $GRIPSACK_HOME",
                destination.display()
            ),
        })?;
    gripsack_fs::publish_dir(ctx.home_dir()?, stage, relative)?;
    Ok(())
}
