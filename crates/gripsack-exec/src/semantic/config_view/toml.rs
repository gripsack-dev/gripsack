//! TOML 0.9's admitted document tree supplies decoded keys and real source
//! spans. Numeric conversion follows toml::Value (i64/f64); dates stay typed.
//! The cheap lexical complexity guard runs before the parser can construct an
//! arbitrarily deep dotted-key tree. It is a resource gate, not a TOML parser.

use std::collections::BTreeMap;

use super::{
    FallbackReason, MAX_DEPTH, MAX_NODES, MAX_PATH_BYTES, Node, SourceSpan, Value, field_path,
};

pub(super) fn parse(text: &str) -> Result<Node, FallbackReason> {
    guard_complexity(text.as_bytes())?;
    let parsed = ::toml::de::DeTable::parse(text).map_err(|_| FallbackReason::InvalidSyntax)?;
    let mut nodes = 0;
    table(
        parsed.into_inner(),
        SourceSpan {
            start: 0,
            end: text.len(),
        },
        0,
        1,
        &mut nodes,
    )
}

fn table(
    fields: ::toml::de::DeTable<'_>,
    span: SourceSpan,
    depth: usize,
    path_bytes: usize,
    nodes: &mut usize,
) -> Result<Node, FallbackReason> {
    let mut result = BTreeMap::new();
    for (key, child) in fields {
        let key = key.into_inner().into_owned();
        if key.len() > MAX_PATH_BYTES {
            return Err(FallbackReason::ResourceLimit);
        }
        let child_path_bytes = path_bytes + field_path("", &key).len();
        let span = child.span().into();
        let child = convert(child.into_inner(), span, depth + 1, child_path_bytes, nodes)?;
        result.insert(key, child);
    }
    Ok(Node {
        span,
        value: Value::Object(result),
    })
}

fn convert(
    value: ::toml::de::DeValue<'_>,
    span: SourceSpan,
    depth: usize,
    path_bytes: usize,
    nodes: &mut usize,
) -> Result<Node, FallbackReason> {
    *nodes += 1;
    if depth > MAX_DEPTH || *nodes > MAX_NODES || path_bytes > MAX_PATH_BYTES {
        return Err(FallbackReason::ResourceLimit);
    }
    let value = match value {
        ::toml::de::DeValue::String(value) => Value::String(value.into_owned()),
        ::toml::de::DeValue::Integer(value) => Value::Integer(
            i64::from_str_radix(value.as_str(), value.radix())
                .map_err(|_| FallbackReason::InvalidSyntax)?,
        ),
        ::toml::de::DeValue::Float(value) => {
            let parsed: f64 = value
                .as_str()
                .parse()
                .map_err(|_| FallbackReason::InvalidSyntax)?;
            if parsed.is_infinite() && !value.as_str().contains("inf") {
                return Err(FallbackReason::InvalidSyntax);
            }
            Value::Float(parsed)
        }
        ::toml::de::DeValue::Boolean(value) => Value::Boolean(value),
        ::toml::de::DeValue::Datetime(value) => Value::Datetime(value.to_string()),
        ::toml::de::DeValue::Array(values) => {
            let mut result = Vec::with_capacity(values.len());
            for (index, child) in values.into_iter().enumerate() {
                let span = child.span().into();
                result.push(convert(
                    child.into_inner(),
                    span,
                    depth + 1,
                    path_bytes + format!("[{index}]").len(),
                    nodes,
                )?);
            }
            Value::Array(result)
        }
        ::toml::de::DeValue::Table(fields) => return table(fields, span, depth, path_bytes, nodes),
    };
    Ok(Node { span, value })
}

fn guard_complexity(bytes: &[u8]) -> Result<(), FallbackReason> {
    let mut offset = 0;
    let mut nesting = 0_usize;
    let mut dots = 0_usize;
    while let Some(&byte) = bytes.get(offset) {
        match byte {
            b'#' => {
                while bytes.get(offset).is_some_and(|byte| *byte != b'\n') {
                    offset += 1;
                }
                continue;
            }
            b'\'' | b'"' => {
                let quote = byte;
                let triple = bytes.get(offset..offset + 3) == Some(&[quote; 3]);
                offset += if triple { 3 } else { 1 };
                while let Some(&byte) = bytes.get(offset) {
                    if quote == b'"' && byte == b'\\' {
                        offset += 2;
                    } else if byte == quote {
                        let run = bytes[offset..]
                            .iter()
                            .take_while(|&&byte| byte == quote)
                            .count();
                        if !triple || run >= 3 {
                            offset += if triple { run } else { 1 };
                            break;
                        }
                        offset += run;
                    } else {
                        offset += 1;
                    }
                }
                continue;
            }
            b'[' | b'{' => nesting += 1,
            b']' | b'}' => nesting = nesting.saturating_sub(1),
            b'.' => dots += 1,
            b'\n' if nesting == 0 => dots = 0,
            _ => {}
        }
        // Conservative per-top-level-statement syntax budget. Dots in scalar
        // floats count too; quoted strings/comments do not. Exhaustion is an
        // explicit resource fallback, never an invalid/equal configuration.
        if nesting + dots > MAX_DEPTH {
            return Err(FallbackReason::ResourceLimit);
        }
        offset += 1;
    }
    Ok(())
}
