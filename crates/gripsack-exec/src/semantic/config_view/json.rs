//! Strict JSON container admission with serde_json scalar semantics. Keeping
//! offsets while admitting each key prevents duplicate-key collapse and avoids
//! searching raw source text for a decoded field after parsing.

use std::collections::BTreeMap;

use super::{
    FallbackReason, MAX_DEPTH, MAX_NODES, MAX_PATH_BYTES, Node, SourceSpan, Value, field_path,
};

pub(super) fn parse(text: &str) -> Result<Node, FallbackReason> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        offset: 0,
        nodes: 0,
    };
    let root = parser.value(0, 1)?;
    parser.whitespace();
    if parser.offset != parser.bytes.len() {
        return Err(FallbackReason::InvalidSyntax);
    }
    Ok(root)
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
    nodes: usize,
}

impl Parser<'_> {
    fn whitespace(&mut self) {
        while matches!(
            self.bytes.get(self.offset),
            Some(b' ' | b'\t' | b'\r' | b'\n')
        ) {
            self.offset += 1;
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.bytes.get(self.offset) == Some(&byte) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn string(&mut self) -> Result<String, FallbackReason> {
        self.whitespace();
        let start = self.offset;
        if !self.take(b'"') {
            return Err(FallbackReason::InvalidSyntax);
        }
        while let Some(byte) = self.bytes.get(self.offset) {
            self.offset += 1;
            match byte {
                b'\\' => self.offset += 1,
                b'"' => {
                    return serde_json::from_slice(&self.bytes[start..self.offset])
                        .map_err(|_| FallbackReason::InvalidSyntax);
                }
                _ => {}
            }
        }
        Err(FallbackReason::InvalidSyntax)
    }

    fn value(&mut self, depth: usize, path_bytes: usize) -> Result<Node, FallbackReason> {
        self.nodes += 1;
        if depth > MAX_DEPTH || self.nodes > MAX_NODES || path_bytes > MAX_PATH_BYTES {
            return Err(FallbackReason::ResourceLimit);
        }
        self.whitespace();
        let start = self.offset;
        let value =
            match self.bytes.get(self.offset) {
                Some(b'{') => {
                    self.offset += 1;
                    let mut fields = BTreeMap::new();
                    if !self.take(b'}') {
                        loop {
                            let key = self.string()?;
                            if key.len() > MAX_PATH_BYTES {
                                return Err(FallbackReason::ResourceLimit);
                            }
                            if fields.contains_key(&key) {
                                return Err(FallbackReason::DuplicateKey);
                            }
                            if !self.take(b':') {
                                return Err(FallbackReason::InvalidSyntax);
                            }
                            let value =
                                self.value(depth + 1, path_bytes + field_path("", &key).len())?;
                            fields.insert(key, value);
                            if self.take(b'}') {
                                break;
                            }
                            if !self.take(b',') {
                                return Err(FallbackReason::InvalidSyntax);
                            }
                        }
                    }
                    Value::Object(fields)
                }
                Some(b'[') => {
                    self.offset += 1;
                    let mut values = Vec::new();
                    if !self.take(b']') {
                        loop {
                            values.push(self.value(
                                depth + 1,
                                path_bytes + format!("[{}]", values.len()).len(),
                            )?);
                            if self.take(b']') {
                                break;
                            }
                            if !self.take(b',') {
                                return Err(FallbackReason::InvalidSyntax);
                            }
                        }
                    }
                    Value::Array(values)
                }
                Some(b'"') => Value::String(self.string()?),
                Some(_) => {
                    while self.bytes.get(self.offset).is_some_and(|byte| {
                        !matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b',' | b'}' | b']')
                    }) {
                        self.offset += 1;
                    }
                    match serde_json::from_slice(&self.bytes[start..self.offset]) {
                        Ok(serde_json::Value::Null) => Value::Null,
                        Ok(serde_json::Value::Bool(value)) => Value::Boolean(value),
                        Ok(serde_json::Value::Number(value)) => Value::JsonNumber(value),
                        _ => return Err(FallbackReason::InvalidSyntax),
                    }
                }
                None => return Err(FallbackReason::InvalidSyntax),
            };
        Ok(Node {
            span: SourceSpan {
                start,
                end: self.offset,
            },
            value,
        })
    }
}
