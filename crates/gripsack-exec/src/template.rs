//! Whole-file template rendering (0001 §3.7); managed blocks live separately.

use std::collections::BTreeMap;

use crate::ctx::ExecError;

/// Render `{{ name }}` substitutions. `{{{{` is a literal `{{` — the
/// chezmoi pitfall: payloads that themselves carry template syntax
/// (helm values, jinja configs) must be expressible. Anything else
/// (conditionals, filters) is out of scope by design: per-host logic
/// lives in the frontend, which computes `vars` at eval time.
pub fn render_template(
    bytes: &[u8],
    vars: &BTreeMap<String, String>,
    from: &str,
) -> Result<Vec<u8>, ExecError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ExecError::Step {
        module: from.to_string(),
        step: "render".into(),
        detail: format!("template payload {from:?} is not UTF-8"),
    })?;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(stripped) = rest.strip_prefix("{{{{") {
            out.push_str("{{");
            rest = stripped;
            continue;
        }
        let body = &rest[2..];
        let Some(end) = body.find("}}") else {
            return Err(ExecError::Step {
                module: from.to_string(),
                step: "render".into(),
                detail: format!("unbalanced `{{{{` in template {from:?}"),
            });
        };
        let name = body[..end].trim();
        match vars.get(name) {
            Some(v) => out.push_str(v),
            None => {
                return Err(ExecError::Step {
                    module: from.to_string(),
                    step: "render".into(),
                    detail: format!(
                        "template {from:?} references undefined variable {name:?} (have: {})",
                        vars.keys().cloned().collect::<Vec<_>>().join(", ")
                    ),
                });
            }
        }
        rest = &body[end + 2..];
    }
    out.push_str(rest);
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn template_substitutes_and_escapes() {
        let out = render_template(
            b"email = {{ email }}\nliteral = {{{{ not-a-var }}\n",
            &vars(&[("email", "a@b.c")]),
            "id",
        )
        .unwrap();
        assert_eq!(out, b"email = a@b.c\nliteral = {{ not-a-var }}\n");
    }

    #[test]
    fn template_undefined_variable_is_a_loud_error() {
        let err = render_template(b"{{ typo }}", &vars(&[("email", "a@b.c")]), "id").unwrap_err();
        let ExecError::Step { detail, .. } = &err else {
            panic!("expected ExecError::Step, got {err:?}");
        };
        assert!(detail.contains("undefined variable \"typo\""));
        assert!(detail.contains("email"));
    }

    #[test]
    fn template_unbalanced_braces_error() {
        assert!(render_template(b"x {{ y", &vars(&[]), "id").is_err());
    }
}
