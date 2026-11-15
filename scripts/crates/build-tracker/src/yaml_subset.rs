//! The small YAML subset a ticket file (`docs/specs/<name>/tickets.yaml`) is
//! written in: block mappings and lists, plain, single- and double-quoted
//! scalars, `[]` and comments. Anything else is refused with the line it is on.

use crate::state::Failure;
use crate::text::{is_space, is_terminator, trim};

#[derive(Clone, Debug, PartialEq)]
pub enum Yaml {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Yaml>),
    /// Entries in file order; a key appears once.
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }
}

struct Line {
    indent: usize,
    no: usize,
    text: String,
}

fn refused(no: usize, reason: &str) -> Failure {
    Failure::new("invalid-tickets", format!("the ticket file is not readable at line {no}: {reason}"))
}

fn is_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

/// `^([A-Za-z_][\w-]*):(?: (.*))?$`: the key and what follows the colon.
fn split_key(text: &str) -> Option<(&str, &str)> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let end = chars.find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-')).map_or(text.len(), |(index, _)| index);
    let rest = text[end..].strip_prefix(':')?;
    if rest.is_empty() {
        return Some((&text[..end], ""));
    }
    let value = rest.strip_prefix(' ')?;
    (!value.chars().any(is_terminator)).then_some((&text[..end], value))
}

struct Reader {
    lines: Vec<Line>,
    position: usize,
}

impl Reader {
    fn peek(&self) -> Option<&Line> {
        self.lines.get(self.position)
    }

    /// The block that starts at the line with `indent` and text `text`.
    fn node(&mut self, indent: usize, text: &str) -> Result<Yaml, Failure> {
        if is_item(text) {
            self.list(indent)
        } else {
            self.map(indent)
        }
    }

    fn list(&mut self, indent: usize) -> Result<Yaml, Failure> {
        let mut items = Vec::new();
        while let Some(line) = self.peek().filter(|line| line.indent == indent && is_item(&line.text)) {
            let no = line.no;
            let text = line.text.clone();
            let rest = text[1..].trim_start_matches(is_space).to_string();
            if rest.is_empty() {
                self.position += 1;
                let Some(next) = self.peek().filter(|next| next.indent > indent) else {
                    return Err(refused(no, "a list item has no value"));
                };
                let (next_indent, next_text) = (next.indent, next.text.clone());
                items.push(self.node(next_indent, &next_text)?);
            } else if split_key(&rest).is_some() {
                let inner = indent + (text.chars().count() - rest.chars().count());
                self.lines[self.position] = Line { indent: inner, no, text: rest };
                items.push(self.map(inner)?);
            } else {
                self.position += 1;
                items.push(scalar(&rest, no)?);
            }
        }
        Ok(Yaml::List(items))
    }

    fn map(&mut self, indent: usize) -> Result<Yaml, Failure> {
        let mut entries: Vec<(String, Yaml)> = Vec::new();
        while let Some(line) = self.peek().filter(|line| line.indent == indent && !is_item(&line.text)) {
            let no = line.no;
            let Some((key, rest)) = split_key(&line.text) else {
                return Err(refused(no, "expected `key: value`"));
            };
            let (key, rest) = (key.to_string(), rest.to_string());
            if key == "__proto__" || entries.iter().any(|(name, _)| *name == key) {
                return Err(refused(no, &format!("the key {key} appears twice")));
            }
            self.position += 1;
            let rest = if rest.starts_with('#') { "" } else { rest.as_str() };
            let rest = trim(rest);
            if !rest.is_empty() {
                entries.push((key, scalar(rest, no)?));
                continue;
            }
            let nested = self.peek().filter(|next| next.indent > indent || (next.indent == indent && is_item(&next.text))).map(|next| (next.indent, next.text.clone()));
            let value = match nested {
                Some((next_indent, next_text)) => self.node(next_indent, &next_text)?,
                None => Yaml::Null,
            };
            entries.push((key, value));
        }
        if let Some(stray) = self.peek().filter(|stray| stray.indent > indent) {
            return Err(refused(stray.no, "unexpected indentation"));
        }
        Ok(Yaml::Map(entries))
    }
}

/// Reads the ticket file's text, or says which line it cannot read.
pub fn parse_yaml_subset(source: &str) -> Result<Yaml, Failure> {
    let mut lines = Vec::new();
    for (index, raw) in source.strip_prefix('\u{feff}').unwrap_or(source).split('\n').enumerate() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let body = raw.trim_start_matches(is_space);
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if raw.trim_start_matches(' ').starts_with('\t') {
            return Err(refused(index + 1, "a tab cannot indent"));
        }
        let indent = raw.encode_utf16().count() - body.encode_utf16().count();
        lines.push(Line { indent, no: index + 1, text: body.trim_end_matches(is_space).to_string() });
    }
    let Some(first) = lines.first() else {
        return Err(refused(1, "the file is empty"));
    };
    let (indent, text) = (first.indent, first.text.clone());
    let mut reader = Reader { lines, position: 0 };
    let value = reader.node(indent, &text)?;
    match reader.peek() {
        Some(rest) => Err(refused(rest.no, "this line does not fit its parent")),
        None => Ok(value),
    }
}

/// The index of the quote that closes a single-quoted scalar (`''` is a quote inside it).
fn closing_quote(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().skip(1).peekable();
    while let Some((index, c)) = chars.next() {
        if c != '\'' {
            continue;
        }
        if chars.next_if(|(_, next)| *next == '\'').is_none() {
            return Some(index);
        }
    }
    None
}

/// What may follow a quoted scalar: spaces and a comment.
fn only_comment(tail: &str) -> bool {
    let tail = tail.trim_start_matches(is_space);
    tail.is_empty() || (tail.starts_with('#') && !tail.chars().any(is_terminator))
}

/// The text of a double-quoted scalar, a JSON string, with what follows it already checked.
fn double_quoted(text: &str) -> Option<String> {
    let mut escaped = false;
    let mut end = None;
    for (index, c) in text.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            end = Some(index);
            break;
        }
    }
    let end = end?;
    if !only_comment(&text[end + 1..]) {
        return None;
    }
    match crate::json::parse(&text[..=end]) {
        Ok(crate::json::Value::Str(value)) => Some(value),
        _ => None,
    }
}

fn scalar(text: &str, no: usize) -> Result<Yaml, Failure> {
    if text.starts_with('\'') {
        let end = closing_quote(text).filter(|end| only_comment(&text[end + 1..]));
        let Some(end) = end else {
            return Err(refused(no, "a single-quoted value must end with its quote"));
        };
        return Ok(Yaml::Str(text[1..end].replace("''", "'")));
    }
    if text.starts_with('"') {
        return double_quoted(text).map(Yaml::Str).ok_or_else(|| refused(no, "a double-quoted value must be a JSON string"));
    }
    if text == "[]" {
        return Ok(Yaml::List(Vec::new()));
    }
    if text.starts_with(['[', '{', '|', '>', '&', '*', '!', '%', '@', '`']) {
        return Err(refused(no, "this YAML form is not supported in a ticket file"));
    }
    let plain = without_comment(text);
    if plain.contains(": ") || plain.ends_with(':') {
        return Err(refused(no, "a plain value cannot hold `: `; quote it"));
    }
    Ok(match plain {
        "null" | "~" => Yaml::Null,
        "true" => Yaml::Bool(true),
        "false" => Yaml::Bool(false),
        _ if is_integer(plain) => Yaml::Num(plain.parse().unwrap_or(f64::NAN)),
        _ => Yaml::Str(plain.to_string()),
    })
}

/// `text.replace(/\s+#.*$/, "")`: a comment starts at the first `#` that follows whitespace.
fn without_comment(text: &str) -> &str {
    let mut start: Option<usize> = None;
    for (index, c) in text.char_indices() {
        if is_space(c) {
            start.get_or_insert(index);
        } else {
            if c == '#' {
                if let Some(start) = start {
                    return &text[..start];
                }
            }
            start = None;
        }
    }
    text
}

fn is_integer(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}
