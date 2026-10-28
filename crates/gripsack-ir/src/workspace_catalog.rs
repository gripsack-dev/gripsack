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
}
