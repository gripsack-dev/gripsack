//! Resolved producer and consumer target/layout compatibility. The
//! read-only v4 wire cannot install a fixed-prefix package into a
//! prefix-less environment or image; no host fallback is admitted.

use super::{Edge, TargetBinding};
use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::{
    PackageLayout, PlatformArch, PlatformOs, WorkspaceOutput, WorkspacePlatform,
};

/// Platform-bound edges: conservative v4 identity is exact
/// os/arch/ABI/minimum-OS equality — never a host-compatibility
/// assumption for cross-target builds (0052 §2.2, §4).
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
            if package.target != recipe.target {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!(
                            "{}: target mismatch — package `{}` targets {} but recipe `{}` \
                             targets {}; a package and its producer recipe must agree exactly \
                             (os/arch/ABI/minimum OS)",
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
            if *consumer != package.target {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!(
                            "{}: target mismatch — `{}` targets {} but package `{}` targets \
                             {}; a selection and its package must agree exactly \
                             (os/arch/ABI/minimum OS)",
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
            if package.layout == PackageLayout::FixedPrefix {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!(
                            "{}: package `{}` has layout fixed_prefix, which requires a \
                             consumer-declared installation prefix; the v4 wire has no consumer \
                             prefix slot, so a fixed_prefix package cannot be selected by an \
                             environment or image",
                            edge.relation, package.name
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
    if let Some(abi) = &platform.abi {
        rendered.push_str(&format!("/{abi}"));
    }
    if let Some(minimum_os) = &platform.minimum_os {
        rendered.push_str(&format!(" (minimum OS {minimum_os})"));
    }
    rendered
}
