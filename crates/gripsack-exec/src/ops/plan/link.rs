//! Owned links through the kernel's per-unit decision (plan/0056 K1).
//!
//! A link is gripsack's only when the manifest records exactly that link:
//! neither a record alone (FND-1: a user's replacement of an owned link was
//! re-pointed) nor a target under `$GRIPSACK_HOME` (an unrecorded user link
//! into the store was re-pointed) confers ownership.
use super::*;

pub(super) fn plan_link(
    view: &DestView,
    source: &Path,
    content_hash: store::hash::ManifestHash,
) -> Result<Op, ExecError> {
    let link_intent = Intended::Object(ObjectIdentity::Link(source.to_string_lossy().into_owned()));
    let link = |authority| {
        view.base(
            OpKind::Link {
                target: source.to_path_buf(),
            },
            Some(authority),
            link_intent.clone(),
        )
        .with_produces(view.produces(content_hash.clone(), None, false))
    };
    let intended = store::hash::kernel::link(source.as_os_str().as_encoded_bytes());
    Ok(match view.decide(intended) {
        Decision::Satisfied => view
            .base(OpKind::Satisfied, None, link_intent.clone())
            .with_produces(view.produces(content_hash.clone(), None, false)),
        Decision::Create => link(Authority::Fresh),
        Decision::ReplaceOwn => link(Authority::Update),
        Decision::TakeOver => link(Authority::TakeOver),
        // rendered "needs --take-over"; apply refuses, rollback keeps
        Decision::Refuse(why) => refuse(view, why)?,
    })
}

/// A refused owned link as an inert op the caller turns into a refusal:
/// a drifted or foreign object stands, an object without identity is an
/// error.
fn refuse(view: &DestView, why: Refusal) -> Result<Op, ExecError> {
    let authority = match why {
        Refusal::Drifted => Authority::Drifted,
        Refusal::Unowned => Authority::Foreign,
        Refusal::Unsupported => return Err(unsupported(view)),
    };
    Ok(view.base(
        OpKind::Preserved,
        Some(authority),
        Intended::Object(
            view.observed_identity()
                .expect("a refused destination holds an object"),
        ),
    ))
}

/// The kernel refused an object without an identity it can record.
pub(super) fn unsupported(view: &DestView) -> ExecError {
    ExecError::Step {
        output: view.output.into(),
        step: "plan".into(),
        detail: format!(
            "{} is neither a regular file nor a symlink — move it aside to continue",
            view.entry.to
        ),
    }
}
