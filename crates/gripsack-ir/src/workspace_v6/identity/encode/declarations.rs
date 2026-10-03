use super::{Encoder, PackageDigest, commands};
use crate::workspace_v6::lock::{
    BytecodePolicy, ChannelPriority, DefinitionPins, LockedCondaEnvironment, LockedNoArch,
    ReceiptPolicy, WorkspaceLock,
};
use crate::{
    HostFacts,
    workspace::{Weekday, WorkspaceCalendar, WorkspaceDestination},
    workspace_v6::*,
};
use gripsack_policy::semantic::canonical_entries;
use std::collections::BTreeMap;

fn strings(writer: &mut Encoder, values: &[String], ordered: bool) {
    if ordered {
        writer.number(values.len() as u64);
        for value in values {
            writer.text(value);
        }
        return;
    }
    // The verified canonicalization kernel (gripsack-policy semantic):
    // sorted, duplicate-free, determined by the entry set alone.
    let entries: Vec<(&[u8], &[u8], ())> = values
        .iter()
        .map(|value| (value.as_bytes(), &[][..], ()))
        .collect();
    let canonical = canonical_entries(&entries);
    writer.number(canonical.len() as u64);
    for (bytes, _, ()) in canonical {
        writer.field(bytes);
    }
}
fn steps(writer: &mut Encoder, values: &[WorkspaceStep]) {
    writer.number(values.len() as u64);
    for step in values {
        match step {
            WorkspaceStep::Command(command) => {
                writer.field(b"command");
                commands::declaration(writer, command);
            }
            WorkspaceStep::Action(WorkspaceAction::EnsureArtifact { output, .. }) => {
                writer.field(b"ensure-artifact");
                writer.text(output);
            }
        }
    }
}
fn locks(writer: &mut Encoder, values: &[WorkspaceMutationLock]) {
    // The verified canonicalization kernel: (scope, key) pair order,
    // duplicate-free, determined by the lock set alone.
    let entries: Vec<(&[u8], &[u8], ())> = values
        .iter()
        .map(|lock| {
            (
                match lock.scope {
                    MutationLockScope::User => &b"user"[..],
                    MutationLockScope::Store => &b"store"[..],
                },
                lock.key.as_bytes(),
                (),
            )
        })
        .collect();
    let canonical = canonical_entries(&entries);
    writer.number(canonical.len() as u64);
    for (scope, key, ()) in canonical {
        writer.field(scope);
        writer.field(key);
    }
}
fn file(writer: &mut Encoder, value: &WorkspaceFile) {
    match &value.source {
        None => writer.field(b"no-source"),
        Some(WorkspaceSource::RepoFile { path }) => {
            writer.field(b"repo-file");
            writer.text(path);
        }
        Some(WorkspaceSource::ArtifactFile { output, selector }) => {
            writer.field(b"artifact-file");
            writer.text(output);
            writer.text(selector);
        }
        Some(WorkspaceSource::Tree {
            output,
            include,
            exclude,
        }) => {
            writer.field(b"artifact-tree");
            writer.text(output);
            strings(writer, include, false);
            strings(writer, exclude, false);
        }
    }
    match &value.content {
        WorkspaceContent::Identity => writer.field(b"identity"),
        WorkspaceContent::Literal { text } => {
            writer.field(b"literal");
            writer.text(text);
        }
        WorkspaceContent::Template {
            template,
            variables,
            result_digest,
        } => {
            writer.field(b"template");
            writer.text(template);
            writer.number(variables.len() as u64);
            for (key, value) in variables {
                writer.text(key);
                writer.text(value);
            }
            writer.optional(result_digest);
        }
    }
    match &value.destination {
        WorkspaceDestination::Symlink { path } => {
            writer.field(b"symlink");
            writer.text(path);
        }
        WorkspaceDestination::TrackedCopy { path } => {
            writer.field(b"tracked-copy");
            writer.text(path);
        }
        WorkspaceDestination::ManagedBlock { path, marker } => {
            writer.field(b"managed-block");
            writer.text(path);
            writer.text(marker);
        }
    }
    writer.number(value.checks.len() as u64);
    for check in &value.checks {
        writer.text(&check.check);
        writer.field(match check.subject {
            FileCheckSubject::Source => b"source",
            FileCheckSubject::Rendered => b"rendered",
            FileCheckSubject::Deployed => b"deployed",
        });
        writer.field(match check.stage {
            FileCheckStage::PreFlip => b"pre-flip",
            FileCheckStage::PostLink => b"post-link",
            FileCheckStage::PostActivate => b"post-activate",
        });
    }
}
fn output(writer: &mut Encoder, output: &WorkspaceOutput) {
    writer.text(output.name());
    writer.text(output.kind());
    match output {
        WorkspaceOutput::Recipe(value) => {
            writer.source(&value.source);
            writer.execution(&value.execution);
            writer.platform(&value.target);
            writer.field(match value.output_kind {
                RecipeOutputKind::File => b"file",
                RecipeOutputKind::Tree => b"tree",
            });
            steps(writer, &value.steps);
            strings(writer, &value.checks, false);
        }
        WorkspaceOutput::Package(value) => {
            match &value.producer {
                WorkspaceProducer::Recipe { recipe } => {
                    writer.field(b"recipe");
                    writer.text(recipe);
                }
                WorkspaceProducer::Provider { provider } => {
                    writer.field(b"provider");
                    writer.source(provider);
                }
            }
            writer.platform(&value.target);
            writer.layout(&value.layout);
            writer.number(value.commands.len() as u64);
            for (name, path) in &value.commands {
                writer.text(name);
                writer.text(path);
            }
            strings(writer, &value.runtime, false);
        }
        WorkspaceOutput::Environment(value) => {
            writer.platform(&value.target);
            strings(writer, &value.packages, true);
            match &value.prefix {
                Some(prefix) => {
                    writer.field(b"prefix");
                    writer.text(prefix.as_str());
                }
                None => writer.field(b"no-prefix"),
            }
            writer.number(value.env.len() as u64);
            for (name, value) in &value.env {
                writer.text(name);
                commands::declared_argument(writer, value);
            }
        }
        WorkspaceOutput::Task(value) => {
            steps(writer, &value.steps);
            strings(writer, &value.deps, false);
            writer.optional(&value.environment);
            strings(writer, &value.checks, false);
            locks(writer, &value.mutation_locks);
            match &value.context {
                TaskContext::Host { mutable_paths } => {
                    writer.field(b"host");
                    strings(writer, mutable_paths, false);
                }
            }
        }
        WorkspaceOutput::Schedule(value) => {
            writer.text(&value.task);
            writer.field(b"user");
            match &value.trigger {
                WorkspaceCalendar::Daily { time } => {
                    writer.field(b"daily");
                    writer.text(time);
                }
                WorkspaceCalendar::Weekly { weekday, time } => {
                    writer.field(b"weekly");
                    writer.field(match weekday {
                        Weekday::Mon => b"mon",
                        Weekday::Tue => b"tue",
                        Weekday::Wed => b"wed",
                        Weekday::Thu => b"thu",
                        Weekday::Fri => b"fri",
                        Weekday::Sat => b"sat",
                        Weekday::Sun => b"sun",
                    });
                    writer.text(time);
                }
            }
        }
        WorkspaceOutput::Check(value) => {
            writer.text(&value.subject);
            commands::declaration(writer, &value.run);
        }
        WorkspaceOutput::Image(value) => {
            writer.platform(&value.target);
            strings(writer, &value.packages, true);
            writer.optional(&value.base);
            writer.number(value.destinations.len() as u64);
            for (package, destination) in &value.destinations {
                writer.text(package);
                writer.text(destination.path.as_str());
                writer.number(destination.owner.uid as u64);
                writer.number(destination.owner.gid as u64);
            }
            writer.number(value.config.entrypoint.len() as u64);
            for argument in &value.config.entrypoint {
                commands::declared_argument(writer, argument);
            }
            strings(writer, &value.config.args, false);
            writer.number(value.config.env.len() as u64);
            for (key, value) in &value.config.env {
                writer.text(key);
                writer.text(value);
            }
            writer.text(&value.config.cwd);
            writer.number(value.config.user.uid as u64);
            writer.number(value.config.user.gid as u64);
        }
        WorkspaceOutput::Profile(value) => {
            writer.optional(&value.environment);
            strings(writer, &value.schedules, true);
            strings(writer, &value.hooks, true);
            writer.number(value.files.len() as u64);
            for value in &value.files {
                file(writer, value);
            }
        }
        WorkspaceOutput::Hook(value) => {
            writer.field(match value.trigger {
                HookTrigger::PostLink => b"post-link",
                HookTrigger::PostActivate => b"post-activate",
                HookTrigger::OnRemove => b"on-remove",
            });
            commands::declaration(writer, &value.run);
        }
    }
}

pub(super) fn plan(
    workspace: &WorkspaceV6,
    facts: &HostFacts,
    definitions: &DefinitionPins,
    packages: &BTreeMap<String, PackageDigest>,
    lock: Option<&WorkspaceLock>,
) -> [u8; 32] {
    let mut writer = Encoder::new(b"gripsack/v6/admitted-plan");
    writer.definitions(definitions);
    writer.text(&facts.os);
    writer.text(&facts.arch);
    writer.optional(&facts.libc);
    strings(&mut writer, &facts.tags, false);
    let mut outputs: Vec<_> = workspace.outputs.iter().collect();
    outputs.sort_unstable_by_key(|output| output.name());
    writer.number(outputs.len() as u64);
    for value in outputs {
        output(&mut writer, value);
    }
    let mut inputs: Vec<_> = workspace.inputs.iter().collect();
    inputs.sort_unstable_by_key(|input| &input.name);
    writer.number(inputs.len() as u64);
    for input in inputs {
        writer.text(&input.name);
        match &input.origin {
            InputOrigin::RepoFile { path } => {
                writer.field(b"repo-file");
                writer.text(path);
            }
            InputOrigin::RepoDirectory {
                path,
                include,
                exclude,
            } => {
                writer.field(b"repo-directory");
                writer.text(path);
                strings(&mut writer, include, false);
                strings(&mut writer, exclude, false);
            }
        }
    }
    locks(&mut writer, &workspace.mutation_locks);
    writer.number(packages.len() as u64);
    for (name, package) in packages {
        writer.text(name);
        writer.field(package.bytes());
    }
    match lock {
        Some(lock) => {
            writer.field(b"lock");
            writer.field(&self::lock(lock));
        }
        None => writer.field(b"no-lock"),
    }
    writer.finish()
}

pub(super) fn lock(lock: &WorkspaceLock) -> [u8; 32] {
    let mut writer = Encoder::new(b"gripsack/v6/workspace-lock");
    writer.number(lock.lock_version.into());
    writer.number(lock.resolutions.len() as u64);
    writer.definitions(&lock.definitions);
    for (platform, resolution) in &lock.resolutions {
        writer.text(platform);
        let mut pins: Vec<_> = resolution.pins.iter().collect();
        pins.sort_unstable_by_key(|pin| &pin.output);
        writer.number(pins.len() as u64);
        for pin in pins {
            writer.text(&pin.output);
            writer.locked_source(&pin.source);
            writer.optional(&pin.resolved.url);
            writer.optional(&pin.resolved.version);
            writer.optional(&pin.resolved.sha256);
            writer.optional(&pin.resolved.tree256);
            writer.optional(&pin.resolved.api_url);
            writer.optional(&pin.resolved.repo256);
            match &pin.conda {
                Some(environment) => {
                    writer.field(b"conda");
                    conda(&mut writer, environment);
                }
                None => writer.field(b"no-conda"),
            }
        }
        let mut transitive: Vec<_> = resolution.transitive.iter().collect();
        transitive.sort_unstable_by_key(|pin| &pin.name);
        writer.number(transitive.len() as u64);
        for pin in transitive {
            writer.text(&pin.name);
            writer.optional(&pin.version);
            writer.text(&pin.sha256);
            writer.optional(&pin.source);
        }
    }
    writer.finish()
}

/// The complete frozen Conda closure fold: every normalized record
/// field in canonical order. Collections are validity-checked into
/// canonical order by the lock reader, so encoding order is stable.
fn conda(writer: &mut Encoder, environment: &LockedCondaEnvironment) {
    writer.text(&environment.platform);
    writer.number(environment.channels.len() as u64);
    for channel in &environment.channels {
        writer.text(channel);
    }
    writer.field(match environment.channel_priority {
        ChannelPriority::Strict => b"strict",
        ChannelPriority::Flexible => b"flexible",
    });
    writer.number(environment.system_requirements.virtual_packages.len() as u64);
    for requirement in &environment.system_requirements.virtual_packages {
        writer.text(&requirement.name);
        writer.text(&requirement.minimum_version);
        writer.optional(&requirement.build);
    }
    writer.optional(&environment.system_requirements.archspec);
    writer.number(environment.virtual_packages.len() as u64);
    for package in &environment.virtual_packages {
        writer.text(&package.name);
        writer.text(&package.version);
        writer.text(&package.build);
    }
    writer.number(environment.packages.len() as u64);
    for package in &environment.packages {
        writer.text(&package.name);
        writer.text(&package.version);
        writer.text(&package.build);
        writer.number(package.build_number);
        writer.text(&package.subdir);
        writer.text(&package.channel);
        writer.text(&package.url);
        writer.text(&package.sha256);
        optional_number(writer, package.size);
        optional_number(writer, package.timestamp);
        optional_number(writer, package.indexed_timestamp);
        writer.optional(&package.attestations_sha256);
        writer.optional(&package.md5);
        writer.optional(&package.legacy_bz2_md5);
        optional_number(writer, package.legacy_bz2_size);
        writer.optional(&package.arch);
        writer.optional(&package.platform);
        writer.field(match package.noarch {
            LockedNoArch::None => b"noarch-none",
            LockedNoArch::Generic => b"noarch-generic",
            LockedNoArch::Python => b"noarch-python",
        });
        writer.optional(&package.license);
        writer.optional(&package.license_family);
        strings(writer, &package.depends, true);
        strings(writer, &package.constrains, true);
        writer.number(package.extra_depends.len() as u64);
        for (extra, dependencies) in &package.extra_depends {
            writer.text(extra);
            strings(writer, dependencies, true);
        }
        strings(writer, &package.flags, true);
        writer.optional(&package.python_site_packages_path);
        if let Some(exports) = &package.run_exports {
            writer.field(b"run-exports");
            for requirements in [
                &exports.weak,
                &exports.strong,
                &exports.noarch,
                &exports.weak_constrains,
                &exports.strong_constrains,
            ] {
                strings(writer, requirements, true);
            }
        } else {
            writer.field(b"no-run-exports");
        }
        if let Some(purls) = &package.purls {
            writer.field(b"package-urls");
            writer.number(purls.len() as u64);
            for purl in purls {
                writer.text(purl);
            }
        } else {
            writer.field(b"no-package-urls");
        }
        strings(writer, &package.track_features, true);
        writer.optional(&package.features);
    }
    writer.field(match environment.materializer.bytecode {
        BytecodePolicy::Suppress => b"bytecode-suppress",
    });
    writer.field(match environment.materializer.receipt {
        ReceiptPolicy::NormalizedCondaMeta => b"receipt-normalized-conda-meta",
    });
}

fn optional_number(writer: &mut Encoder, value: Option<u64>) {
    match value {
        Some(value) => {
            writer.field(b"some");
            writer.number(value);
        }
        None => writer.field(b"none"),
    }
}

pub(super) fn conda_closure(environment: &LockedCondaEnvironment) -> [u8; 32] {
    let mut writer = Encoder::new(b"gripsack/v6/conda-closure");
    conda(&mut writer, environment);
    writer.finish()
}
