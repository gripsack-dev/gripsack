//! E114 — unknown placeholder in a fetch/install/verify string. The
//! placeholder set is the contract (0016 §D1): `{version}` and the
//! platform facts (`{system}`, `{target}`, `{arch}`, `{arch.go}`,
//! `{os}`). A typo otherwise sails through check and dies as a 404 at
//! fetch — the opposite of errors-that-point-at-your-code.

use crate::diagnostic::{Diagnostic, codes};
use crate::model::{Ir, Verify};
use crate::span::Span;
use crate::step::StepAction;

use crate::placeholders::SUPPORTED;

/// `{...}` runs in a string, as raw names.
fn placeholders(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        match rest.find('}') {
            Some(close) => {
                out.push(&rest[..close]);
                rest = &rest[close + 1..];
            }
            None => break,
        }
    }
    out
}

/// Nearest known placeholder by edit distance, when plausible.
/// (Same ratio rule as griplint's did-you-mean.)
fn suggest(typo: &str) -> Option<&'static str> {
    SUPPORTED
        .iter()
        .map(|c| (*c, similarity(typo, c)))
        .filter(|(_, r)| *r >= 0.6)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(c, _)| c)
        .filter(|c| *c != typo)
}

/// bigram similarity (Sørensen–Dice), the same shape as difflib's ratio.
fn similarity(a: &str, b: &str) -> f64 {
    let bigrams = |s: &str| -> Vec<(char, char)> {
        let chars: Vec<char> = s.chars().collect();
        chars.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let (a, b) = (bigrams(a), bigrams(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.iter().filter(|g| b.contains(g)).count();
    2.0 * shared as f64 / (a.len() + b.len()) as f64
}

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        // every string slot that can carry a placeholder
        let mut strings: Vec<(&str, &Option<Span>)> = Vec::new();
        let mut fetch_specs: Vec<&crate::model::FetchSpec> =
            module.fetch.as_ref().into_iter().collect();
        if let Some(steps) = &module.steps {
            for step in steps {
                if let StepAction::Fetch { fetch } = &step.action {
                    fetch_specs.push(fetch);
                }
            }
        }
        // fetch/verify strings carry no per-node span — the module's
        // span is the provenance (0004); entries have their own
        let module_span = &module.span;
        for spec in fetch_specs {
            match spec {
                crate::model::FetchSpec::GithubRelease { asset, .. } => {
                    strings.push((asset, module_span))
                }
                crate::model::FetchSpec::Tarball { url, .. } => strings.push((url, module_span)),
                crate::model::FetchSpec::Git { .. } => {}
                crate::model::FetchSpec::File { path } => strings.push((path, module_span)),
                crate::model::FetchSpec::Plugin { args, .. } => strings.extend(
                    args.as_object()
                        .into_iter()
                        .flat_map(|m| m.values())
                        .filter_map(|v| v.as_str())
                        .map(|s| (s, module_span)),
                ),
                crate::model::FetchSpec::Brew { .. } | crate::model::FetchSpec::Pixi { .. } => {}
            }
            let unsupported = match spec {
                crate::model::FetchSpec::Tarball { url, .. } => Some(url.as_str()),
                crate::model::FetchSpec::File { path } => Some(path.as_str()),
                crate::model::FetchSpec::Plugin { args, .. } => args
                    .as_object()
                    .into_iter()
                    .flat_map(|object| object.values())
                    .filter_map(|value| value.as_str())
                    .find(|value| value.contains("{version.bare}")),
                _ => None,
            };
            if unsupported.is_some_and(|pattern| pattern.contains("{version.bare}")) {
                diagnostics.push(Diagnostic::error(codes::UNKNOWN_PLACEHOLDER,
                    "{version.bare} needs a resolved version; use it in a GitHub asset or payload path")
                    .with_label(module_span.clone(), format!("module {name:?} here")));
            }
        }
        for entry in module.install.iter().chain(module.config.iter()) {
            strings.push((&entry.from, &entry.span));
        }
        if let Some(steps) = &module.steps {
            for step in steps {
                match &step.action {
                    StepAction::Install { entries } | StepAction::ConfigDeploy { entries } => {
                        strings.extend(
                            entries
                                .iter()
                                .map(|entry| (entry.from.as_str(), &entry.span)),
                        )
                    }
                    _ => {}
                }
            }
        }
        let mut verifies: Vec<&Verify> = module.verify.as_ref().into_iter().collect();
        if let Some(steps) = &module.steps {
            verifies.extend(steps.iter().flat_map(|step| {
                let action = match &step.action {
                    StepAction::Verify { verify } => Some(verify),
                    _ => None,
                };
                action.into_iter().chain(step.verify.iter())
            }));
        }
        for verify in verifies {
            match verify {
                Verify::BinaryRuns { path, .. } | Verify::FileExists { path } => {
                    strings.push((path, module_span))
                }
                _ => {}
            }
        }
        for (text, span) in strings {
            for placeholder in placeholders(text) {
                if SUPPORTED.contains(&placeholder) {
                    continue;
                }
                let mut d = Diagnostic::error(
                    codes::UNKNOWN_PLACEHOLDER,
                    format!("unknown placeholder '{{{placeholder}}}' in {text:?}"),
                );
                d = d.with_label(span.clone(), format!("module \"{name}\" here"));
                if let Some(s) = suggest(placeholder) {
                    d = d.with_help(format!("did you mean '{{{s}}}'?"));
                }
                diagnostics.push(d);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ir_with_asset(asset: &str) -> String {
        format!(
            r#"{{"ir_version": 3, "host": {{"os": "linux", "arch": "x86_64", "tags": [], "libc": "glibc-2.36"}}, "modules": {{"demo": {{
            "fetch": {{"kind": "github_release", "repo": "o/r", "asset": {asset:?}}},
            "span": {{"file": "modules/demo.ts", "line": 3, "col": 1}}}}}}}}"#
        )
    }

    #[test]
    fn a_typo_is_an_error_with_a_span_and_a_suggestion() {
        let ir = crate::parse(&ir_with_asset("starship-{sytem}.tar.gz")).unwrap();
        let diagnostics = crate::sema::run(&ir);
        let d = diagnostics
            .iter()
            .find(|d| d.code == codes::UNKNOWN_PLACEHOLDER)
            .expect("typo must be flagged");
        assert!(d.message.contains("{sytem}"));
        assert_eq!(d.help.as_deref(), Some("did you mean '{system}'?"), "{d:?}");
        let label = d.labels.first().expect("a span is the point");
        assert_eq!(label.span.as_ref().unwrap().line, 3); // the module's span
    }

    #[test]
    fn the_known_set_passes_clean() {
        let ir = crate::parse(&ir_with_asset(
            "pkg-{version}-{system}-{target}-{arch}-{arch.go}-{os}.tar.gz",
        ))
        .unwrap();
        let diagnostics = crate::sema::run(&ir);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.code != codes::UNKNOWN_PLACEHOLDER),
            "{diagnostics:?}"
        );
    }
}
