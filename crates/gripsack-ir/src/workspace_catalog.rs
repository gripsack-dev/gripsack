//! Borrowed metadata for the current workspace output catalog.
use crate::{Ir, Span};

#[derive(Debug, Clone, Copy)]
pub struct CatalogOutput<'a> {
    pub name: &'a str,
    pub kind: &'static str,
    pub span: &'a Span,
}

impl Ir {
    pub fn workspace_outputs(&self) -> impl Iterator<Item = CatalogOutput<'_>> {
        self.workspace.outputs.iter().map(|output| CatalogOutput {
            name: output.name(),
            kind: output.kind(),
            span: output.span(),
        })
    }

    /// Declaration spans (outputs and, for profiles, their files) by owner.
    pub fn output_owners(&self) -> OutputOwners {
        use crate::workspace_model::WorkspaceOutput;
        let mut entries = Vec::new();
        for output in &self.workspace.outputs {
            let owner = (output.kind(), output.name().to_owned());
            entries.push((output.span().clone(), owner.0, owner.1.clone()));
            if let WorkspaceOutput::Profile(profile) = output {
                for file in &profile.files {
                    entries.push((file.span.clone(), owner.0, owner.1.clone()));
                }
            }
        }
        OutputOwners { entries }
    }
}

/// Maps declaration spans back to the output that owns them.
#[derive(Debug, Clone, Default)]
pub struct OutputOwners {
    entries: Vec<(Span, &'static str, String)>,
}

impl OutputOwners {
    pub fn extend(&mut self, other: OutputOwners) {
        self.entries.extend(other.entries);
    }

    /// Name the output a diagnostic belongs to (`package "hunk": ...`).
    ///
    /// The first labeled span selects its owner. Messages which already quote
    /// the name, and spans shared by several outputs, are left as raised:
    /// guessing would misattribute.
    pub fn attribute(&self, diagnostic: &mut crate::Diagnostic) {
        let Some(span) = diagnostic.labels.iter().find_map(|label| label.span.as_ref()) else {
            return;
        };
        let mut owners = std::collections::BTreeSet::new();
        for (owned, kind, name) in &self.entries {
            if owned == span {
                owners.insert((*kind, name.as_str()));
            }
        }
        let mut owners = owners.into_iter();
        if let (Some((kind, name)), None) = (owners.next(), owners.next())
            && !diagnostic.message.contains(&format!("{name:?}"))
        {
            diagnostic.message = format!("{kind} {name:?}: {}", diagnostic.message);
        }
    }
}
