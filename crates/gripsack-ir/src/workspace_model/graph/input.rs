//! Captured-input references use a separate namespace from output graph edges.
//! These zero-allocation projections are consumed by the real admission walk.
use super::*;
verus! {
#[derive(Debug, Clone, Copy)]
pub enum InputSite { Argument, PixiManifest, PixiLock }
#[derive(Debug, Clone, Copy)]
pub struct InputReference<'a> { pub to: &'a str, pub at: &'a Span, pub site: InputSite }
pub struct InputView<'a> { pub to: Seq<char>, pub at: &'a Span, pub site: InputSite }
pub open spec fn input_view<'a>(value: Option<InputReference<'a>>) -> Option<InputView<'a>> {
    match value { Some(v) => Some(InputView { to: v.to@, at: v.at, site: v.site }), None => None }
}
pub open spec fn argument_input_spec<'a>(arg: &'a WorkspaceArg, at: &'a Span) -> Option<InputView<'a>> {
    match arg {
        WorkspaceArg::Input { input } => Some(InputView { to: input@, at, site: InputSite::Argument }),
        WorkspaceArg::Literal { .. } | WorkspaceArg::Artifact { .. } | WorkspaceArg::PackageCommand { .. }
        | WorkspaceArg::Source { .. } | WorkspaceArg::Output { .. } => None,
    }
}
pub fn argument_input_reference<'a>(arg: &'a WorkspaceArg, at: &'a Span) -> (value: Option<InputReference<'a>>)
    ensures input_view(value) == argument_input_spec(arg, at),
{
    match arg {
        WorkspaceArg::Input { input } => Some(InputReference { to: input, at, site: InputSite::Argument }),
        WorkspaceArg::Literal { .. } | WorkspaceArg::Artifact { .. } | WorkspaceArg::PackageCommand { .. }
        | WorkspaceArg::Source { .. } | WorkspaceArg::Output { .. } => None,
    }
}
pub open spec fn source_inputs_spec<'a>(source: &'a AcquisitionSource, at: &'a Span) -> Seq<Option<InputView<'a>>> {
    match source {
        AcquisitionSource::Fetch(_) | AcquisitionSource::CondaEnvironment(_) => seq![None, None],
        AcquisitionSource::PixiLock(value) => seq![
            Some(InputView { to: value.manifest@, at, site: InputSite::PixiManifest }),
            Some(InputView { to: value.lock@, at, site: InputSite::PixiLock })],
    }
}
pub fn source_input_references<'a>(source: &'a AcquisitionSource, at: &'a Span) -> (values: [Option<InputReference<'a>>; 2])
    ensures values@.map_values(|v: Option<InputReference<'a>>| input_view(v)) =~= source_inputs_spec(source, at),
{
    match source {
        AcquisitionSource::Fetch(_) | AcquisitionSource::CondaEnvironment(_) => [None, None],
        AcquisitionSource::PixiLock(value) => [
            Some(InputReference { to: &value.manifest, at, site: InputSite::PixiManifest }),
            Some(InputReference { to: &value.lock, at, site: InputSite::PixiLock })],
    }
}
pub open spec fn output_source_inputs_spec<'a>(output: &'a WorkspaceOutput) -> Seq<Option<InputView<'a>>> {
    match output {
        WorkspaceOutput::Recipe(value) => source_inputs_spec(&value.source, &value.span),
        WorkspaceOutput::Package(value) => match &value.producer {
            WorkspaceProducer::Provider { provider } => source_inputs_spec(provider, &value.span),
            WorkspaceProducer::Recipe { .. } => seq![None, None],
        },
        WorkspaceOutput::Environment(_) | WorkspaceOutput::Task(_) | WorkspaceOutput::Schedule(_)
        | WorkspaceOutput::Check(_) | WorkspaceOutput::Image(_) | WorkspaceOutput::Profile(_)
        | WorkspaceOutput::Hook(_) => seq![None, None],
    }
}
pub fn output_source_input_references(output: &WorkspaceOutput) -> (values: [Option<InputReference<'_>>; 2])
    ensures values@.map_values(|v: Option<InputReference<'_>>| input_view(v)) =~= output_source_inputs_spec(output),
{
    match output {
        WorkspaceOutput::Recipe(value) => source_input_references(&value.source, &value.span),
        WorkspaceOutput::Package(value) => match &value.producer {
            WorkspaceProducer::Provider { provider } => source_input_references(provider, &value.span),
            WorkspaceProducer::Recipe { .. } => [None, None],
        },
        WorkspaceOutput::Environment(_) | WorkspaceOutput::Task(_) | WorkspaceOutput::Schedule(_)
        | WorkspaceOutput::Check(_) | WorkspaceOutput::Image(_) | WorkspaceOutput::Profile(_)
        | WorkspaceOutput::Hook(_) => [None, None],
    }
}
}
