//! Workspace-owned persistent containers. Docker supervises processes; this
//! module owns explicit start/stop intent, readiness and resource attribution.
//! Database volumes are never removed by Gripsack, including after failures.
mod engine;
mod plan;
mod state;
#[cfg(test)]
mod tests;

use crate::{Ctx, ExecError, LifecycleSession};
use engine::Engine;
use gripsack_ir::Ir;
use reqwest::Method;
use serde::Serialize;
use serde_json::json;
use state::{ServiceRecord, ServiceState, WorkspaceServiceId};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("service: {0}")]
    Refused(String),
    #[error("service state: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid service metadata: {0}")]
    Json(#[from] serde_json::Error),
}
fn failure(message: impl Into<String>) -> ServiceError {
    ServiceError::Refused(message.into())
}
fn exec_error(error: ServiceError) -> ExecError {
    ExecError::Step {
        output: "services".into(),
        step: "readiness".into(),
        detail: error.to_string(),
    }
}

#[derive(Debug, Serialize)]
pub struct ServiceStatus {
    pub name: String,
    pub revision: String,
    pub container: Option<String>,
    pub state: String,
}

/// Explicit startup authority. All declarations, inherited values, ownership
/// collisions and pinned images are checked before a container is started.
pub fn up(
    ir: &Ir,
    ctx: &Ctx,
    socket: Option<&Path>,
    deadline: Instant,
) -> Result<Vec<ServiceStatus>, ServiceError> {
    let selected = plan::selection(ir, &ctx.only)?;
    if selected.is_empty() {
        return Err(failure("workspace declares no selected services"));
    }
    // Resolve only explicitly declared environment references. This effect is
    // outside evaluation and no inherited value enters a Gripsack receipt.
    let mut inherited = std::collections::BTreeMap::new();
    for service in &selected {
        for binding in service.env.values() {
            if let gripsack_ir::workspace_model::ServiceEnvValue::Inherited { name } = binding
                && !inherited.contains_key(name)
            {
                let value = std::env::var(name).map_err(|_| {
                    failure(format!(
                        "required service variable {name:?} is absent or not UTF-8"
                    ))
                })?;
                if value.len() > gripsack_ir::workspace_model::MAX_SERVICE_VALUE_BYTES
                    || value.contains('\0')
                {
                    return Err(failure(
                        "inherited service value exceeds its bound or contains NUL",
                    ));
                }
                inherited.insert(name.clone(), value);
            }
        }
    }
    crate::workspace::admit_service_pins(ir, ctx, &ctx.only)
        .map_err(|error| failure(error.to_string()))?;
    let session = LifecycleSession::acquire(&ctx.home)?;
    let identity = WorkspaceServiceId::for_context(ctx)?;
    let existing = ServiceState::read(ctx, &identity)?;
    let endpoint = socket.or_else(|| existing.as_ref().map(|state| state.socket.as_path()));
    let engine = Engine::connect(endpoint, deadline)?;
    let mut state = match existing {
        Some(state) => {
            state.admit_engine(&engine)?;
            state
        }
        None => ServiceState::new(identity, &engine)?,
    };
    let configurations: Vec<_> = selected
        .iter()
        .map(|service| plan::configuration(service, &state, &inherited))
        .collect::<Result<_, _>>()?;
    // Observe every existing container before publishing new intent. A changed
    // configuration requires an explicit down/up; no surprise database restart.
    for service in &selected {
        let name = state.workspace.resource("service", &service.name);
        let observed = engine.get(&["containers", &name, "json"])?;
        let record = state
            .services
            .iter()
            .find(|record| record.name == service.name);
        if let Some(observed) = observed {
            let record = record.ok_or_else(|| {
                failure("container name collision without an owned creation intent")
            })?;
            plan::admit_container(&observed, &state, record)?;
            if record.revision != plan::revision(service)? {
                return Err(failure(format!(
                    "service {:?} changed; run grip down for it before grip up (volumes are retained)",
                    service.name
                )));
            }
        }
    }
    // Acquisition may fail, but leaves an existing running service unchanged.
    for service in &selected {
        engine.ensure_image(&service.image)?;
        let image = engine
            .get(&["images", &service.image, "json"])?
            .ok_or_else(|| failure("pinned image disappeared"))?;
        if let Some(volumes) = image["Config"]["Volumes"].as_object()
            && volumes.keys().any(|target| {
                !service
                    .volumes
                    .iter()
                    .any(|volume| &volume.target == target)
            })
        {
            return Err(failure(format!(
                "service {:?}: declare a named volume for every image VOLUME; anonymous data volumes are not created",
                service.name
            )));
        }
    }
    state.persist(ctx, &session)?;
    ensure_network(&engine, &state)?;
    for service in &selected {
        for volume in &service.volumes {
            ensure_volume(&engine, &state, &volume.name)?;
        }
    }
    for (service, configuration) in selected.iter().zip(&configurations) {
        let name = state.workspace.resource("service", &service.name);
        let revision = plan::revision(service)?;
        let index = match state
            .services
            .iter()
            .position(|record| record.name == service.name)
        {
            Some(index) => index,
            None => {
                state.services.push(ServiceRecord {
                    name: service.name.clone(),
                    revision,
                    container: None,
                });
                state.services.len() - 1
            }
        };
        let existing = engine.get(&["containers", &name, "json"])?;
        let container = if let Some(existing) = existing {
            plan::admit_container(&existing, &state, &state.services[index])?
        } else {
            state.services[index] = ServiceRecord {
                name: service.name.clone(),
                revision,
                container: None,
            };
            state.persist(ctx, &session)?;
            let created = engine.mutate(
                Method::POST,
                &["containers", "create"],
                &[("name", &name), ("platform", "linux/amd64")],
                Some(configuration),
                &[201],
            )?;
            let id = created["Id"].as_str().ok_or_else(|| {
                failure("Docker create returned no container identity; inspect status before retry")
            })?;
            let observed = engine
                .get(&["containers", id, "json"])?
                .ok_or_else(|| failure("created container cannot be inspected"))?;
            plan::admit_container(&observed, &state, &state.services[index])?
        };
        state.services[index].container = Some(container.clone());
        state.persist(ctx, &session)?;
        let observed = engine
            .get(&["containers", &container, "json"])?
            .ok_or_else(|| failure("owned container disappeared before start"))?;
        if observed["State"]["Running"].as_bool() != Some(true) {
            engine.mutate(
                Method::POST,
                &["containers", &container, "start"],
                &[],
                None,
                &[204, 304],
            )?;
        }
        wait_healthy(&engine, &state, &state.services[index], deadline)?;
    }
    statuses(&engine, &state, &ctx.only)
}

fn ensure_network(engine: &Engine, state: &ServiceState) -> Result<(), ServiceError> {
    let name = state.workspace.resource("network", "default");
    if engine.get(&["networks", &name])?.is_none() {
        engine.mutate(Method::POST, &["networks", "create"], &[], Some(&json!({"Name":name,"Driver":"bridge","CheckDuplicate":true,"Labels":plan::labels(state,"network")})), &[201])?;
    }
    let observed = engine
        .get(&["networks", &name])?
        .ok_or_else(|| failure("workspace network is absent after creation"))?;
    plan::admit_labels(&observed["Labels"], state, "network")?;
    if observed["Driver"].as_str() != Some("bridge") || observed["Name"].as_str() != Some(&name) {
        return Err(failure("workspace network policy changed externally"));
    }
    Ok(())
}
fn ensure_volume(engine: &Engine, state: &ServiceState, volume: &str) -> Result<(), ServiceError> {
    let name = state.workspace.resource("volume", volume);
    if engine.get(&["volumes", &name])?.is_none() {
        engine.mutate(
            Method::POST,
            &["volumes", "create"],
            &[],
            Some(&json!({"Name":name,"Driver":"local","Labels":plan::labels(state,volume)})),
            &[201],
        )?;
    }
    let observed = engine
        .get(&["volumes", &name])?
        .ok_or_else(|| failure("workspace volume is absent after creation"))?;
    plan::admit_labels(&observed["Labels"], state, volume)?;
    if observed["Driver"].as_str() != Some("local")
        || observed["Name"].as_str() != Some(&name)
        || observed["Options"]
            .as_object()
            .is_some_and(|options| !options.is_empty())
    {
        return Err(failure(
            "workspace volume has foreign driver or mount options",
        ));
    }
    Ok(())
}
fn wait_healthy(
    engine: &Engine,
    state: &ServiceState,
    record: &ServiceRecord,
    deadline: Instant,
) -> Result<(), ServiceError> {
    let name = state.workspace.resource("service", &record.name);
    loop {
        let value = engine
            .get(&["containers", &name, "json"])?
            .ok_or_else(|| failure("owned service disappeared while waiting for readiness"))?;
        plan::admit_container(&value, state, record)?;
        if plan::is_healthy(&value) {
            return Ok(());
        }
        if value["State"]["Running"].as_bool() != Some(true)
            || value["State"]["Health"]["Status"].as_str() == Some("unhealthy")
        {
            return Err(failure(format!(
                "service {:?} failed readiness; container and volumes retained; inspect grip services status/logs",
                record.name
            )));
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                failure("service readiness deadline expired; container and volumes retained")
            })?;
        std::thread::sleep(remaining.min(Duration::from_millis(200)));
    }
}
fn selected<'a>(
    state: &'a ServiceState,
    names: &[String],
) -> Result<Vec<&'a ServiceRecord>, ServiceError> {
    for name in names {
        if !state.services.iter().any(|record| &record.name == name) {
            return Err(failure(format!("no recorded service {name:?}")));
        }
    }
    Ok(state
        .services
        .iter()
        .filter(|record| names.is_empty() || names.contains(&record.name))
        .collect())
}
fn statuses(
    engine: &Engine,
    state: &ServiceState,
    names: &[String],
) -> Result<Vec<ServiceStatus>, ServiceError> {
    selected(state, names)?
        .into_iter()
        .map(|record| {
            let name = state.workspace.resource("service", &record.name);
            let (container, status) = match engine.get(&["containers", &name, "json"])? {
                None => (None, "absent".to_owned()),
                Some(value) => {
                    let id = plan::admit_container(&value, state, record)?;
                    let status = if plan::is_healthy(&value) {
                        "healthy"
                    } else if value["State"]["Running"].as_bool() == Some(true) {
                        value["State"]["Health"]["Status"]
                            .as_str()
                            .unwrap_or("unknown")
                    } else {
                        value["State"]["Status"].as_str().unwrap_or("unknown")
                    };
                    (Some(id), status.to_owned())
                }
            };
            Ok(ServiceStatus {
                name: record.name.clone(),
                revision: record.revision.to_string(),
                container,
                state: status,
            })
        })
        .collect()
}

/// Read recorded state only; never evaluates a repository or starts a service.
pub fn status(ctx: &Ctx, deadline: Instant) -> Result<Vec<ServiceStatus>, ServiceError> {
    let identity = WorkspaceServiceId::for_context(ctx)?;
    let Some(state) = ServiceState::read(ctx, &identity)? else {
        return Ok(Vec::new());
    };
    let engine = Engine::connect(Some(&state.socket), deadline)?;
    state.admit_engine(&engine)?;
    statuses(&engine, &state, &ctx.only)
}

/// Explicit stop and removal of owned containers only. Named volumes and the
/// workspace network remain; neither down, GC nor rollback deletes their data.
pub fn down(ctx: &Ctx, deadline: Instant) -> Result<Vec<ServiceStatus>, ServiceError> {
    let session = LifecycleSession::acquire(&ctx.home)?;
    let identity = WorkspaceServiceId::for_context(ctx)?;
    let Some(mut state) = ServiceState::read(ctx, &identity)? else {
        return Ok(Vec::new());
    };
    let engine = Engine::connect(Some(&state.socket), deadline)?;
    state.admit_engine(&engine)?;
    let observations = statuses(&engine, &state, &ctx.only)?;
    for observation in observations.iter().rev() {
        if let Some(container) = &observation.container {
            engine.mutate(
                Method::POST,
                &["containers", container, "stop"],
                &[("t", "10")],
                None,
                &[204, 304, 404],
            )?;
            engine.mutate(
                Method::DELETE,
                &["containers", container],
                &[("v", "false"), ("force", "false")],
                None,
                &[204, 404],
            )?;
        }
        state
            .services
            .iter_mut()
            .find(|record| record.name == observation.name)
            .expect("observed record")
            .container = None;
        state.persist(ctx, &session)?;
    }
    statuses(&engine, &state, &ctx.only)
}

pub fn logs(
    ctx: &Ctx,
    service: &str,
    lines: u16,
    deadline: Instant,
) -> Result<Vec<u8>, ServiceError> {
    let identity = WorkspaceServiceId::for_context(ctx)?;
    let state = ServiceState::read(ctx, &identity)?
        .ok_or_else(|| failure("workspace has no recorded services"))?;
    let record = state
        .services
        .iter()
        .find(|record| record.name == service)
        .ok_or_else(|| failure("service is not recorded"))?;
    let engine = Engine::connect(Some(&state.socket), deadline)?;
    state.admit_engine(&engine)?;
    let name = state.workspace.resource("service", &record.name);
    let observed = engine
        .get(&["containers", &name, "json"])?
        .ok_or_else(|| failure("service container is absent"))?;
    let id = plan::admit_container(&observed, &state, record)?;
    engine.logs(&id, lines)
}

/// Called by the shared task runner under its lifecycle admission. Readiness is
/// an observation, not a promise of future availability or startup authority.
pub(crate) fn require_ready(
    ir: &Ir,
    ctx: &Ctx,
    services: &[String],
    deadline: Instant,
) -> Result<(), ExecError> {
    if services.is_empty() {
        return Ok(());
    }
    let check = || -> Result<(), ServiceError> {
        let selected = plan::selection(ir, services)?;
        let identity = WorkspaceServiceId::for_context(ctx)?;
        let state = ServiceState::read(ctx, &identity)?.ok_or_else(|| {
            failure("required services are not installed; run grip up explicitly")
        })?;
        let engine = Engine::connect(Some(&state.socket), deadline)?;
        state.admit_engine(&engine)?;
        for service in selected {
            let record = state
                .services
                .iter()
                .find(|record| record.name == service.name)
                .ok_or_else(|| {
                    failure(format!(
                        "required service {:?} has not been started",
                        service.name
                    ))
                })?;
            if record.revision != plan::revision(service)? {
                return Err(failure(
                    "required service declaration differs from its running revision; explicitly down/up it",
                ));
            }
            let name = state.workspace.resource("service", &record.name);
            let value = engine
                .get(&["containers", &name, "json"])?
                .ok_or_else(|| failure("required service container is absent"))?;
            plan::admit_container(&value, &state, record)?;
            if !plan::is_healthy(&value) {
                return Err(failure(format!(
                    "required service {:?} is not healthy",
                    service.name
                )));
            }
        }
        Ok(())
    };
    check().map_err(exec_error)
}
