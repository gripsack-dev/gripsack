use super::{
    BuildPlan, CheckError, pb,
    structure::{self, Inputs},
};
use crate::{identity::SnapshotDigest, plan::Node};
use gripsack_policy::buildkit::{isolated_execution, output_mount};

pub(super) fn operation(
    plan: &BuildPlan,
    expected: &Node,
    actual: &pb::Op,
    inputs: &mut Inputs<'_>,
) -> Result<(), CheckError> {
    match (expected, actual.op.as_ref()) {
        (Node::Image { reference }, Some(pb::op::Op::Source(source))) => {
            structure::known(source)?;
            structure::constraints(actual, plan.platform, true)?;
            if source.identifier.strip_prefix("docker-image://") != Some(reference.as_str())
                || !source.attrs.is_empty()
            {
                return Err(CheckError::Mismatch(
                    "toolchain source/pin/options substitution",
                ));
            }
        }
        (Node::Local { name, digest }, Some(pb::op::Op::Source(source))) => {
            structure::known(source)?;
            structure::constraints(actual, plan.platform, false)?;
            if source.identifier.strip_prefix("local://") != Some(name.as_str())
                || source.attrs.len() != 2
            {
                return Err(CheckError::Mismatch(
                    "captured source identity/options substitution",
                ));
            }
            let (mut shared, mut unique) = (false, false);
            for entry in &source.attrs {
                structure::known(entry)?;
                let flag = match entry.key.as_str() {
                    "local.sharedkeyhint" => &mut shared,
                    "local.unique" => &mut unique,
                    _ => return Err(CheckError::Mismatch("undeclared source/session attribute")),
                };
                if *flag
                    || !SnapshotDigest::parse(&entry.value).is_ok_and(|actual| &actual == digest)
                {
                    return Err(CheckError::Mismatch("captured source digest substitution"));
                }
                *flag = true;
            }
        }
        (
            Node::File { .. } | Node::Directory { .. } | Node::Copy { .. } | Node::Install { .. },
            Some(pb::op::Op::File(file)),
        ) => {
            structure::constraints(actual, plan.platform, false)?;
            file_operation(expected, file, inputs)?;
        }
        (
            Node::Process {
                root,
                argv,
                env,
                cwd,
                mounts,
                output,
            },
            Some(pb::op::Op::Exec(exec)),
        ) => {
            structure::constraints(actual, plan.platform, true)?;
            structure::known(exec)?;
            if !isolated_execution(exec.network, exec.security) {
                return Err(CheckError::Mismatch("execution isolation weakened"));
            }
            let meta = exec
                .meta
                .as_ref()
                .ok_or(CheckError::Mismatch("missing process metadata"))?;
            structure::known(meta)?;
            if &meta.args != argv
                || &meta.env != env
                || &meta.cwd != cwd
                || !meta.user.is_empty()
                || !meta.hostname.is_empty()
                || !meta.remove_mount_stubs_recursive
                || !meta.valid_exit_codes.is_empty()
            {
                return Err(CheckError::Mismatch(
                    "argv/environment/cwd/exit policy substitution",
                ));
            }
            if exec.mounts.len() != mounts.len() + 1 {
                return Err(CheckError::Mismatch("process mount coverage"));
            }
            let mut root_seen = false;
            let mut observed = [false; crate::plan::MAX_PLAN_NODES];
            if mounts.len() >= observed.len() {
                return Err(CheckError::Mismatch("process mount bound"));
            }
            for mount in &exec.mounts {
                structure::known(mount)?;
                if !mount.selector.is_empty()
                    || !mount.result_id.is_empty()
                    || mount.content_cache != 0
                {
                    return Err(CheckError::Mismatch(
                        "unchecked mount selector/cache/result policy",
                    ));
                }
                if mount.dest == "/" {
                    if root_seen
                        || !output_mount(mount.mount_type, mount.readonly, mount.output, false)
                    {
                        return Err(CheckError::Mismatch(
                            "process root is not one read-only bind",
                        ));
                    }
                    root_seen = true;
                    inputs.bind(mount.input, Some(*root))?;
                } else {
                    let (index, expected_mount) = mounts
                        .iter()
                        .enumerate()
                        .find(|(_, candidate)| candidate.destination == mount.dest)
                        .ok_or(CheckError::Mismatch("undeclared mount destination"))?;
                    let selected = &mount.dest == output;
                    if observed[index]
                        || !output_mount(mount.mount_type, mount.readonly, mount.output, selected)
                        || mount.readonly != expected_mount.readonly
                    {
                        return Err(CheckError::Mismatch("mount/output policy substitution"));
                    }
                    observed[index] = true;
                    inputs.bind(mount.input, expected_mount.source)?;
                }
            }
            if !root_seen || !observed[..mounts.len()].iter().all(|seen| *seen) {
                return Err(CheckError::Mismatch("missing admitted root or mount"));
            }
        }
        _ => return Err(CheckError::Mismatch("operation kind substitution")),
    }
    Ok(())
}

fn file_operation(
    expected: &Node,
    actual: &pb::FileOp,
    inputs: &mut Inputs<'_>,
) -> Result<(), CheckError> {
    structure::known(actual)?;
    let [action] = actual.actions.as_slice() else {
        return Err(CheckError::Mismatch("file-action cardinality"));
    };
    structure::known(action)?;
    if action.output != 0 {
        return Err(CheckError::Mismatch("file output index substitution"));
    }
    match (expected, action.action.as_ref()) {
        (
            Node::File {
                input,
                path,
                data,
                mode,
            },
            Some(pb::file_action::Action::Mkfile(file)),
        ) => {
            structure::known(file)?;
            inputs.bind(action.input, *input)?;
            inputs.bind(action.secondary_input, None)?;
            if &file.path != path
                || &file.data != data
                || file.mode != *mode as i32
                || file.timestamp != 0
            {
                return Err(CheckError::Mismatch(
                    "literal file contents/path/mode/time substitution",
                ));
            }
        }
        (
            Node::Directory { input, path, mode },
            Some(pb::file_action::Action::Mkdir(directory)),
        ) => {
            structure::known(directory)?;
            inputs.bind(action.input, *input)?;
            inputs.bind(action.secondary_input, None)?;
            if &directory.path != path
                || directory.mode != *mode as i32
                || !directory.make_parents
                || directory.timestamp != 0
            {
                return Err(CheckError::Mismatch(
                    "directory path/mode/parent/time substitution",
                ));
            }
        }
        (
            Node::Copy { input, source, source_path, destination, contents }
            | Node::Install { input, source, source_path, destination, contents, .. },
            Some(pb::file_action::Action::Copy(copy)),
        ) => {
            structure::known(copy)?;
            inputs.bind(action.input, *input)?;
            inputs.bind(action.secondary_input, Some(*source))?;
            let ownership = match expected {
                Node::Install { uid, gid, .. } => Some((*uid, *gid)),
                _ => None,
            };
            check_ownership(copy.owner.as_ref(), ownership)?;
            if &copy.src != source_path
                || &copy.dest != destination
                || copy.dir_copy_contents != *contents
                || copy.mode != -1
                || copy.follow_symlink
                || copy.attempt_unpack_docker_compatibility
                || !copy.create_dest_path
                || copy.allow_wildcard
                || copy.allow_empty_wildcard
                || copy.timestamp != 0
                || !copy.include_patterns.is_empty()
                || !copy.exclude_patterns.is_empty()
                || copy.always_replace_existing_dest_paths
                || !copy.mode_str.is_empty()
                || !copy.required_paths.is_empty()
            {
                return Err(CheckError::Mismatch(
                    "copy selector/content/collision policy substitution",
                ));
            }
        }
        _ => return Err(CheckError::Mismatch("file-action kind substitution")),
    }
    Ok(())
}

fn check_ownership(
    actual: Option<&pb::ChownOpt>,
    expected: Option<(u32, u32)>,
) -> Result<(), CheckError> {
    let Some((uid, gid)) = expected else {
        return if actual.is_none() { Ok(()) } else {
            Err(CheckError::Mismatch("undeclared file ownership override"))
        };
    };
    let owner = actual.ok_or(CheckError::Mismatch("missing image ownership override"))?;
    structure::known(owner)?;
    for (actual, expected) in [(owner.user.as_ref(), uid), (owner.group.as_ref(), gid)] {
        let actual = actual.ok_or(CheckError::Mismatch("missing numeric image owner"))?;
        structure::known(actual)?;
        if actual.user != Some(pb::user_opt::User::ById(expected)) {
            return Err(CheckError::Mismatch("image owner substitution or name lookup"));
        }
    }
    Ok(())
}
