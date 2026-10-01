use super::{BuildPlan, MAX_PLAN_NODES, Node, PlanError};
use crate::identity::safe_atom;
use gripsack_process::Sha256Digest;
use std::collections::BTreeMap;

pub(super) fn validate(plan: &BuildPlan) -> Result<(), PlanError> {
    plan.exporter.validate().map_err(PlanError::Exporter)?;
    if plan.nodes.is_empty()
        || plan.nodes.len() > MAX_PLAN_NODES
        || plan.root.index() >= plan.nodes.len()
    {
        return Err(PlanError::NodeLimit);
    }
    let mut sources = BTreeMap::new();
    for (index, node) in plan.nodes.iter().enumerate() {
        let mut valid_edges = true;
        node.visit_inputs(|producer| {
            valid_edges &= gripsack_policy::buildkit::backward_edge(producer.index(), index);
        });
        if !valid_edges {
            return Err(PlanError::Edge { node: index });
        }
        let invalid = |reason| PlanError::Operation {
            node: index,
            reason,
        };
        match node {
            Node::Image { reference } => {
                let Some((name, hash)) = reference.rsplit_once("@sha256:") else {
                    return Err(invalid(
                        "toolchain image must use an explicit SHA-256 digest",
                    ));
                };
                if !name.contains('/')
                    || name.starts_with('/')
                    || name.ends_with('/')
                    || !name.bytes().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || b"./_:-".contains(&c)
                    })
                    || Sha256Digest::parse(hash).is_err()
                {
                    return Err(invalid(
                        "image reference must be a normalized, digest-pinned registry path",
                    ));
                }
            }
            Node::Local { name, digest } => {
                if !safe_atom(name) {
                    return Err(invalid("source names are bounded opaque ASCII atoms"));
                }
                if sources
                    .insert(name, digest)
                    .is_some_and(|previous| previous != digest)
                {
                    return Err(invalid(
                        "one source name cannot identify different captured bytes",
                    ));
                }
            }
            Node::File {
                path, mode, data, ..
            } => {
                if !absolute_path(path)
                    || path == "/"
                    || *mode > 0o777
                    || data.len() > super::MAX_DEFINITION_BYTES
                {
                    return Err(invalid(
                        "file needs a normalized non-root path, rwx mode and bounded content",
                    ));
                }
            }
            Node::Directory { path, mode, .. } => {
                if !absolute_path(path) || *mode > 0o777 {
                    return Err(invalid(
                        "directory needs a normalized absolute path and rwx mode",
                    ));
                }
            }
            Node::Copy { source_path, destination, .. }
            | Node::Install { source_path, destination, .. } => {
                if !absolute_path(source_path) || !absolute_path(destination) {
                    return Err(invalid("copy selectors must be normalized absolute paths"));
                }
            }
            Node::Process {
                argv,
                env,
                cwd,
                mounts,
                output,
                ..
            } => {
                if argv.first().is_none_or(|program| {
                    !absolute_path(program)
                        && (program.is_empty()
                            || matches!(program.as_str(), "." | "..")
                            || program.contains('/'))
                }) || argv.iter().any(|arg| arg.contains('\0'))
                {
                    return Err(invalid(
                        "process argv must name an absolute executable or a bare name in its explicit image-local PATH, and contain no NUL",
                    ));
                }
                if !absolute_path(cwd) {
                    return Err(invalid("process cwd must be explicit and absolute"));
                }
                let mut previous_key = None;
                let mut explicit_path = false;
                for entry in env {
                    let Some((key, value)) = entry.split_once('=') else {
                        return Err(invalid("invalid environment binding"));
                    };
                    if !environment_key(key)
                        || value.contains('\0')
                        || previous_key.is_some_and(|previous| previous >= key)
                    {
                        return Err(invalid("environment keys must be valid, unique and sorted"));
                    }
                    previous_key = Some(key);
                    explicit_path |= key == "PATH";
                }
                if !explicit_path {
                    return Err(invalid(
                        "an explicit PATH binding is required; use PATH= for no search path",
                    ));
                }
                if mounts.len() >= MAX_PLAN_NODES {
                    return Err(invalid("process mount count exceeds its graph bound"));
                }
                let mut outputs = 0;
                for (position, mount) in mounts.iter().enumerate() {
                    if !absolute_path(&mount.destination) || mount.destination == "/" {
                        return Err(invalid(
                            "non-root mounts require explicit normalized absolute destinations",
                        ));
                    }
                    if !mount.readonly {
                        if &mount.destination != output {
                            return Err(invalid("only the declared output mount may be writable"));
                        }
                        outputs += 1;
                    }
                    if mounts[..position]
                        .iter()
                        .any(|other| overlaps(&other.destination, &mount.destination))
                    {
                        return Err(invalid("mount destinations must be disjoint"));
                    }
                }
                if outputs != 1 {
                    return Err(invalid("process needs exactly one writable output mount"));
                }
            }
        }
    }
    // Topological references point backwards; one reverse traversal is enough.
    let mut reachable = vec![false; plan.nodes.len()];
    reachable[plan.root.index()] = true;
    for index in (0..plan.nodes.len()).rev() {
        if reachable[index] {
            plan.nodes[index].visit_inputs(|producer| reachable[producer.index()] = true);
        }
    }
    if let Some(node) = reachable.iter().position(|needed| !needed) {
        return Err(PlanError::Unreachable { node });
    }
    Ok(())
}

pub(crate) fn absolute_path(path: &str) -> bool {
    path == "/"
        || (path.starts_with('/')
            && !path.contains('\0')
            && path[1..]
                .split('/')
                .all(|part| !matches!(part, "" | "." | "..")))
}
pub(super) fn environment_key(key: &str) -> bool {
    key.bytes()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
fn overlaps(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
