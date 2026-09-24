//! Resolved provider/consumer target and prefix compatibility. No
//! ambient-host assumption or silent fixed-prefix relocation is
//! allowed by read-only workspace admission.

use super::{Edge, TargetBinding};
use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::{
    PackageLayout, PlatformArch, PlatformOs, WorkspaceOutput, WorkspacePlatform,
};

/// OS/arch/ABI agree and a producer's minimum OS floor cannot exceed
/// a consumer's floor. A missing consumer ABI is not a wildcard.
pub(super) fn check_binding(
    edge: &Edge,
    target: &WorkspaceOutput,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match edge.binding {
        TargetBinding::None => {}
        TargetBinding::Producer => {
            // Construction guarantees package → recipe here.
            let (WorkspaceOutput::Package(package), WorkspaceOutput::Recipe(recipe)) =
                (edge.from, target)
            else {
                return;
            };
            if !recipe.target.supports(&package.target) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!(
                            "{}: target mismatch — package `{}` targets {} but recipe `{}` \
                             targets {}; the producer's OS/arch/ABI must match and its minimum \
                             OS may not exceed the package target",
                            edge.relation,
                            package.name,
                            platform(&package.target),
                            recipe.name,
                            platform(&recipe.target)
                        ),
                    )
                    .with_label(Some(edge.at.clone()), "reference declared here")
                    .with_label(
                        Some(recipe.span.clone()),
                        format!("`{}` declared here", recipe.name),
                    ),
                );
            }
        }
        TargetBinding::Selection => {
            // Kind was verified by the caller: selections expect a
            // package; construction guarantees an environment or image
            // consumer.
            let WorkspaceOutput::Package(package) = target else {
                return;
            };
            let consumer = match edge.from {
                WorkspaceOutput::Environment(environment) => &environment.target,
                WorkspaceOutput::Image(image) => &image.target,
                _ => return,
            };
            if !package.target.supports(consumer) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!(
                            "{}: target mismatch — `{}` targets {} but package `{}` targets \
                             {}; the package OS/arch/ABI must match and its minimum OS may not \
                             exceed the selected consumer target",
                            edge.relation,
                            edge.from.name(),
                            platform(consumer),
                            package.name,
                            platform(&package.target)
                        ),
                    )
                    .with_label(Some(edge.at.clone()), "selection declared here")
                    .with_label(
                        Some(package.span.clone()),
                        format!("`{}` declared here", package.name),
                    ),
                );
            }
            if let PackageLayout::FixedPrefix { prefix } = &package.layout {
                let matches_destination = match edge.from {
                    WorkspaceOutput::Environment(environment) => {
                        environment.prefix.as_ref() == Some(prefix)
                    }
                    WorkspaceOutput::Image(_) => false,
                    _ => false,
                };
                if !matches_destination {
                    diagnostics.push(
                        Diagnostic::error(
                            codes::UNKNOWN_WORKSPACE_REF,
                            format!(
                                "{}: package `{}` has layout fixed_prefix at {:?}; the \
                                 selecting {} `{}` must declare exactly that install prefix \
                                 (image prefix materialization is unavailable until B4)",
                                edge.relation,
                                package.name,
                                prefix.as_str(),
                                edge.from.kind(),
                                edge.from.name()
                            ),
                        )
                        .with_label(Some(edge.at.clone()), "selection declared here")
                        .with_label(
                            Some(package.span.clone()),
                            format!("`{}` declared here", package.name),
                        ),
                    );
                }
            }
        }
    }
}

/// Diagnostic rendering of a platform requirement: `linux/x86_64`,
/// with ABI and minimum OS when declared.
fn platform(platform: &WorkspacePlatform) -> String {
    let os = match platform.os {
        PlatformOs::Linux => "linux",
        PlatformOs::Macos => "macos",
    };
    let arch = match platform.arch {
        PlatformArch::X86_64 => "x86_64",
        PlatformArch::Aarch64 => "aarch64",
    };
    let mut rendered = format!("{os}/{arch}");
    if let Some(abi) = platform.abi {
        rendered.push_str(match abi {
            crate::workspace::PlatformAbi::Gnu => "/gnu",
            crate::workspace::PlatformAbi::Musl => "/musl",
            crate::workspace::PlatformAbi::Darwin => "/darwin",
        });
    }
    if let Some(version) = platform.minimum_os {
        rendered.push_str(&format!(
            " (minimum OS {}.{}.{})",
            version.major,
            version.minor,
            version.patch.unwrap_or(0)
        ));
    }
    rendered
}
