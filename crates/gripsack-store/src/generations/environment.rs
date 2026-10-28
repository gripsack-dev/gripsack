//! New workspace contributions are data, not shell expressions. Historical
//! untagged records retain their documented expansion semantics byte-for-byte;
//! old readers reject the new `kind` tag rather than reinterpreting literals.
use crate::EnvironmentOperation;
use serde::{Deserialize, Serialize};

/// A historical shell expression recorded in a generation manifest.
/// Its untagged wire and expansion semantics predate workspace authoring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedEnvironment {
    pub name: String,
    #[serde(default)]
    pub op: EnvironmentOperation,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EnvironmentContribution {
    Structured(StructuredEnvironment),
    LegacyExpression(RetainedEnvironment),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredEnvironment {
    Literal {
        name: String,
        op: EnvironmentOperation,
        value: String,
    },
    /// An explicit relative path in this deployment's immutable store object.
    /// Literal values never perform legacy `{store}` or shell substitution.
    StorePath {
        name: String,
        op: EnvironmentOperation,
        path: String,
    },
}
impl From<RetainedEnvironment> for EnvironmentContribution {
    fn from(value: RetainedEnvironment) -> Self {
        Self::LegacyExpression(value)
    }
}
impl EnvironmentContribution {
    pub fn name(&self) -> &str {
        match self {
            Self::LegacyExpression(value) => &value.name,
            Self::Structured(
                StructuredEnvironment::Literal { name, .. }
                | StructuredEnvironment::StorePath { name, .. },
            ) => name,
        }
    }
    pub fn operation(&self) -> EnvironmentOperation {
        match self {
            Self::LegacyExpression(value) => value.op,
            Self::Structured(
                StructuredEnvironment::Literal { op, .. }
                | StructuredEnvironment::StorePath { op, .. },
            ) => *op,
        }
    }
    pub(crate) fn valid_structured(&self) -> bool {
        let Self::Structured(value) = self else {
            return true;
        };
        let name = self.name();
        if !name
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return false;
        }
        match value {
            StructuredEnvironment::Literal { value, .. } => !value.contains('\0'),
            StructuredEnvironment::StorePath { path, .. } => {
                !path.is_empty()
                    && !path.contains('\0')
                    && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn historical_expression_wire_and_default_operation_are_retained() {
        let contribution: EnvironmentContribution = serde_json::from_value(json!({
            "name": "PATH", "value": "{store}/bin:$PATH"
        }))
        .unwrap();
        assert_eq!(
            contribution,
            EnvironmentContribution::LegacyExpression(RetainedEnvironment {
                name: "PATH".into(),
                op: EnvironmentOperation::Set,
                value: "{store}/bin:$PATH".into(),
            })
        );
        assert_eq!(
            serde_json::to_value(contribution).unwrap(),
            json!({"name": "PATH", "op": "set", "value": "{store}/bin:$PATH"})
        );
        for op in ["set", "prepend", "append"] {
            let wire = json!({"name": "PATH", "op": op, "value": "{store}/bin"});
            let contribution: EnvironmentContribution =
                serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(serde_json::to_value(contribution).unwrap(), wire);
        }
    }

    #[test]
    fn historical_expressions_still_reject_unknown_fields() {
        assert!(
            serde_json::from_value::<EnvironmentContribution>(json!({
                "name": "PATH", "value": "{store}/bin", "unknown": true
            }))
            .is_err()
        );
    }
}
