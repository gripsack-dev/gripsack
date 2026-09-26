//! Historical v4 workspace reader. Its wire meanings are retained;
//! v5 execution/layout/OS requirements are never interpreted as v4.
//! The temporary v5-shaped projection is ONLY for shared read-only
//! semantic admission. It is discarded, never serialized or executed.

use crate::diagnostic::{Diagnostic, codes};
use crate::span::Span;
use crate::workspace::{
    CheckOutput, EnvironmentOutput, HookOutput, ImageOutput, LinuxWorker, PackageLayout,
    PackageOutput, PlatformAbi, PlatformArch, PlatformOs, ProfileOutput, RecipeExecution,
    RecipeOutput, RecipeOutputKind, ScheduleOutput, TaskOutput, Workspace, WorkspaceArg,
    WorkspaceCommand, WorkspaceFetch, WorkspaceOutput, WorkspacePlatform, WorkspaceProducer,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyWorkspaceV4 {
    pub span: Span,
    pub outputs: Vec<LegacyOutputV4>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegacyOutputV4 {
    Recipe(LegacyRecipeV4),
    Package(LegacyPackageV4),
    Environment(LegacyEnvironmentV4),
    Task(TaskOutput),
    Schedule(ScheduleOutput),
    Check(CheckOutput),
    Image(LegacyImageV4),
    Profile(ProfileOutput),
    Hook(HookOutput),
}

impl LegacyOutputV4 {
    pub fn name(&self) -> &str {
        match self {
            Self::Recipe(o) => &o.name,
            Self::Package(o) => &o.name,
            Self::Environment(o) => &o.name,
            Self::Task(o) => &o.name,
            Self::Schedule(o) => &o.name,
            Self::Check(o) => &o.name,
            Self::Image(o) => &o.name,
            Self::Profile(o) => &o.name,
            Self::Hook(o) => &o.name,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Recipe(_) => "recipe",
            Self::Package(_) => "package",
            Self::Environment(_) => "environment",
            Self::Task(_) => "task",
            Self::Schedule(_) => "schedule",
            Self::Check(_) => "check",
            Self::Image(_) => "image",
            Self::Profile(_) => "profile",
            Self::Hook(_) => "hook",
        }
    }

    pub fn span(&self) -> &Span {
        match self {
            Self::Recipe(o) => &o.span,
            Self::Package(o) => &o.span,
            Self::Environment(o) => &o.span,
            Self::Task(o) => &o.span,
            Self::Schedule(o) => &o.span,
            Self::Check(o) => &o.span,
            Self::Image(o) => &o.span,
            Self::Profile(o) => &o.span,
            Self::Hook(o) => &o.span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyExecutionV4 {
    Native,
    Host,
    IsolatedLinux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyLayoutV4 {
    Relocatable,
    FixedPrefix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyPlatformV4 {
    pub os: PlatformOs,
    pub arch: PlatformArch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_os: Option<String>,
}

impl LegacyPlatformV4 {
    fn for_validation(&self) -> WorkspacePlatform {
        let abi = match (self.os, self.abi.as_deref()) {
            (PlatformOs::Linux, Some("gnu")) => Some(PlatformAbi::Gnu),
            (PlatformOs::Linux, Some("musl")) => Some(PlatformAbi::Musl),
            (PlatformOs::Macos, Some("darwin")) => Some(PlatformAbi::Darwin),
            _ => None,
        };
        // The historical strings have no guaranteed version grammar.
        // Their exact equality is checked separately below, never
        // reinterpreted as a v5 minimum-OS requirement.
        WorkspacePlatform {
            os: self.os,
            arch: self.arch,
            abi,
            minimum_os: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyRecipeV4 {
    pub name: String,
    pub span: Span,
    pub source: WorkspaceFetch,
    pub execution: LegacyExecutionV4,
    pub output_kind: RecipeOutputKind,
    pub target: LegacyPlatformV4,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkspaceCommand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyPackageV4 {
    pub name: String,
    pub span: Span,
    pub producer: WorkspaceProducer,
    pub commands: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime: Vec<String>,
    pub target: LegacyPlatformV4,
    pub layout: LegacyLayoutV4,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyEnvironmentV4 {
    pub name: String,
    pub span: Span,
    pub packages: Vec<String>,
    pub target: LegacyPlatformV4,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, WorkspaceArg>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyImageV4 {
    pub name: String,
    pub span: Span,
    pub packages: Vec<String>,
    pub target: LegacyPlatformV4,
}

impl LegacyWorkspaceV4 {
    pub(crate) fn validation_projection(&self) -> Workspace {
        Workspace {
            span: self.span.clone(),
            outputs: self
                .outputs
                .iter()
                .map(|output| match output {
                    LegacyOutputV4::Recipe(r) => WorkspaceOutput::Recipe(RecipeOutput {
                        name: r.name.clone(),
                        span: r.span.clone(),
                        source: r.source.clone(),
                        // v4 execution is never promoted to v5 execution.
                        // This isolated marker is discarded after pure sema.
                        execution: RecipeExecution::IsolatedLinux {
                            worker: LinuxWorker::Buildkit,
                        },
                        output_kind: r.output_kind,
                        target: r.target.for_validation(),
                        steps: r.steps.clone(),
                        checks: r.checks.clone(),
                    }),
                    LegacyOutputV4::Package(p) => WorkspaceOutput::Package(PackageOutput {
                        name: p.name.clone(),
                        span: p.span.clone(),
                        producer: p.producer.clone(),
                        commands: p.commands.clone(),
                        runtime: p.runtime.clone(),
                        target: p.target.for_validation(),
                        // v4 fixed-prefix has no destination; v4-only
                        // selection rejection runs below, never substitute one.
                        layout: PackageLayout::Relocatable,
                    }),
                    LegacyOutputV4::Environment(e) => {
                        WorkspaceOutput::Environment(EnvironmentOutput {
                            name: e.name.clone(),
                            span: e.span.clone(),
                            packages: e.packages.clone(),
                            target: e.target.for_validation(),
                            prefix: None,
                            env: e.env.clone(),
                        })
                    }
                    LegacyOutputV4::Image(i) => WorkspaceOutput::Image(ImageOutput {
                        name: i.name.clone(),
                        span: i.span.clone(),
                        packages: i.packages.clone(),
                        target: i.target.for_validation(),
                    }),
                    LegacyOutputV4::Task(o) => WorkspaceOutput::Task(o.clone()),
                    LegacyOutputV4::Schedule(o) => WorkspaceOutput::Schedule(o.clone()),
                    LegacyOutputV4::Check(o) => WorkspaceOutput::Check(o.clone()),
                    LegacyOutputV4::Profile(o) => WorkspaceOutput::Profile(o.clone()),
                    LegacyOutputV4::Hook(o) => WorkspaceOutput::Hook(o.clone()),
                })
                .collect(),
        }
    }

    /// v4 compares its opaque target strings exactly and has no prefix
    /// destination field. This retains its old read-only admission
    /// behavior even when the v5 typed comparator admits newer floors.
    pub(crate) fn check_bindings(&self, diagnostics: &mut Vec<Diagnostic>) {
        let mut outputs = BTreeMap::new();
        for output in &self.outputs {
            outputs.entry(output.name()).or_insert(output);
        }
        for output in &self.outputs {
            match output {
                LegacyOutputV4::Package(package) => {
                    if let WorkspaceProducer::Recipe { recipe } = &package.producer
                        && let Some(LegacyOutputV4::Recipe(producer)) =
                            outputs.get(recipe.as_str()).copied()
                    {
                        compare_v4_targets(
                            output,
                            &package.target,
                            &producer.target,
                            &producer.span,
                            diagnostics,
                        );
                    }
                }
                LegacyOutputV4::Environment(environment) => check_selection(
                    output,
                    &environment.target,
                    &environment.packages,
                    &outputs,
                    diagnostics,
                ),
                LegacyOutputV4::Image(image) => check_selection(
                    output,
                    &image.target,
                    &image.packages,
                    &outputs,
                    diagnostics,
                ),
                _ => {}
            }
        }
    }
}

fn check_selection(
    consumer: &LegacyOutputV4,
    target: &LegacyPlatformV4,
    packages: &[String],
    outputs: &BTreeMap<&str, &LegacyOutputV4>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for name in packages {
        if let Some(LegacyOutputV4::Package(package)) = outputs.get(name.as_str()).copied() {
            compare_v4_targets(
                consumer,
                target,
                &package.target,
                &package.span,
                diagnostics,
            );
            if package.layout == LegacyLayoutV4::FixedPrefix {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!(
                            "{} `{}` selects v4 fixed_prefix package `{name}` without a declared install prefix",
                            consumer.kind(), consumer.name(),
                        ),
                    )
                    .with_label(Some(consumer.span().clone()), "selection declared here")
                    .with_label(Some(package.span.clone()), "package declared here"),
                );
            }
        }
    }
}

fn compare_v4_targets(
    consumer: &LegacyOutputV4,
    requested: &LegacyPlatformV4,
    provided: &LegacyPlatformV4,
    provider_span: &Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if requested != provided {
        diagnostics.push(Diagnostic::error(
            codes::UNKNOWN_WORKSPACE_REF,
            format!("v4 {} `{}` target mismatch: provider's declared OS/arch/ABI/minimum-OS strings differ", consumer.kind(), consumer.name()),
        )
        .with_label(Some(consumer.span().clone()), "consumer declared here")
        .with_label(Some(provider_span.clone()), "provider declared here"));
    }
}
