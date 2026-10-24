//! Owned-link policy lifting into executable operations.
use super::*;

pub(super) fn plan_link(
    view: &DestView,
    source: &Path,
    content_hash: store::hash::ManifestHash,
    already: bool,
) -> Op {
    use crate::deploy::LinkPlan;
    let exists = view.observed.is_some();
    let ours = match &view.observed {
        Some(crate::deploy::Observation::Symlink { target }) => {
            Path::new(target.as_os_str()).starts_with(view.home)
        }
        _ => false,
    };
    let recorded = view.prev.is_some_and(|e| !e.preserved_drift);
    let link_intent = Intended::Object(ObjectIdentity::Link(source.to_string_lossy().into_owned()));
    match crate::deploy::plan_link(exists, ours, recorded, view.take_over) {
        LinkPlan::Link if already => view
            .base(OpKind::Satisfied, None, link_intent)
            .with_produces(view.produces(content_hash, None, false)),
        LinkPlan::Link => view
            .base(
                OpKind::Link {
                    target: source.to_path_buf(),
                },
                Some(Authority::Fresh),
                link_intent,
            )
            .with_produces(view.produces(content_hash, None, false)),
        LinkPlan::TakeOver => view
            .base(
                OpKind::Link {
                    target: source.to_path_buf(),
                },
                Some(Authority::TakeOver),
                link_intent,
            )
            .with_produces(view.produces(content_hash, None, false)),
        // rendered "foreign — needs --take-over"; apply refuses
        LinkPlan::Refuse => view.base(
            OpKind::Preserved,
            Some(Authority::Foreign),
            Intended::Object(view.observed_identity().expect("refuse implies an object")),
        ),
    }
}
