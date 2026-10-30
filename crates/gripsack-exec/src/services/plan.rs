//! Pure service selection, identity and exact Docker configuration.
use super::{
    ServiceError, failure,
    state::{MAX_SERVICES, ServiceState},
};
use gripsack_ir::{
    Ir,
    workspace_model::{ServiceEnvValue, ServiceHealth, ServiceOutput, WorkspaceOutput},
};
use gripsack_process::Sha256Digest;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const WORKSPACE_LABEL: &str = "dev.gripsack.workspace";
pub(super) const OWNER_LABEL: &str = "dev.gripsack.owner";
pub(super) const MEMBER_LABEL: &str = "dev.gripsack.member";
pub(super) const REVISION_LABEL: &str = "dev.gripsack.revision";

pub(super) fn selection<'a>(
    ir: &'a Ir,
    selected: &[String],
) -> Result<Vec<&'a ServiceOutput>, ServiceError> {
    let catalog: BTreeMap<_, _> = ir
        .workspace
        .outputs
        .iter()
        .filter_map(|output| match output {
            WorkspaceOutput::Service(service) => Some((service.name.as_str(), service)),
            _ => None,
        })
        .collect();
    if catalog.len() > MAX_SERVICES {
        return Err(failure("workspace exceeds the service count bound"));
    }
    let mut pending = BTreeSet::new();
    let mut complete = BTreeSet::new();
    let mut result = Vec::new();
    fn visit<'a>(
        name: &'a str,
        catalog: &BTreeMap<&'a str, &'a ServiceOutput>,
        pending: &mut BTreeSet<&'a str>,
        complete: &mut BTreeSet<&'a str>,
        result: &mut Vec<&'a ServiceOutput>,
    ) -> Result<(), ServiceError> {
        if complete.contains(name) {
            return Ok(());
        }
        let service = catalog
            .get(name)
            .ok_or_else(|| failure(format!("no service output named {name:?}")))?;
        if !pending.insert(name) {
            return Err(failure(format!("service dependency cycle at {name:?}")));
        }
        for dependency in &service.depends_on {
            visit(dependency, catalog, pending, complete, result)?;
        }
        pending.remove(name);
        complete.insert(name);
        result.push(service);
        Ok(())
    }
    if selected.is_empty() {
        for name in catalog.keys() {
            visit(name, &catalog, &mut pending, &mut complete, &mut result)?;
        }
    } else {
        for name in selected {
            let (name, _) = catalog
                .get_key_value(name.as_str())
                .ok_or_else(|| failure(format!("no service output named {name:?}")))?;
            visit(name, &catalog, &mut pending, &mut complete, &mut result)?;
        }
    }
    Ok(result)
}

pub(super) fn revision(service: &ServiceOutput) -> Result<Sha256Digest, ServiceError> {
    let mut value = serde_json::to_value(service)?;
    value
        .as_object_mut()
        .expect("service is a record")
        .remove("span");
    for key in ["depends_on", "ports", "volumes"] {
        if let Some(items) = value.get_mut(key).and_then(Value::as_array_mut) {
            items.sort_by_cached_key(Value::to_string);
        }
    }
    Ok(Sha256Digest::of(&serde_json::to_vec(&(
        "gripsack.service.v1",
        value,
    ))?))
}

pub(super) fn labels(state: &ServiceState, member: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (WORKSPACE_LABEL.to_owned(), state.workspace.as_string()),
        (OWNER_LABEL.to_owned(), state.owner.to_string()),
        (MEMBER_LABEL.to_owned(), member.to_owned()),
    ])
}

pub(super) fn admit_labels(
    value: &Value,
    state: &ServiceState,
    member: &str,
) -> Result<(), ServiceError> {
    if labels(state, member)
        .iter()
        .any(|(key, expected)| value.get(key).and_then(Value::as_str) != Some(expected.as_str()))
    {
        return Err(failure(
            "foreign or drifted Docker resource; no automatic adoption, replacement or deletion",
        ));
    }
    Ok(())
}

pub(super) fn configuration(
    service: &ServiceOutput,
    state: &ServiceState,
    inherited: &BTreeMap<String, String>,
) -> Result<Value, ServiceError> {
    let revision = revision(service)?;
    let mut labels = labels(state, &service.name);
    labels.insert(REVISION_LABEL.to_owned(), revision.to_string());
    let mut env = Vec::with_capacity(service.env.len());
    for (key, value) in &service.env {
        let value = match value {
            ServiceEnvValue::Literal { value } => value,
            ServiceEnvValue::Inherited { name } => inherited.get(name).ok_or_else(|| {
                failure(format!(
                    "service {:?} requires inherited variable {name:?}",
                    service.name
                ))
            })?,
        };
        if value.contains('\0') {
            return Err(failure("service environment contains NUL"));
        }
        env.push(format!("{key}={value}"));
    }
    let mut exposed = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    for port in &service.ports {
        let name = format!("{}/tcp", port.container);
        exposed.insert(name.clone(), json!({}));
        bindings
            .entry(name)
            .or_insert_with(Vec::new)
            .push(json!({"HostIp":port.address,"HostPort":port.host.to_string()}));
    }
    let mounts: Vec<_> = service.volumes.iter().map(|volume| json!({
        "Type":"volume", "Source":state.workspace.resource("volume", &volume.name), "Target":volume.target,
        "ReadOnly":false, "VolumeOptions":{"NoCopy":false}
    })).collect();
    let ServiceHealth::Command {
        argv,
        interval_seconds,
        timeout_seconds,
        retries,
    } = &service.health;
    let mut health_command = Vec::with_capacity(argv.len() + 1);
    health_command.push("CMD");
    health_command.extend(argv.iter().map(String::as_str));
    let network = state.workspace.resource("network", "default");
    let mut configuration = json!({
        "Image":service.image, "Env":env, "Labels":labels,
        "AttachStdin":false, "OpenStdin":false, "StdinOnce":false, "Tty":false,
        "ExposedPorts":exposed,
        "Healthcheck":{"Test":health_command,"Interval":u64::from(*interval_seconds)*1_000_000_000,"Timeout":u64::from(*timeout_seconds)*1_000_000_000,"Retries":retries},
        "HostConfig":{"NetworkMode":network,"PortBindings":bindings,"Mounts":mounts,
            "RestartPolicy":{"Name":"no"},"AutoRemove":false,"Privileged":false,
            "LogConfig":{"Type":"local","Config":{"max-size":"10m","max-file":"3"}}},
        "NetworkingConfig":{"EndpointsConfig":{network:{"Aliases":[service.name]}}}
    });
    if !service.command.is_empty() {
        configuration["Cmd"] = json!(service.command);
    }
    Ok(configuration)
}

pub(super) fn admit_container(
    value: &Value,
    state: &ServiceState,
    record: &super::state::ServiceRecord,
) -> Result<String, ServiceError> {
    let id = value
        .get("Id")
        .and_then(Value::as_str)
        .ok_or_else(|| failure("Docker container has no identity"))?;
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(failure("Docker container identity is invalid"));
    }
    if record
        .container
        .as_ref()
        .is_some_and(|expected| expected != id)
    {
        return Err(failure(
            "recorded container was replaced; refusing the foreign replacement",
        ));
    }
    admit_labels(&value["Config"]["Labels"], state, &record.name)?;
    if value["Config"]["Labels"][REVISION_LABEL].as_str()
        != Some(record.revision.to_string().as_str())
    {
        return Err(failure(
            "Docker container revision does not match the admitted ownership intent",
        ));
    }
    let name = format!("/{}", state.workspace.resource("service", &record.name));
    if value["Name"].as_str() != Some(&name) {
        return Err(failure("owned container name changed externally"));
    }
    if value["HostConfig"]["Privileged"].as_bool() != Some(false)
        || value["HostConfig"]["NetworkMode"].as_str()
            != Some(state.workspace.resource("network", "default").as_str())
        || value["HostConfig"]["RestartPolicy"]["Name"].as_str() != Some("no")
    {
        return Err(failure("owned container runtime policy drifted"));
    }
    Ok(id.to_owned())
}

pub(super) fn is_healthy(container: &Value) -> bool {
    container["State"]["Running"].as_bool() == Some(true)
        && container["State"]["Health"]["Status"].as_str() == Some("healthy")
}
