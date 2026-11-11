//! The untrusted-text boundary shared by prompts and native output. Intentional
//! renderer colours never pass through this adapter. This is control escaping,
//! not a claim to discover secrets embedded in arbitrary prose.
use std::{
    fmt,
    io::{self, Write},
};

fn control(character: char) -> bool {
    character.is_control()
        || matches!(character, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

struct SafeCharacter(char);
impl fmt::Display for SafeCharacter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if control(self.0) {
            for character in self.0.escape_default() {
                write!(formatter, "{character}")?;
            }
            Ok(())
        } else {
            let mut encoded = [0; 4];
            formatter.write_str(self.0.encode_utf8(&mut encoded))
        }
    }
}

struct SafeText<'a> {
    value: &'a str,
    quoted: bool,
}

impl<'a> SafeText<'a> {
    fn new(value: &'a str) -> Self {
        Self {
            value,
            quoted: value.chars().any(control),
        }
    }
}

impl fmt::Display for SafeText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.quoted {
            return formatter.write_str(self.value);
        }
        formatter.write_str("\"")?;
        for character in self.value.chars() {
            if matches!(character, '\\' | '"') {
                formatter.write_str("\\")?;
            }
            write!(formatter, "{}", SafeCharacter(character))?;
        }
        formatter.write_str("\"")
    }
}

/// Escape borrowed untrusted text directly into the renderer without copying it.
pub fn display(value: &str) -> impl fmt::Display + '_ {
    SafeText::new(value)
}

/// Preserve ordinary owned values without another allocation. A value that can
/// forge prompt lines is quoted/escaped, matching the original trust prompt.
pub fn tame(value: String) -> String {
    let text = SafeText::new(&value);
    if text.quoted { text.to_string() } else { value }
}

/// Render a bounded captured byte stream. LF retains normal output lines;
/// terminal controls and invalid UTF-8 bytes cannot reach the terminal raw.
pub fn write_output(writer: &mut impl Write, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let (valid, invalid) = match std::str::from_utf8(bytes) {
            Ok(text) => (text, 0),
            Err(error) => {
                let length = error.valid_up_to();
                // SAFETY: Utf8Error guarantees that this exact prefix is valid.
                let valid = unsafe { std::str::from_utf8_unchecked(&bytes[..length]) };
                (valid, error.error_len().unwrap_or(bytes.len() - length))
            }
        };
        let mut start = 0;
        for (offset, character) in valid.char_indices() {
            if control(character) && character != '\n' {
                writer.write_all(&valid.as_bytes()[start..offset])?;
                write!(writer, "{}", SafeCharacter(character))?;
                start = offset + character.len_utf8();
            }
        }
        writer.write_all(&valid.as_bytes()[start..])?;
        bytes = &bytes[valid.len()..];
        for byte in &bytes[..invalid] {
            write!(writer, "\\x{byte:02x}")?;
        }
        bytes = &bytes[invalid..];
    }
    Ok(())
}
