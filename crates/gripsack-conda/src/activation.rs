//! Hermetic evaluation of conda `etc/conda/activate.d/*.sh` scripts (#405).
//!
//! Activation scripts are sourced by conda into the user's shell. Gripsack
//! never executes them: it interprets a small, deterministic POSIX-shell subset
//! and records only the exported variables the scripts leave behind. Nothing
//! reads the host environment (every ambient variable is unset), the
//! filesystem, or a subprocess, so the result is a pure function of the
//! script bytes and the prefix path.
//!
//! Supported: comments, assignments, `export`, `unset`, `:`/`true`/`false`,
//! `test`/`[` with `-n`, `-z`, `=`, `==`, `!=` and `!`, `if`/`elif`/`else`,
//! `while`, `for`, `&&`/`||`, shell functions with positional arguments,
//! `return`, quoting, and `$name`/`${name}`/`${name:-d}`/`${name-d}`/
//! `${name:+a}`/`${name+a}`/`${name#p}`/`${name##p}`/`${name%p}`/`${name%%p}`
//! (literal patterns). Anything else (command substitution, pipelines,
//! redirections, other commands, globbing) is refused with its line number.

use std::collections::{BTreeMap, BTreeSet};

/// Maximum script bytes interpreted, and maximum interpreted commands.
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;
const MAX_STEPS: usize = 200_000;
const MAX_VALUE_BYTES: usize = 64 * 1024;
const MAX_DEPTH: usize = 32;

/// Variables whose value gripsack owns; activation may not set them.
fn reserved(name: &str) -> bool {
    name == "PATH" || name == "IFS" || name.starts_with("LD_") || name == "CONDA_PREFIX"
}

/// Bookkeeping variables that conda scripts use to restore state on
/// deactivation; gripsack has no deactivation, so they carry no meaning.
fn bookkeeping(name: &str) -> bool {
    name.contains("CONDA_BACKUP")
}

/// Evaluate scripts in order (conda sources them sorted by name, in one
/// shell) and return the exported variables, ordered by name.
pub fn evaluate<'a>(
    scripts: impl IntoIterator<Item = (&'a str, &'a str)>,
    prefix: &str,
) -> Result<BTreeMap<String, String>, String> {
    let mut shell = Shell::new(prefix);
    for (name, text) in scripts {
        if text.len() > MAX_SCRIPT_BYTES {
            return Err(format!("{name}: script exceeds {MAX_SCRIPT_BYTES} bytes"));
        }
        let list = Parser::new(text)
            .script()
            .map_err(|error| format!("{name}: {error}"))?;
        match shell.run_list(&list, 0) {
            Ok(_) | Err(Ctl::Return(_)) => {}
            Err(Ctl::Error(error)) => return Err(format!("{name}: {error}")),
        }
    }
    let mut result = BTreeMap::new();
    for name in &shell.exported {
        let Some(value) = shell.vars.get(name) else {
            continue;
        };
        if name == "CONDA_PREFIX" || bookkeeping(name) {
            continue;
        }
        result.insert(name.clone(), value.clone());
    }
    Ok(result)
}

// ---------------------------------------------------------------- syntax

#[derive(Debug, Clone)]
enum Op {
    Default { colon: bool },
    Alternate { colon: bool },
    PrefixShort,
    PrefixLong,
    SuffixShort,
    SuffixLong,
}

#[derive(Debug, Clone)]
enum Kind {
    Lit(String),
    Param {
        name: String,
        op: Option<(Op, Word)>,
    },
}

#[derive(Debug, Clone)]
struct Seg {
    quoted: bool,
    kind: Kind,
}
type Word = Vec<Seg>;

#[derive(Debug, Clone)]
enum Cmd {
    Assign(Vec<(String, Word)>),
    Simple(Vec<Word>, usize),
    If {
        arms: Vec<(List, List)>,
        otherwise: Option<List>,
    },
    While(List, List),
    For(String, Vec<Word>, List),
    Function(String, List),
}

/// A pipeline-less and/or chain: first command then (is_and, negated, command).
#[derive(Debug, Clone)]
struct AndOr {
    first: (bool, Cmd),
    rest: Vec<(bool, bool, Cmd)>,
}
type List = Vec<AndOr>;

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

type PResult<T> = Result<T, String>;

const KEYWORDS: [&str; 9] = ["then", "elif", "else", "fi", "do", "done", "}", "{", "in"];

impl Parser {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
        }
    }
    fn line(&self) -> usize {
        1 + self.chars[..self.pos.min(self.chars.len())]
            .iter()
            .filter(|c| **c == '\n')
            .count()
    }
    fn err<T>(&self, detail: impl std::fmt::Display) -> PResult<T> {
        Err(format!("line {}: {detail}", self.line()))
    }
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }
    fn script(mut self) -> PResult<List> {
        let list = self.list(&[])?;
        if self.pos < self.chars.len() {
            return self.err("unexpected token");
        }
        Ok(list)
    }
    fn blanks(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                self.pos += 1;
            } else if c == '\\' && self.at(1) == Some('\n') {
                self.pos += 2;
            } else if c == '#' {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }
    fn separators(&mut self) {
        loop {
            self.blanks();
            match self.peek() {
                Some('\n' | ';') => self.pos += 1,
                _ => break,
            }
        }
    }
    /// The unquoted keyword at the cursor, if the next word is exactly one.
    fn keyword(&mut self) -> Option<&'static str> {
        self.blanks();
        let start = self.pos;
        let mut end = start;
        while self
            .chars
            .get(end)
            .is_some_and(|c| !c.is_whitespace() && !";&|<>()#'\"$\\`".contains(*c))
        {
            end += 1;
        }
        let word: String = self.chars[start..end].iter().collect();
        let after = self.chars.get(end).copied();
        if !matches!(after, None | Some(' ' | '\t' | '\n' | ';' | '#')) {
            return None;
        }
        KEYWORDS.iter().copied().find(|keyword| *keyword == word)
    }
    fn expect(&mut self, keyword: &str) -> PResult<()> {
        self.separators();
        if self.keyword() == Some(keyword_static(keyword)) {
            self.pos += keyword.len();
            Ok(())
        } else {
            self.err(format!("expected `{keyword}`"))
        }
    }
    fn list(&mut self, stops: &[&str]) -> PResult<List> {
        let mut list = Vec::new();
        loop {
            self.separators();
            if self.pos >= self.chars.len() {
                break;
            }
            if let Some(keyword) = self.keyword()
                && stops.contains(&keyword)
            {
                break;
            }
            list.push(self.and_or()?);
        }
        Ok(list)
    }
    fn and_or(&mut self) -> PResult<AndOr> {
        let first = self.negated_command()?;
        let mut rest = Vec::new();
        loop {
            self.blanks();
            let and = match (self.peek(), self.at(1)) {
                (Some('&'), Some('&')) => true,
                (Some('|'), Some('|')) => false,
                (Some('|'), _) => return self.err("pipelines are not supported"),
                (Some('&'), _) => return self.err("background jobs are not supported"),
                (Some('>' | '<'), _) => return self.err("redirections are not supported"),
                _ => break,
            };
            self.pos += 2;
            // A newline may follow the operator.
            while matches!(self.peek(), Some(' ' | '\t' | '\n')) {
                self.pos += 1;
            }
            let (negated, command) = self.negated_command()?;
            rest.push((and, negated, command));
        }
        Ok(AndOr { first, rest })
    }
    fn negated_command(&mut self) -> PResult<(bool, Cmd)> {
        self.blanks();
        let mut negated = false;
        if self.peek() == Some('!') && matches!(self.at(1), Some(' ' | '\t')) {
            negated = true;
            self.pos += 1;
        }
        Ok((negated, self.command()?))
    }
    fn command(&mut self) -> PResult<Cmd> {
        if self.keyword().is_some() {
            return self.err("unexpected keyword");
        }
        self.blanks();
        let start = self.pos;
        let word_text: String = self.chars[start..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == '_')
            .collect();
        let after = self.chars.get(start + word_text.chars().count()).copied();
        match (word_text.as_str(), after) {
            ("if", Some(' ' | '\t' | '\n')) => {
                self.pos += 2;
                return self.if_command();
            }
            ("while", Some(' ' | '\t' | '\n')) => {
                self.pos += 5;
                let condition = self.list(&["do"])?;
                self.expect("do")?;
                let body = self.list(&["done"])?;
                self.expect("done")?;
                return Ok(Cmd::While(condition, body));
            }
            ("for", Some(' ' | '\t')) => {
                self.pos += 3;
                return self.for_command();
            }
            ("case" | "until" | "select" | "function", Some(' ' | '\t' | '\n')) => {
                return self.err(format!("`{word_text}` is not supported"));
            }
            _ => {}
        }
        // Function definition: name ( ) { ... }
        if !word_text.is_empty() && !word_text.starts_with(|c: char| c.is_ascii_digit()) {
            let mut look = start + word_text.chars().count();
            while matches!(self.chars.get(look), Some(' ' | '\t')) {
                look += 1;
            }
            if self.chars.get(look) == Some(&'(') {
                self.pos = look + 1;
                self.blanks();
                if self.peek() != Some(')') {
                    return self.err("expected `)` in function definition");
                }
                self.pos += 1;
                self.separators();
                if self.keyword() != Some("{") {
                    return self.err("function body must be a `{ ...; }` group");
                }
                self.pos += 1;
                let body = self.list(&["}"])?;
                self.expect("}")?;
                return Ok(Cmd::Function(word_text, body));
            }
        }
        self.simple_command()
    }
    fn if_command(&mut self) -> PResult<Cmd> {
        let mut arms = Vec::new();
        let mut otherwise = None;
        let condition = self.list(&["then"])?;
        self.expect("then")?;
        let body = self.list(&["elif", "else", "fi"])?;
        arms.push((condition, body));
        loop {
            self.separators();
            match self.keyword() {
                Some("elif") => {
                    self.pos += 4;
                    let condition = self.list(&["then"])?;
                    self.expect("then")?;
                    let body = self.list(&["elif", "else", "fi"])?;
                    arms.push((condition, body));
                }
                Some("else") => {
                    self.pos += 4;
                    otherwise = Some(self.list(&["fi"])?);
                }
                Some("fi") => {
                    self.pos += 2;
                    break;
                }
                _ => return self.err("expected `fi`"),
            }
        }
        Ok(Cmd::If { arms, otherwise })
    }
    fn for_command(&mut self) -> PResult<Cmd> {
        self.blanks();
        let name: String = self.chars[self.pos..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == '_')
            .collect();
        if name.is_empty() {
            return self.err("expected a loop variable");
        }
        self.pos += name.chars().count();
        self.blanks();
        if self.keyword() != Some("in") {
            return self.err("`for` requires an `in` word list");
        }
        self.pos += 2;
        let mut words = Vec::new();
        loop {
            self.blanks();
            if matches!(self.peek(), Some(';' | '\n') | None) {
                break;
            }
            words.push(self.word()?);
        }
        self.separators();
        self.expect("do")?;
        let body = self.list(&["done"])?;
        self.expect("done")?;
        Ok(Cmd::For(name, words, body))
    }
    fn simple_command(&mut self) -> PResult<Cmd> {
        let line = self.line();
        let mut assignments = Vec::new();
        let mut words = Vec::new();
        loop {
            self.blanks();
            match self.peek() {
                None | Some('\n' | ';') => break,
                Some('&' | '|') => break,
                Some('>' | '<') => return self.err("redirections are not supported"),
                Some('(' | ')') => return self.err("subshells are not supported"),
                _ => {}
            }
            let word = self.word()?;
            if words.is_empty()
                && let Some((name, value)) = split_assignment(&word)
            {
                assignments.push((name, value));
            } else {
                words.push(word);
            }
        }
        match (assignments.is_empty(), words.is_empty()) {
            (false, true) => Ok(Cmd::Assign(assignments)),
            (true, false) => Ok(Cmd::Simple(words, line)),
            (false, false) => self.err("assignment prefixes on commands are not supported"),
            (true, true) => self.err("empty command"),
        }
    }
    fn word(&mut self) -> PResult<Word> {
        let segments = self.segments(Context::Unquoted)?;
        if segments.is_empty() {
            return self.err("unexpected token");
        }
        Ok(segments)
    }
    fn segments(&mut self, context: Context) -> PResult<Word> {
        let mut segments: Word = Vec::new();
        let mut literal = String::new();
        let quoted_context = !matches!(context, Context::Unquoted | Context::Brace(false));
        macro_rules! flush {
            ($quoted:expr) => {
                if !literal.is_empty() {
                    segments.push(Seg {
                        quoted: $quoted,
                        kind: Kind::Lit(std::mem::take(&mut literal)),
                    });
                }
            };
        }
        loop {
            let Some(c) = self.peek() else {
                if matches!(context, Context::Unquoted) {
                    break;
                }
                return self.err("unterminated quote or expansion");
            };
            match context {
                Context::Unquoted => {
                    if c.is_whitespace() || ";&|<>()".contains(c) {
                        break;
                    }
                }
                Context::Double if c == '"' => break,
                Context::Brace(_) if c == '}' => break,
                _ => {}
            }
            match c {
                '\'' if !matches!(context, Context::Double) => {
                    flush!(quoted_context);
                    self.pos += 1;
                    let start = self.pos;
                    while self.peek().is_some_and(|c| c != '\'') {
                        self.pos += 1;
                    }
                    if self.peek().is_none() {
                        return self.err("unterminated single quote");
                    }
                    let text: String = self.chars[start..self.pos].iter().collect();
                    self.pos += 1;
                    segments.push(Seg {
                        quoted: true,
                        kind: Kind::Lit(text),
                    });
                }
                '"' if !matches!(context, Context::Double) => {
                    flush!(quoted_context);
                    self.pos += 1;
                    let inner = self.segments(Context::Double)?;
                    self.pos += 1;
                    if inner.is_empty() {
                        segments.push(Seg {
                            quoted: true,
                            kind: Kind::Lit(String::new()),
                        });
                    }
                    segments.extend(inner);
                }
                '\\' => {
                    self.pos += 1;
                    match self.peek() {
                        None => literal.push('\\'),
                        Some('\n') => self.pos += 1,
                        Some(next) => {
                            if matches!(context, Context::Double)
                                && !matches!(next, '$' | '`' | '"' | '\\')
                            {
                                literal.push('\\');
                            }
                            literal.push(next);
                            self.pos += 1;
                        }
                    }
                }
                '`' => return self.err("command substitution is not supported"),
                '$' => {
                    flush!(quoted_context);
                    let quoted = quoted_context;
                    let kind = self.dollar()?;
                    match kind {
                        Some(kind) => segments.push(Seg { quoted, kind }),
                        None => literal.push('$'),
                    }
                }
                '*' | '?' if matches!(context, Context::Unquoted | Context::Brace(false)) => {
                    return self.err("pathname expansion is not supported");
                }
                _ => {
                    literal.push(c);
                    self.pos += 1;
                }
            }
        }
        flush!(quoted_context);
        Ok(segments)
    }
    /// After a `$`; `None` means a literal dollar.
    fn dollar(&mut self) -> PResult<Option<Kind>> {
        self.pos += 1;
        let Some(c) = self.peek() else {
            return Ok(None);
        };
        match c {
            '(' => self.err("command substitution and arithmetic are not supported"),
            '{' => {
                self.pos += 1;
                let name = self.param_name()?;
                let op = match (self.peek(), self.at(1)) {
                    (Some('}'), _) => None,
                    (Some(':'), Some('-')) => {
                        self.pos += 2;
                        Some(Op::Default { colon: true })
                    }
                    (Some(':'), Some('+')) => {
                        self.pos += 2;
                        Some(Op::Alternate { colon: true })
                    }
                    (Some('-'), _) => {
                        self.pos += 1;
                        Some(Op::Default { colon: false })
                    }
                    (Some('+'), _) => {
                        self.pos += 1;
                        Some(Op::Alternate { colon: false })
                    }
                    (Some('#'), Some('#')) => {
                        self.pos += 2;
                        Some(Op::PrefixLong)
                    }
                    (Some('#'), _) => {
                        self.pos += 1;
                        Some(Op::PrefixShort)
                    }
                    (Some('%'), Some('%')) => {
                        self.pos += 2;
                        Some(Op::SuffixLong)
                    }
                    (Some('%'), _) => {
                        self.pos += 1;
                        Some(Op::SuffixShort)
                    }
                    _ => return self.err("unsupported parameter expansion"),
                };
                let op = match op {
                    None => None,
                    Some(op) => {
                        let pattern = matches!(
                            op,
                            Op::PrefixShort | Op::PrefixLong | Op::SuffixShort | Op::SuffixLong
                        );
                        let argument = self.segments(Context::Brace(false))?;
                        if pattern && argument.iter().any(pattern_has_glob) {
                            return self.err("glob patterns are not supported");
                        }
                        Some((op, argument))
                    }
                };
                if self.peek() != Some('}') {
                    return self.err("unterminated parameter expansion");
                }
                self.pos += 1;
                Ok(Some(Kind::Param { name, op }))
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut name = String::new();
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        name.push(c);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Ok(Some(Kind::Param { name, op: None }))
            }
            c if c.is_ascii_digit() || c == '?' || c == '#' => {
                self.pos += 1;
                Ok(Some(Kind::Param {
                    name: c.to_string(),
                    op: None,
                }))
            }
            '@' | '*' => self.err("`$@` and `$*` are not supported"),
            _ => Ok(None),
        }
    }
    fn param_name(&mut self) -> PResult<String> {
        let mut name = String::new();
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        name.push(c);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
            Some(c) if c.is_ascii_digit() => {
                name.push(c);
                self.pos += 1;
            }
            Some(c @ ('?' | '#')) if self.at(1) == Some('}') => {
                name.push(c);
                self.pos += 1;
            }
            _ => return self.err("unsupported parameter expansion"),
        }
        Ok(name)
    }
}

fn keyword_static(keyword: &str) -> &'static str {
    KEYWORDS
        .iter()
        .copied()
        .find(|candidate| *candidate == keyword)
        .unwrap_or("")
}

fn pattern_has_glob(segment: &Seg) -> bool {
    !segment.quoted && matches!(&segment.kind, Kind::Lit(text) if text.contains(['*', '?', '[']))
}

#[derive(Clone, Copy)]
enum Context {
    Unquoted,
    Double,
    /// Argument of `${name op arg}`; the flag marks an enclosing double quote.
    Brace(bool),
}

fn split_assignment(word: &Word) -> Option<(String, Word)> {
    let Some(Seg {
        quoted: false,
        kind: Kind::Lit(first),
    }) = word.first()
    else {
        return None;
    };
    let (name, rest) = first.split_once('=')?;
    if name.is_empty()
        || name.starts_with(|c: char| c.is_ascii_digit())
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    let mut value = Word::new();
    if !rest.is_empty() {
        value.push(Seg {
            quoted: false,
            kind: Kind::Lit(rest.to_owned()),
        });
    }
    value.extend(word[1..].iter().cloned());
    Some((name.to_owned(), value))
}

// ------------------------------------------------------------- evaluation

enum Ctl {
    Return(i32),
    Error(String),
}
type Flow = Result<i32, Ctl>;

struct Shell {
    vars: BTreeMap<String, String>,
    exported: BTreeSet<String>,
    functions: BTreeMap<String, List>,
    positional: Vec<Vec<String>>,
    status: i32,
    steps: usize,
}

fn fail<T>(detail: impl Into<String>) -> Result<T, Ctl> {
    Err(Ctl::Error(detail.into()))
}

impl Shell {
    fn new(prefix: &str) -> Self {
        let mut vars = BTreeMap::new();
        vars.insert("CONDA_PREFIX".into(), prefix.to_owned());
        let mut exported = BTreeSet::new();
        exported.insert("CONDA_PREFIX".into());
        Self {
            vars,
            exported,
            functions: BTreeMap::new(),
            positional: vec![Vec::new()],
            status: 0,
            steps: 0,
        }
    }
    fn ifs(&self) -> String {
        self.vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".into())
    }
    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            "?" => Some(self.status.to_string()),
            "#" => Some((self.positional.last().map_or(0, Vec::len)).to_string()),
            digit if digit.chars().all(|c| c.is_ascii_digit()) => {
                let index: usize = digit.parse().ok()?;
                if index == 0 {
                    return Some("activate".into());
                }
                self.positional.last()?.get(index - 1).cloned()
            }
            _ => self.vars.get(name).cloned(),
        }
    }
    fn param(&self, name: &str, op: &Option<(Op, Word)>) -> Result<String, Ctl> {
        let value = self.lookup(name);
        let Some((op, argument)) = op else {
            return Ok(value.unwrap_or_default());
        };
        Ok(match op {
            Op::Default { colon } => match value {
                Some(value) if !(*colon && value.is_empty()) => value,
                _ => self.string(argument)?,
            },
            Op::Alternate { colon } => match value {
                Some(value) if !(*colon && value.is_empty()) => self.string(argument)?,
                _ => String::new(),
            },
            Op::PrefixShort | Op::PrefixLong => {
                let value = value.unwrap_or_default();
                let pattern = self.string(argument)?;
                value.strip_prefix(&pattern).unwrap_or(&value).to_owned()
            }
            Op::SuffixShort | Op::SuffixLong => {
                let value = value.unwrap_or_default();
                let pattern = self.string(argument)?;
                value.strip_suffix(&pattern).unwrap_or(&value).to_owned()
            }
        })
    }
    /// Expand without field splitting (assignments and parameter arguments).
    fn string(&self, word: &Word) -> Result<String, Ctl> {
        let mut text = String::new();
        for segment in word {
            match &segment.kind {
                Kind::Lit(literal) => text.push_str(literal),
                Kind::Param { name, op } => text.push_str(&self.param(name, op)?),
            }
            if text.len() > MAX_VALUE_BYTES {
                return fail("value exceeds its byte bound");
            }
        }
        Ok(text)
    }
    /// Expand to command fields, splitting unquoted expansions on `IFS`.
    fn fields(&self, word: &Word) -> Result<Vec<String>, Ctl> {
        let ifs = self.ifs();
        let mut fields = Vec::new();
        let mut current: Option<String> = None;
        for segment in word {
            match &segment.kind {
                Kind::Lit(literal) => current.get_or_insert_with(String::new).push_str(literal),
                Kind::Param { name, op } => {
                    let value = self.param(name, op)?;
                    if segment.quoted {
                        current.get_or_insert_with(String::new).push_str(&value);
                        continue;
                    }
                    if value.contains(['*', '?', '[']) {
                        return fail("pathname expansion is not supported");
                    }
                    for c in value.chars() {
                        if ifs.contains(c) {
                            if c.is_whitespace() {
                                fields.extend(current.take());
                            } else {
                                fields.push(current.take().unwrap_or_default());
                            }
                        } else {
                            current.get_or_insert_with(String::new).push(c);
                        }
                    }
                }
            }
        }
        fields.extend(current);
        Ok(fields)
    }
    fn tick(&mut self) -> Result<(), Ctl> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return fail("script exceeds its step bound");
        }
        Ok(())
    }
    fn run_list(&mut self, list: &List, depth: usize) -> Flow {
        let mut status = 0;
        for and_or in list {
            status = self.run_and_or(and_or, depth)?;
            self.status = status;
        }
        Ok(status)
    }
    fn run_and_or(&mut self, chain: &AndOr, depth: usize) -> Flow {
        let (negated, command) = &chain.first;
        let mut status = self.run_command(command, depth)?;
        if *negated {
            status = i32::from(status == 0);
        }
        self.status = status;
        for (and, negated, command) in &chain.rest {
            if (*and && status != 0) || (!*and && status == 0) {
                continue;
            }
            status = self.run_command(command, depth)?;
            if *negated {
                status = i32::from(status == 0);
            }
            self.status = status;
        }
        Ok(status)
    }
    fn run_command(&mut self, command: &Cmd, depth: usize) -> Flow {
        self.tick()?;
        match command {
            Cmd::Assign(assignments) => {
                for (name, word) in assignments {
                    let value = self.string(word)?;
                    self.set(name, value)?;
                }
                Ok(0)
            }
            Cmd::If { arms, otherwise } => {
                for (condition, body) in arms {
                    if self.run_list(condition, depth)? == 0 {
                        return self.run_list(body, depth);
                    }
                }
                match otherwise {
                    Some(body) => self.run_list(body, depth),
                    None => Ok(0),
                }
            }
            Cmd::While(condition, body) => {
                while self.run_list(condition, depth)? == 0 {
                    self.tick()?;
                    self.run_list(body, depth)?;
                }
                Ok(0)
            }
            Cmd::For(name, words, body) => {
                let mut items = Vec::new();
                for word in words {
                    items.extend(self.fields(word)?);
                }
                let mut status = 0;
                for item in items {
                    self.tick()?;
                    self.set(name, item)?;
                    status = self.run_list(body, depth)?;
                }
                Ok(status)
            }
            Cmd::Function(name, body) => {
                self.functions.insert(name.clone(), body.clone());
                Ok(0)
            }
            Cmd::Simple(words, line) => self.simple(words, *line, depth),
        }
    }
    fn set(&mut self, name: &str, value: String) -> Result<(), Ctl> {
        if value.len() > MAX_VALUE_BYTES {
            return fail(format!("{name} exceeds its byte bound"));
        }
        if name == "CONDA_PREFIX" {
            return fail("activation may not reassign CONDA_PREFIX");
        }
        self.vars.insert(name.to_owned(), value);
        Ok(())
    }
    fn simple(&mut self, words: &[Word], line: usize, depth: usize) -> Flow {
        let mut argv = Vec::new();
        for word in words {
            argv.extend(self.fields(word)?);
        }
        let Some((command, arguments)) = argv.split_first() else {
            return Ok(0);
        };
        let at = |detail: String| Ctl::Error(format!("line {line}: {detail}"));
        match command.as_str() {
            ":" | "true" => Ok(0),
            "false" => Ok(1),
            "export" => {
                for argument in arguments {
                    let (name, value) = match argument.split_once('=') {
                        Some((name, value)) => (name, Some(value.to_owned())),
                        None => (argument.as_str(), None),
                    };
                    if name.is_empty()
                        || name.starts_with(|c: char| c.is_ascii_digit())
                        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        return Err(at(format!("export of invalid name {name:?}")));
                    }
                    if name != "CONDA_PREFIX" && reserved(name) {
                        return Err(at(format!(
                            "activation sets {name}, which gripsack manages itself"
                        )));
                    }
                    if let Some(value) = value {
                        self.set(name, value).map_err(|error| match error {
                            Ctl::Error(detail) => at(detail),
                            other => other,
                        })?;
                    }
                    self.exported.insert(name.to_owned());
                }
                Ok(0)
            }
            "unset" => {
                for name in arguments.iter().filter(|name| name.as_str() != "-v") {
                    if name == "CONDA_PREFIX" {
                        return Err(at("activation may not unset CONDA_PREFIX".into()));
                    }
                    self.vars.remove(name);
                    self.exported.remove(name);
                    self.functions.remove(name);
                }
                Ok(0)
            }
            "return" => {
                let code = match arguments.first() {
                    Some(value) => value
                        .parse()
                        .map_err(|_| at(format!("return of non-numeric status {value:?}")))?,
                    None => self.status,
                };
                Err(Ctl::Return(code))
            }
            "test" => self.test(arguments).map_err(&at),
            "[" => match arguments.split_last() {
                Some((last, rest)) if last == "]" => self.test(rest).map_err(&at),
                _ => Err(at("`[` without closing `]`".into())),
            },
            name if self.functions.contains_key(name) => {
                if depth >= MAX_DEPTH {
                    return Err(at("function nesting is too deep".into()));
                }
                let body = self.functions[name].clone();
                self.positional.push(arguments.to_vec());
                let outcome = self.run_list(&body, depth + 1);
                self.positional.pop();
                match outcome {
                    Ok(status) | Err(Ctl::Return(status)) => Ok(status),
                    Err(error) => Err(error),
                }
            }
            other => Err(at(format!(
                "command `{other}` is not supported in activation scripts"
            ))),
        }
    }
    fn test(&self, arguments: &[String]) -> Result<i32, String> {
        let (negated, arguments) = match arguments.split_first() {
            Some((first, rest)) if first == "!" => (true, rest),
            _ => (false, arguments),
        };
        let truth = match arguments {
            [] => false,
            [value] => !value.is_empty(),
            [flag, value] if flag == "-n" => !value.is_empty(),
            [flag, value] if flag == "-z" => value.is_empty(),
            [left, operator, right] if operator == "=" || operator == "==" => left == right,
            [left, operator, right] if operator == "!=" => left != right,
            _ => return Err("unsupported `test` expression".into()),
        };
        Ok(i32::from(truth == negated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(script: &str) -> Result<BTreeMap<String, String>, String> {
        evaluate([("test.sh", script)], "/p")
    }

    #[test]
    fn simple_exports_resolve_against_the_prefix_with_ambient_unset() {
        let result = run("#!/bin/sh\n\
             export XDG_DATA_DIRS_CONDA_BACKUP=\"${XDG_DATA_DIRS:-}\"\n\
             export XDG_DATA_DIRS=\"$CONDA_PREFIX/share:${XDG_DATA_DIRS:-/usr/share}\"\n")
        .unwrap();
        assert_eq!(result["XDG_DATA_DIRS"], "/p/share:/usr/share");
        assert!(!result.contains_key("XDG_DATA_DIRS_CONDA_BACKUP"));
    }

    #[test]
    fn libxml2_split_activation_script_evaluates() {
        let script = r##"#!/bin/sh

if test -n "${XML_CATALOG_FILES:-}"; then
    xml_catalog_files_libxml2="${XML_CATALOG_FILES}"
    XML_CATALOG_FILES="${XML_CATALOG_FILES} "
else
    xml_catalog_files_libxml2=""
    XML_CATALOG_FILES=""
fi

conda_catalog_files=""
ifs_libxml2="${IFS}"
IFS=" "
rem="${CONDA_PREFIX}"
for pre in ${rem}; do
    while test "${rem#"${pre}"}" = "${rem}"; do
	conda_catalog_files="${conda_catalog_files}%20"
	rem=${rem#" "}
    done
    conda_catalog_files="${conda_catalog_files}${pre}"
    rem=${rem#"${pre}"}
done
IFS="${ifs_libxml2}"

conda_catalog_files="file://${conda_catalog_files}/etc/xml/catalog file:///etc/xml/catalog"
export XML_CATALOG_FILES="${XML_CATALOG_FILES}${conda_catalog_files}"
unset conda_catalog_files ifs_libxml2 rem
"##;
        let result = evaluate([("libxml2.sh", script)], "/opt/my env/p").unwrap();
        assert_eq!(
            result["XML_CATALOG_FILES"],
            "file:///opt/my%20env/p/etc/xml/catalog file:///etc/xml/catalog"
        );
        let result = evaluate([("libxml2.sh", script)], "/opt/p").unwrap();
        assert_eq!(
            result["XML_CATALOG_FILES"],
            "file:///opt/p/etc/xml/catalog file:///etc/xml/catalog"
        );
    }

    #[test]
    fn functions_conditionals_and_return_are_interpreted() {
        let result = run("set_it() {\n  if [ \"$1\" = a ] && [ -n \"$2\" ]; then\n    export V=\"$2-$#\"\n    return 0\n  fi\n  export V=no\n}\nset_it a b\nset_it x y\n[ -z \"${W-}\" ] || export BAD=1\n")
        .unwrap();
        assert_eq!(result["V"], "no");
        assert!(!result.contains_key("BAD"));
    }

    #[test]
    fn top_level_return_ends_the_script() {
        let result = run("export A=1\nreturn 0\nexport B=2\n").unwrap();
        assert_eq!(result.keys().collect::<Vec<_>>(), ["A"]);
    }

    #[test]
    fn unsupported_constructs_are_refused_with_their_line() {
        for (script, needle) in [
            ("export A=$(id)\n", "command substitution"),
            ("export A=`id`\n", "command substitution"),
            ("\n\ncurl http://x\n", "line 3: command `curl`"),
            ("echo hi | cat\n", "pipelines"),
            ("echo > f\n", "redirections"),
            ("export A=*\n", "pathname"),
            ("[ -w /x ]\n", "unsupported `test`"),
            ("export PATH=/x\n", "manages itself"),
            ("export LD_PRELOAD=/x\n", "manages itself"),
            ("FOO=1 cmd\n", "assignment prefixes"),
            ("case x in x) ;; esac\n", "not supported"),
        ] {
            let error = run(script).unwrap_err();
            assert!(error.contains(needle), "{script:?}: {error}");
        }
    }

    #[test]
    fn evaluation_is_bounded() {
        let error = run("while true; do :; done\n").unwrap_err();
        assert!(error.contains("step bound"), "{error}");
        let error = run("f() { f; }\nf\n").unwrap_err();
        assert!(error.contains("too deep"), "{error}");
    }

    #[test]
    fn scripts_share_one_shell_in_order() {
        let result = evaluate(
            [
                ("a.sh", "base=$CONDA_PREFIX/x\n"),
                ("b.sh", "export B=\"$base\"\n"),
            ],
            "/p",
        )
        .unwrap();
        assert_eq!(result["B"], "/p/x");
    }
}
