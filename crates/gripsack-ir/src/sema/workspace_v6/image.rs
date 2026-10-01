use super::{fail, values};
use crate::{Diagnostic, codes, workspace::PlatformOs, workspace_v6::*};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn check(image: &ImageOutput, out: &mut Vec<Diagnostic>) {
    let invalid = |out: &mut Vec<Diagnostic>, message| {
        fail(out, codes::INVALID_WORKSPACE_VALUE, &image.span, message);
    };
    if image.target.os != PlatformOs::Linux {
        invalid(out, "OCI production requires an explicit Linux target");
    }
    if image.base.as_ref().is_some_and(|reference| !values::image_reference(reference)) {
        invalid(out, "image base must be a normalized digest-pinned registry reference");
    }
    for destination in image.destinations.values() {
        if !destination.path.is_safe() {
            invalid(out, "image package destinations must be normalized absolute paths below /");
        }
    }
    if !absolute_path(&image.config.cwd) {
        invalid(out, "image working directory must be normalized and absolute");
    }
    for (index, argument) in image.config.entrypoint.iter().enumerate() {
        values::argument(argument, &image.span, out);
        match argument {
            WorkspaceArg::Literal { value } if index != 0 || absolute_path(value) => {}
            WorkspaceArg::PackageCommand { package, .. } if image.packages.contains(package) => {}
            WorkspaceArg::PackageCommand { .. } => invalid(out, "image command package must be selected explicitly"),
            _ => invalid(out, "image entrypoint accepts absolute executable literals and selected package commands, not host or production paths"),
        }
    }
    if image.config.args.iter().any(|value| value.contains('\0')) {
        invalid(out, "image command arguments cannot contain NUL");
    }
    for (key, value) in &image.config.env {
        if !environment_key(key) || value.contains('\0') {
            invalid(out, "image environment requires valid variable names and NUL-free values");
        }
    }
}

/// Visit only declared package runtime edges, not build tools or command-side
/// references, so a placement cannot silently smuggle in an unselected output.
pub(super) fn selection<'a>(
    image: &'a ImageOutput,
    catalog: &BTreeMap<&'a str, &'a WorkspaceOutput>,
    out: &mut Vec<Diagnostic>,
) {
    let mut selected = BTreeSet::new();
    let mut pending: Vec<&str> = image.packages.iter().map(String::as_str).collect();
    let mut placements: BTreeMap<String, &PackageOutput> = BTreeMap::new();
    while let Some(name) = pending.pop() {
        if !selected.insert(name) { continue; }
        let Some(WorkspaceOutput::Package(package)) = catalog.get(name).copied() else { continue; };
        pending.extend(package.runtime.iter().map(String::as_str));
        let destination = image.destinations.get(name);
        match &package.layout {
            PackageLayoutV6::FixedPrefix { prefix } => {
                if destination.is_none_or(|destination| &destination.path != prefix) {
                    out.push(Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, "fixed-prefix image package requires its exact declared destination")
                        .with_label(Some(image.span.clone()), "image selection")
                        .with_label(Some(package.span.clone()), "fixed-prefix package"));
                }
            }
            PackageLayoutV6::PrefixMaterialized => {
                if destination.is_none() {
                    out.push(Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, "prefix-materialized image package requires an explicit destination to materialize at")
                        .with_label(Some(image.span.clone()), "image selection")
                        .with_label(Some(package.span.clone()), "prefix-materialized package"));
                }
            }
            PackageLayoutV6::Relocatable => {}
        }
        let prefix = destination.map_or_else(|| format!("/opt/gripsack/{name}"), |destination| destination.path.as_str().to_owned());
        if !absolute_path(&prefix) || prefix == "/" {
            fail(out, codes::INVALID_WORKSPACE_VALUE, &image.span, "package name requires an explicit normalized image destination");
            continue;
        }
        for (other_prefix, other) in &placements {
            if overlaps(&prefix, other_prefix) {
                out.push(Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, format!("image package destinations overlap: {prefix:?} and {other_prefix:?}"))
                    .with_label(Some(package.span.clone()), "package placement")
                    .with_label(Some(other.span.clone()), "conflicting package placement")
                    .with_label(Some(image.span.clone()), "destinations selected here"));
            }
        }
        placements.insert(prefix, package);
    }
    for name in image.destinations.keys() {
        if !selected.contains(name.as_str()) {
            fail(out, codes::UNKNOWN_WORKSPACE_REF, &image.span, format!("image destination names unselected runtime package {name:?}"));
        }
    }
}

fn absolute_path(value: &str) -> bool {
    value == "/" || value.strip_prefix('/').is_some_and(|relative| values::selector(relative) && relative != ".")
}
fn environment_key(value: &str) -> bool {
    value.bytes().next().is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
fn overlaps(left: &str, right: &str) -> bool {
    left == right
        || left.strip_prefix(right).is_some_and(|rest| rest.starts_with('/'))
        || right.strip_prefix(left).is_some_and(|rest| rest.starts_with('/'))
}
