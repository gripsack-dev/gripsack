use super::fail;
use crate::sema::workspace::span_value::{
    admissible_destination, admissible_repo_file, valid_local_time,
};
use crate::{Diagnostic, Span, codes, workspace::PlatformOs, workspace_model::*};
use gripsack_policy::workspace_command::{ArgumentPosition, admit_argument, admit_bash_options};

pub(super) fn selector(value: &str) -> bool {
    value == "." || admissible_repo_file(value)
}
fn span(value: &Span, out: &mut Vec<Diagnostic>) {
    if value.file.is_empty() || value.line == 0 || value.col == Some(0) {
        fail(
            out,
            codes::BAD_WORKSPACE_SPAN,
            value,
            "a declaration needs a nonempty source file and positive line/column",
        );
    }
}
fn invalid(out: &mut Vec<Diagnostic>, at: &Span, message: &'static str) {
    fail(out, codes::INVALID_WORKSPACE_VALUE, at, message);
}
fn hash(value: &str) -> bool {
    crate::workspace_model::identity::ArtifactDigest::parse(value).is_ok()
}
fn globs(include: &[String], exclude: &[String]) -> bool {
    !include.is_empty()
        && include
            .iter()
            .chain(exclude)
            .all(|pattern| admissible_repo_file(pattern))
}
fn lock(value: &WorkspaceMutationLock, out: &mut Vec<Diagnostic>) {
    span(&value.span, out);
    if value.key.is_empty()
        || value.key.len() > 128
        || !value.key.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        invalid(
            out,
            &value.span,
            "mutation-lock keys must be 1..=128 printable non-whitespace ASCII bytes",
        );
    }
}
/// The v6 acquisition grammar (A3): per-variant provenance spans, the
/// legacy Brew/Pixi fetch spellings rejected (superseded by the Conda
/// lanes), and bounded lexical checks on Conda/Pixi declarations.
fn source(value: &AcquisitionSource, target: &WorkspacePlatform, out: &mut Vec<Diagnostic>) {
    span(value.span(), out);
    match value {
        AcquisitionSource::Fetch(fetch) => {
            if matches!(
                fetch.fetch,
                crate::FetchSpec::Brew { .. } | crate::FetchSpec::Pixi { .. }
            ) {
                invalid(
                    out,
                    value.span(),
                    "brew/pixi fetch spellings are superseded by conda_environment/pixi_lock sources",
                );
            }
        }
        AcquisitionSource::CondaEnvironment(source) => {
            if source.channels.is_empty()
                || source
                    .channels
                    .iter()
                    .any(|channel| !admissible_text(channel))
            {
                invalid(
                    out,
                    &source.span,
                    "conda environments need nonempty text channels in priority order",
                );
            }
            if source.packages.is_empty() {
                invalid(out, &source.span, "conda environments need packages");
            }
            for (name, spec) in &source.packages {
                if !admissible_text(name)
                    || name != &name.to_ascii_lowercase()
                    || !admissible_text(spec)
                {
                    invalid(
                        out,
                        &source.span,
                        "conda package names are lowercase text with a nonempty MatchSpec",
                    );
                }
            }
            if let Some(requirements) = &source.system_requirements {
                if let Err(message) = requirements.validate() {
                    invalid(out, &source.span, message);
                }
                if target.os != PlatformOs::Linux
                    || source
                        .platforms
                        .iter()
                        .any(|platform| platform.os != PlatformOs::Linux)
                {
                    invalid(
                        out,
                        &source.span,
                        "conda system requirements support only Linux",
                    );
                }
            }
            let mut seen: Vec<&WorkspacePlatform> = Vec::new();
            for platform in &source.platforms {
                if !platform.valid_abi() || seen.contains(&platform) {
                    invalid(
                        out,
                        &source.span,
                        "conda platforms need a compatible ABI and no duplicates",
                    );
                }
                seen.push(platform);
            }
        }
        AcquisitionSource::PixiLock(source) => {
            if !admissible_text(&source.manifest)
                || !admissible_text(&source.lock)
                || !admissible_text(&source.environment)
            {
                invalid(
                    out,
                    &source.span,
                    "pixi imports name workspace inputs and an environment with nonempty text",
                );
            }
        }
    }
}
fn admissible_text(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}
/// Layout/source coherence (A3): Conda-backed packages materialize at a
/// derived prefix; no other producer may claim prefix materialization.
fn layout(value: &PackageOutput, out: &mut Vec<Diagnostic>) {
    let conda = matches!(
        &value.producer,
        WorkspaceProducer::Provider {
            provider: AcquisitionSource::CondaEnvironment(_) | AcquisitionSource::PixiLock(_)
        }
    );
    match &value.layout {
        CatalogPackageLayout::FixedPrefix { prefix } => {
            if !prefix.is_safe() {
                invalid(
                    out,
                    &value.span,
                    "fixed prefix must be a normalized absolute POSIX path below /",
                );
            }
            if conda {
                invalid(
                    out,
                    &value.span,
                    "conda-backed packages materialize at a derived prefix, not a declared literal",
                );
            }
        }
        CatalogPackageLayout::Relocatable if conda => {
            invalid(
                out,
                &value.span,
                "conda-backed packages are prefix-materialized, not relocatable",
            );
        }
        CatalogPackageLayout::PrefixMaterialized if !conda => {
            invalid(
                out,
                &value.span,
                "only conda-backed provider packages are prefix-materialized",
            );
        }
        _ => {}
    }
}
pub(super) fn check(workspace: &WorkspaceCatalog, out: &mut Vec<Diagnostic>) {
    span(&workspace.span, out);
    if workspace.outputs.is_empty() {
        invalid(out, &workspace.span, "workspace declares no outputs");
    }
    if workspace
        .name
        .as_ref()
        .is_some_and(|name| name.is_empty() || name.chars().any(char::is_control))
    {
        invalid(
            out,
            &workspace.span,
            "workspace display name must be nonempty and contain no controls",
        );
    }
    for input in &workspace.inputs {
        span(&input.span, out);
        if input.name.is_empty() {
            invalid(out, &input.span, "workspace input needs a name");
        }
        let valid = match &input.origin {
            InputOrigin::RepoFile { path } => admissible_repo_file(path),
            InputOrigin::RepoDirectory {
                path,
                include,
                exclude,
            } => selector(path) && globs(include, exclude),
        };
        if !valid {
            invalid(
                out,
                &input.span,
                "captured input requires normalized repository-relative paths and explicit include patterns",
            );
        }
    }
    for value in &workspace.mutation_locks {
        lock(value, out);
    }
    for output in &workspace.outputs {
        span(output.span(), out);
        if output.name().is_empty() || output.name().contains('\0') {
            invalid(
                out,
                output.span(),
                "output names must be nonempty and contain no NUL",
            );
        }
        let target = match output {
            WorkspaceOutput::Recipe(value) => Some(&value.target),
            WorkspaceOutput::Package(value) => Some(&value.target),
            WorkspaceOutput::Environment(value) => Some(&value.target),
            WorkspaceOutput::Image(value) => Some(&value.target),
            _ => None,
        };
        if target.is_some_and(|target| !target.valid_abi()) {
            invalid(
                out,
                output.span(),
                "target ABI is incompatible with its operating system",
            );
        }
        match output {
            WorkspaceOutput::Recipe(value) => {
                source(&value.source, &value.target, out);
                if let RecipeExecution::IsolatedLinux {
                    platform,
                    toolchain,
                    ..
                } = &value.execution
                {
                    if platform.os != PlatformOs::Linux || !platform.valid_abi() {
                        invalid(
                            out,
                            &value.span,
                            "isolated execution requires an explicit compatible Linux platform",
                        );
                    }
                    if !image_reference(&toolchain.reference) {
                        invalid(
                            out,
                            &value.span,
                            "toolchain image requires a normalized registry path and lowercase SHA-256 digest",
                        );
                    }
                }
                steps(&value.steps, out);
            }
            WorkspaceOutput::Package(value) => {
                if let WorkspaceProducer::Provider { provider } = &value.producer {
                    source(provider, &value.target, out);
                }
                layout(value, out);
                for (name, path) in &value.commands {
                    if name.is_empty()
                        || name.contains('/')
                        || name.contains('\0')
                        || !admissible_repo_file(path)
                    {
                        invalid(
                            out,
                            &value.span,
                            "exported commands need a plain name and normalized relative payload path",
                        );
                    }
                }
            }
            WorkspaceOutput::Environment(value) => {
                if value
                    .prefix
                    .as_ref()
                    .is_some_and(|prefix| !prefix.is_safe())
                {
                    invalid(
                        out,
                        &value.span,
                        "environment prefix must be normalized and absolute",
                    );
                }
                environment(&value.env, &value.span, out);
            }
            WorkspaceOutput::Task(value) => {
                if value.steps.is_empty() {
                    invalid(out, &value.span, "a task requires at least one local step");
                }
                steps(&value.steps, out);
                for value in &value.mutation_locks {
                    lock(value, out);
                }
                match &value.context {
                    TaskContext::Host { mutable_paths } => {
                        if mutable_paths
                            .iter()
                            .any(|path| !admissible_destination(path))
                        {
                            invalid(
                                out,
                                &value.span,
                                "mutable host paths must be normalized absolute or ~/ paths",
                            );
                        }
                    }
                }
            }
            WorkspaceOutput::Schedule(value) => {
                let time = match &value.trigger {
                    WorkspaceCalendar::Daily { time } | WorkspaceCalendar::Weekly { time, .. } => {
                        time
                    }
                };
                if !valid_local_time(time) {
                    invalid(out, &value.span, "schedule time must be HH:MM local time");
                }
            }
            WorkspaceOutput::Check(value) => command(&value.run, out),
            WorkspaceOutput::Hook(value) => command(&value.run, out),
            WorkspaceOutput::Profile(value) => {
                for file in &value.files {
                    profile_file(file, out);
                }
            }
            WorkspaceOutput::Image(value) => super::image::check(value, out),
        }
    }
    crate::sema::workspace::destinations::check_declarations(
        workspace
            .outputs
            .iter()
            .filter_map(|output| match output {
                WorkspaceOutput::Profile(profile) => Some(profile),
                _ => None,
            })
            .flat_map(|profile| {
                profile
                    .files
                    .iter()
                    .filter(|file| !matches!(file.source, Some(WorkspaceSource::Tree { .. })))
                    .map(|file| (&file.span, &file.destination))
            }),
        out,
    );
}
fn profile_file(file: &WorkspaceFile, out: &mut Vec<Diagnostic>) {
    span(&file.span, out);
    if !matches!(file.content, WorkspaceContent::Literal { .. }) && file.source.is_none() {
        invalid(
            out,
            &file.span,
            "only literal file content may omit its source",
        );
    }
    match &file.source {
        Some(WorkspaceSource::RepoFile { path }) if !admissible_repo_file(path) => invalid(
            out,
            &file.span,
            "file origin must be a normalized repository-relative file",
        ),
        Some(WorkspaceSource::ArtifactFile { selector: path, .. })
            if !admissible_repo_file(path) =>
        {
            invalid(
                out,
                &file.span,
                "artifact file selector must name a relative file",
            )
        }
        Some(WorkspaceSource::Tree {
            include, exclude, ..
        }) if !globs(include, exclude) => invalid(
            out,
            &file.span,
            "artifact tree requires normalized relative include/exclude patterns",
        ),
        _ => {}
    }
    if let WorkspaceContent::Template {
        result_digest: Some(digest),
        ..
    } = &file.content
        && !hash(digest)
    {
        invalid(
            out,
            &file.span,
            "rendered-content digest must be lowercase SHA-256",
        );
    }
    let destination = match &file.destination {
        WorkspaceDestination::Symlink { path }
        | WorkspaceDestination::TrackedCopy { path }
        | WorkspaceDestination::ManagedBlock { path, .. } => path,
    };
    if !admissible_destination(destination) {
        fail(
            out,
            codes::BAD_DESTINATION,
            &file.span,
            "destination must be a normalized absolute or ~/ path",
        );
    }
    if let WorkspaceDestination::ManagedBlock { marker, .. } = &file.destination
        && (marker.is_empty() || marker.chars().any(char::is_control))
    {
        invalid(
            out,
            &file.span,
            "managed-block marker must be nonempty and contain no controls",
        );
    }
    for check in &file.checks {
        span(&check.span, out);
    }
}
fn steps(steps: &[WorkspaceStep], out: &mut Vec<Diagnostic>) {
    for step in steps {
        match step {
            WorkspaceStep::Command(value) => command(value, out),
            WorkspaceStep::Action(action) => span(action.span(), out),
        }
    }
}
fn environment(
    values: &std::collections::BTreeMap<String, WorkspaceArg>,
    at: &Span,
    out: &mut Vec<Diagnostic>,
) {
    for (name, value) in values {
        if name.is_empty() || name.contains(['=', '\0']) {
            invalid(
                out,
                at,
                "environment name cannot be empty or contain '=' or NUL",
            );
        }
        argument(value, at, out);
        if !admit_argument(ArgumentPosition::Environment, value.policy_origin()) {
            fail(
                out,
                codes::BAD_WORKSPACE_CONTEXT,
                at,
                "package commands are executable references, not environment data",
            );
        }
    }
}
pub(super) fn argument(value: &WorkspaceArg, at: &Span, out: &mut Vec<Diagnostic>) {
    match value {
        WorkspaceArg::Literal { value } if value.contains('\0') => {
            invalid(out, at, "argv/environment values cannot contain NUL")
        }
        WorkspaceArg::Artifact { selector: path, .. }
        | WorkspaceArg::Source { selector: path }
        | WorkspaceArg::Output { selector: path }
            if !selector(path) =>
        {
            invalid(
                out,
                at,
                "selector must be '.' or a normalized relative path",
            )
        }
        WorkspaceArg::PackageCommand {
            sha256: Some(digest),
            ..
        } if !hash(digest) => invalid(
            out,
            at,
            "claimed executable digest must be lowercase SHA-256",
        ),
        _ => {}
    }
}
fn command(value: &WorkspaceCommand, out: &mut Vec<Diagnostic>) {
    span(value.span(), out);
    let (env, cwd) = match value {
        WorkspaceCommand::Exec { argv, env, cwd, .. } => {
            if argv.is_empty() {
                invalid(out, value.span(), "exec requires an executable argument");
            }
            for argument_value in argv {
                argument(argument_value, value.span(), out);
            }
            (env, cwd)
        }
        WorkspaceCommand::RunBash {
            interpreter,
            options,
            body,
            env,
            cwd,
            line_map,
            ..
        } => {
            argument(interpreter, value.span(), out);
            if !admit_argument(ArgumentPosition::Interpreter, interpreter.policy_origin()) {
                fail(
                    out,
                    codes::BAD_WORKSPACE_CONTEXT,
                    value.span(),
                    "Bash requires an explicit package-command interpreter",
                );
            }
            if !admit_bash_options(options) {
                invalid(
                    out,
                    value.span(),
                    "Bash options must be the fixed strict set -e -u -o pipefail",
                );
            }
            if let Some(offset) = body.find("${") {
                let generated = body[..offset].bytes().filter(|byte| *byte == b'\n').count();
                let mut at = value.span().clone();
                if let Some(line) = line_map.get(generated) {
                    at.line = *line;
                }
                invalid(
                    out,
                    &at,
                    "Bash bodies are literal; bind dynamic values through typed argv/environment arguments",
                );
            }
            if line_map.contains(&0) {
                invalid(
                    out,
                    value.span(),
                    "Bash line-map entries must be positive source lines",
                );
            }
            (env, cwd)
        }
    };
    environment(env, value.span(), out);
    if let Some(path) = cwd {
        let valid = match path {
            WorkspacePath::Literal { value } => {
                selector(value) || value == "/" || admissible_destination(value)
            }
            WorkspacePath::Artifact {
                selector: value, ..
            } => selector(value),
            WorkspacePath::Host { path } => admissible_destination(path),
            WorkspacePath::Source { selector: value }
            | WorkspacePath::Output { selector: value } => selector(value),
        };
        if !valid {
            invalid(
                out,
                value.span(),
                "working directory requires a normalized typed path",
            );
        }
    }
}
pub(super) fn image_reference(value: &str) -> bool {
    let Some((name, digest)) = value.rsplit_once("@sha256:") else {
        return false;
    };
    name.contains('/')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.contains("//")
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"./_:-".contains(&byte)
        })
        && hash(digest)
}

#[cfg(test)]
mod baseline_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_conda_baseline_diagnostics_use_the_source_span() {
        for (requirements, platforms, os, valid) in [
            (
                json!({"libc":{"family":"glibc","version":"2.28"},"linux":"4.18"}),
                json!([]),
                "linux",
                true,
            ),
            (
                json!({"libc":{"family":"musl","version":"1.2"}}),
                json!([]),
                "linux",
                false,
            ),
            (json!({"linux":">=4.18"}), json!([]), "linux", false),
            (
                json!({"linux":"4.18"}),
                json!([{"os":"macos","arch":"aarch64"}]),
                "linux",
                false,
            ),
            (json!({"linux":"4.18"}), json!([]), "macos", false),
        ] {
            let declared: AcquisitionSource = serde_json::from_value(json!({
                "kind":"conda_environment",
                "channels":["conda-forge"], "packages":{"python":"*"},
                "platforms":platforms, "system_requirements":requirements,
                "span":{"file":"baseline.ts","line":12}
            }))
            .unwrap();
            let mut diagnostics = Vec::new();
            let target: WorkspacePlatform =
                serde_json::from_value(json!({"os":os,"arch":"aarch64"})).unwrap();
            source(&declared, &target, &mut diagnostics);
            assert_eq!(diagnostics.is_empty(), valid, "{diagnostics:?}");
            if !valid {
                assert!(diagnostics.iter().all(|diagnostic| {
                    diagnostic
                        .labels
                        .iter()
                        .any(|label| label.span.as_ref() == Some(declared.span()))
                }));
            }
        }
    }
}
