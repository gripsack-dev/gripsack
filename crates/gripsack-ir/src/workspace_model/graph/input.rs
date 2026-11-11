//! Captured-input references use a separate namespace from output graph edges.
//! These zero-allocation projections are consumed by the real admission walk.
use super::*;

#[derive(Debug, Clone, Copy)]
pub enum InputSite {
    Argument,
    PixiManifest,
    PixiLock,
}
#[derive(Debug, Clone, Copy)]
pub struct InputReference<'a> {
    pub to: &'a str,
    pub at: &'a Span,
    pub site: InputSite,
}
pub fn argument_input_reference<'a>(
    arg: &'a WorkspaceArg,
    at: &'a Span,
) -> Option<InputReference<'a>> {
    match arg {
        WorkspaceArg::Input { input } => Some(InputReference {
            to: input,
            at,
            site: InputSite::Argument,
        }),
        WorkspaceArg::Literal { .. }
        | WorkspaceArg::Artifact { .. }
        | WorkspaceArg::PackageCommand { .. }
        | WorkspaceArg::Source { .. }
        | WorkspaceArg::Output { .. } => None,
    }
}
pub fn source_input_references<'a>(
    source: &'a AcquisitionSource,
    at: &'a Span,
) -> [Option<InputReference<'a>>; 2] {
    match source {
        AcquisitionSource::Fetch(_)
        | AcquisitionSource::CondaEnvironment(_)
        | AcquisitionSource::Package { .. } => [None, None],
        AcquisitionSource::PixiLock(value) => [
            Some(InputReference {
                to: &value.manifest,
                at,
                site: InputSite::PixiManifest,
            }),
            Some(InputReference {
                to: &value.lock,
                at,
                site: InputSite::PixiLock,
            }),
        ],
    }
}
pub fn output_source_input_references(output: &WorkspaceOutput) -> [Option<InputReference<'_>>; 2] {
    match output {
        WorkspaceOutput::Recipe(value) => source_input_references(&value.source, &value.span),
        WorkspaceOutput::Package(value) => match &value.producer {
            WorkspaceProducer::Provider { provider } => {
                source_input_references(provider, &value.span)
            }
            WorkspaceProducer::Recipe { .. } => [None, None],
        },
        WorkspaceOutput::Environment(_)
        | WorkspaceOutput::Task(_)
        | WorkspaceOutput::Schedule(_)
        | WorkspaceOutput::Check(_)
        | WorkspaceOutput::Image(_)
        | WorkspaceOutput::Profile(_)
        | WorkspaceOutput::Hook(_)
        | WorkspaceOutput::Service(_) => [None, None],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pixi_source_references_its_manifest_and_its_lock() {
        let span = Span {
            file: "w.ts".into(),
            line: 1,
            col: None,
        };
        let source = AcquisitionSource::PixiLock(PixiLockSource {
            manifest: "manifest".into(),
            lock: "lock".into(),
            environment: "default".into(),
            span: span.clone(),
        });
        let [manifest, lock] = source_input_references(&source, &span);
        let (manifest, lock) = (manifest.unwrap(), lock.unwrap());
        assert_eq!((manifest.to, lock.to), ("manifest", "lock"));
        assert!(matches!(
            (manifest.site, lock.site),
            (InputSite::PixiManifest, InputSite::PixiLock)
        ));
    }
}
