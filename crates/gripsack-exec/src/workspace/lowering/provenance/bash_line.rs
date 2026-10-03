//! Read Bash's generated-line prefix across arbitrary bounded log frames.
//! No line text is retained and integer overflow cannot invent a source line.
const PREFIX: &[u8] = b"gripsack-bash: line ";

#[derive(Default)]
pub(super) struct BashLine {
    prefix: usize,
    number: Option<usize>,
    ignoring: bool,
}
impl BashLine {
    pub fn observe(&mut self, bytes: &[u8]) -> Option<usize> {
        let mut last = None;
        for &byte in bytes {
            if byte == b'\n' {
                *self = Self::default();
                continue;
            }
            if self.ignoring {
                continue;
            }
            if self.prefix < PREFIX.len() {
                if byte == PREFIX[self.prefix] {
                    self.prefix += 1;
                } else {
                    self.ignoring = true;
                }
            } else if byte.is_ascii_digit() {
                match self
                    .number
                    .unwrap_or(0)
                    .checked_mul(10)
                    .and_then(|number| number.checked_add(usize::from(byte - b'0')))
                {
                    Some(number) => self.number = Some(number),
                    None => self.ignoring = true,
                }
            } else {
                if byte == b':' {
                    last = self.number.and_then(|number| number.checked_sub(1));
                }
                self.ignoring = true;
            }
        }
        last
    }
}
