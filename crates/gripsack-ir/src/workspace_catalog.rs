//! Common read-only catalog metadata. This view never converts historical
//! execution or file semantics into the current wire representation.
use crate::{Ir, Span};

#[derive(Debug, Clone, Copy)]
pub struct CatalogOutput<'a> {
    pub name: &'a str,
    pub kind: &'static str,
    pub span: &'a Span,
}

impl Ir {
    pub fn workspace_outputs(&self) -> impl Iterator<Item = CatalogOutput<'_>> {
        self.workspace
            .iter()
            .flat_map(|workspace| {
                workspace.outputs.iter().map(|output| CatalogOutput {
                    name: output.name(),
                    kind: output.kind(),
                    span: output.span(),
                })
            })
            .chain(self.workspace_v6.iter().flat_map(|workspace| {
                workspace.outputs.iter().map(|output| CatalogOutput {
                    name: output.name(),
                    kind: output.kind(),
                    span: output.span(),
                })
            }))
            .chain(self.workspace_v4.iter().flat_map(|workspace| {
                workspace.outputs.iter().map(|output| CatalogOutput {
                    name: output.name(),
                    kind: output.kind(),
                    span: output.span(),
                })
            }))
    }
}
