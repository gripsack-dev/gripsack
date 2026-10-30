//! Service configuration and readiness prerequisites, before engine effects.
use super::fail;
use crate::{Diagnostic, Span, codes, workspace_model::*};
use std::collections::BTreeSet;

fn invalid(out: &mut Vec<Diagnostic>, span: &Span, message: &str) {
    fail(out, codes::INVALID_WORKSPACE_VALUE, span, message);
}

fn environment_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn text(value: &str) -> bool {
    value.len() <= MAX_SERVICE_VALUE_BYTES && !value.contains('\0')
}

fn arguments(values: &[String], required: bool) -> bool {
    values.len() <= MAX_SERVICE_ARGUMENTS
        && (!required || !values.is_empty())
        && values.first().is_none_or(|value| !value.is_empty())
        && values.iter().all(|value| text(value))
}

pub(super) fn references(values: &[String], span: &Span, out: &mut Vec<Diagnostic>) {
    let mut seen = BTreeSet::new();
    if values.len() > MAX_SERVICE_ENTRIES
        || values.iter().any(|value| {
            value.is_empty()
                || value.len() > MAX_SERVICE_VALUE_BYTES
                || value.chars().any(char::is_control)
                || !seen.insert(value)
        })
    {
        invalid(
            out,
            span,
            "service prerequisites must be bounded, nonempty and unique output names",
        );
    }
}

pub(super) fn check(service: &ServiceOutput, out: &mut Vec<Diagnostic>) {
    let span = &service.span;
    if !valid_service_image(&service.image) {
        invalid(
            out,
            span,
            "services require an immutable OCI name@sha256:<64 lowercase hex> image",
        );
    }
    references(&service.depends_on, span, out);
    if service.ports.len() > MAX_SERVICE_ENTRIES
        || service.volumes.len() > MAX_SERVICE_ENTRIES
        || service.env.len() > MAX_SERVICE_ENTRIES
    {
        invalid(
            out,
            span,
            "service ports, volumes and environment are limited to 128 entries each",
        );
    }
    let mut ports = BTreeSet::new();
    for port in &service.ports {
        let address = port.address.parse::<std::net::IpAddr>();
        if port.host == 0
            || port.container == 0
            || !address
                .is_ok_and(|address| address.is_loopback() && address.to_string() == port.address)
            || !ports.insert((&port.address, port.host))
        {
            invalid(
                out,
                span,
                "service ports require nonzero ports and unique canonical loopback bindings",
            );
        }
    }
    let mut volumes = BTreeSet::new();
    let mut targets = BTreeSet::new();
    for volume in &service.volumes {
        let valid_name = !volume.name.is_empty()
            && volume.name.len() <= 255
            && volume
                .name
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && volume
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte));
        let valid_target = volume.target.starts_with('/')
            && volume.target != "/"
            && text(&volume.target)
            && volume.target[1..]
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..");
        if !valid_name
            || !valid_target
            || !volumes.insert(&volume.name)
            || !targets.insert(&volume.target)
        {
            invalid(
                out,
                span,
                "service volumes require unique named volumes and normalized absolute targets below /",
            );
        }
    }
    for (name, value) in &service.env {
        if !environment_name(name) || name.len() > MAX_SERVICE_VALUE_BYTES {
            invalid(
                out,
                span,
                "service environment keys must be bounded environment variable names",
            );
        }
        match value {
            ServiceEnvValue::Literal { value } if !text(value) => {
                invalid(
                    out,
                    span,
                    "service environment values must be bounded and contain no NUL",
                );
            }
            ServiceEnvValue::Inherited { name }
                if !environment_name(name) || name.len() > MAX_SERVICE_VALUE_BYTES =>
            {
                invalid(
                    out,
                    span,
                    "inherited service environment names must be explicit environment variable names",
                );
            }
            _ => {}
        }
    }
    if !arguments(&service.command, false) {
        invalid(
            out,
            span,
            "service command arguments must be bounded, contain no NUL and name an executable",
        );
    }
    let ServiceHealth::Command {
        argv,
        interval_seconds,
        timeout_seconds,
        retries,
    } = &service.health;
    if !arguments(argv, true)
        || !(1..=MAX_SERVICE_HEALTH_SECONDS).contains(interval_seconds)
        || !(1..=MAX_SERVICE_HEALTH_SECONDS).contains(timeout_seconds)
        || !(1..=MAX_SERVICE_HEALTH_RETRIES).contains(retries)
    {
        invalid(
            out,
            span,
            "service health requires a command, 1..=3600 second intervals/timeouts and 1..=100 retries",
        );
    }
}
