//! Shared lexical and cross-profile ownership checks.

pub(super) mod destinations;
pub(super) mod span_value;

#[cfg(test)]
pub(crate) mod testutil {
    pub fn doc(outputs: &str) -> String {
        format!(
            r#"{{"ir_version": {}, "host": {{"os": "linux", "arch": "x86_64"}}, "workspace": {{"span": {{"file": "grip.ts", "line": 1}}, "outputs": [{outputs}]}}}}"#,
            crate::IR_VERSION,
        )
    }
}
