//! New workspace contributions are data, not shell expressions. Historical
//! untagged records retain their documented expansion semantics byte-for-byte;
//! old readers reject the new `kind` tag rather than reinterpreting literals.
use gripsack_ir::{EnvOp, EnvVar};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EnvironmentContribution {
    Structured(StructuredEnvironment),
    LegacyExpression(EnvVar),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredEnvironment {
    Literal {
        name: String,
        op: EnvOp,
        value: String,
    },
    /// An explicit relative path in this module's immutable store object.
    /// Literal values never perform legacy `{store}` or shell substitution.
    StorePath {
        name: String,
        op: EnvOp,
        path: String,
    },
}
impl From<EnvVar> for EnvironmentContribution {
    fn from(value: EnvVar) -> Self {
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
    pub fn operation(&self) -> EnvOp {
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
