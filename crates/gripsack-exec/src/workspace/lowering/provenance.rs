//! Requester locations stay outside semantic/cache identities. A checked LLB
//! witness, not a bridge-supplied source path, relates events to these locations.
use crate::ProgressCallback;
use gripsack_buildkit::{plan::NodeIndex, transport::TransportError};
use gripsack_ir::{Diagnostic, Span, codes};
use std::collections::BTreeSet;
mod bash_line;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq)]
pub(in crate::workspace) struct Origin<'a> {
    pub output: &'a str,
    pub span: &'a Span,
    pub line_map: &'a [u32],
}
struct NodeOrigins<'a> {
    primary: Origin<'a>,
    requesters: Vec<Origin<'a>>,
    generated_line: Option<usize>,
    line_parser: bash_line::BashLine,
}
#[derive(Default)]
pub(in crate::workspace) struct SourceMap<'a> {
    nodes: Vec<NodeOrigins<'a>>,
}
impl<'a> SourceMap<'a> {
    pub fn push(&mut self, origin: Origin<'a>) {
        self.nodes.push(NodeOrigins {
            primary: origin,
            requesters: Vec::new(),
            generated_line: None,
            line_parser: Default::default(),
        });
    }
    pub fn note(&mut self, node: NodeIndex, origin: Origin<'a>) {
        let sources = &mut self.nodes[node.index()];
        if sources.primary != origin && !sources.requesters.contains(&origin) {
            sources.requesters.push(origin);
        }
    }
    pub fn log(
        &mut self,
        nodes: &[NodeIndex],
        vertex: &str,
        chunk: &[u8],
        truncated: bool,
        progress: Option<&ProgressCallback>,
    ) {
        let generated = nodes
            .first()
            .and_then(|node| self.nodes.get_mut(node.index()))
            .and_then(|sources| sources.line_parser.observe(chunk));
        for node in nodes {
            if let Some(sources) = self.nodes.get_mut(node.index()) {
                if generated.is_some() {
                    sources.generated_line = generated;
                }
            }
        }
        let Some(progress) = progress else {
            return;
        };
        let text = String::from_utf8_lossy(chunk);
        let text = gripsack_process::terminal::tame(text.into_owned());
        let event = if truncated {
            format!("{text} [truncated]")
        } else {
            text
        };
        let mut seen = BTreeSet::new();
        for node in nodes {
            if let Some(sources) = self.nodes.get(node.index()) {
                for origin in
                    std::iter::once(sources.primary).chain(sources.requesters.iter().copied())
                {
                    let line = sources
                        .generated_line
                        .and_then(|line| origin.line_map.get(line))
                        .copied()
                        .unwrap_or(origin.span.line);
                    if seen.insert((
                        origin.output,
                        origin.span.file.as_str(),
                        line,
                        origin.span.col,
                    )) {
                        progress(
                            &format!("{} ({}:{line})", origin.output, origin.span.file),
                            &event,
                        );
                    }
                }
            }
        }
        if seen.is_empty() {
            progress(vertex, &event);
        }
    }
    pub fn failure(&self, error: &TransportError) -> Diagnostic {
        let mut diagnostic = Diagnostic::error(codes::EXEC_STEP, error.to_string());
        let TransportError::Rejected { nodes, .. } = error else {
            return diagnostic;
        };
        let mut seen = BTreeSet::new();
        for node in nodes {
            if let Some(sources) = self.nodes.get(node.index()) {
                for origin in
                    std::iter::once(sources.primary).chain(sources.requesters.iter().copied())
                {
                    let mut span = origin.span.clone();
                    if let Some(line) = sources
                        .generated_line
                        .and_then(|line| origin.line_map.get(line))
                    {
                        span.line = *line;
                        // The tag's column does not describe a different body line.
                        span.col = None;
                    }
                    if seen.insert((origin.output, span.file.clone(), span.line, span.col)) {
                        diagnostic = diagnostic.with_label(
                            Some(span),
                            format!("{} requested this failed operation", origin.output),
                        );
                    }
                }
            }
        }
        diagnostic
    }
}
