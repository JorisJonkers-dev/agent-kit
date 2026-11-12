//! A small TOML reader for the TOML files the runners read: a Vibe critic agent
//! file, the home's `config.toml`, and the models that configuration declares.
//! It reads what those files hold (tables, arrays of tables, dotted and quoted
//! keys, basic and literal strings, single- and multi-line, booleans, numbers,
//! arrays and inline tables, comments) and fails on anything it cannot read, so
//! a caller never acts on half a file. Dates and the finer points of number
//! syntax are kept as their text.

use crate::json::{Object, Value};

/// The document a TOML file holds: a table, which is a JSON object whose keys
/// stand in the order the file declares them.
pub fn read_toml(text: &str) -> Result<Value, String> {
    TomlReader { chars: text.chars().collect(), at: 0 }.document()
}

struct TomlReader {
    chars: Vec<char>,
    at: usize,
}

impl TomlReader {
    fn document(&mut self) -> Result<Value, String> {
        let mut root = Object::new();
        // The table the next assignment lands in, by its path from the root: an
        // array of tables resolves to its latest table, as the reader's own
        // table lookup does.
        let mut table: Vec<String> = Vec::new();
        loop {
            self.skip(true);
            if self.at >= self.chars.len() {
                return Ok(Value::Obj(root));
            }
            if self.starts_with("[[") {
                self.at += 2;
                let path = self.key_path("]]")?;
                let (last, parent) = path.split_last().expect("a key path names at least one key");
                array_at(&mut root, parent, last)?.push(Value::Obj(Object::new()));
                table = path;
            } else if self.peek() == Some('[') {
                self.at += 1;
                let path = self.key_path("]")?;
                table_at(&mut root, &path)?;
                table = path;
            } else {
                let path = self.key_path("=")?;
                let value = self.value()?;
                assign(&mut root, &table, &path, value)?;
            }
            self.end_of_line()?;
        }
    }

    /// A dotted key, up to and past `end`.
    fn key_path(&mut self, end: &str) -> Result<Vec<String>, String> {
        let mut path = Vec::new();
        loop {
            self.skip(false);
            path.push(self.key()?);
            self.skip(false);
            if self.starts_with(end) {
                self.at += end.chars().count();
                return Ok(path);
            }
            self.expect('.')?;
        }
    }

    fn key(&mut self) -> Result<String, String> {
        if let Some(quote @ ('"' | '\'')) = self.peek() {
            return self.string(quote);
        }
        let start = self.at;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            self.at += 1;
        }
        if self.at == start {
            return Err(self.error("a key"));
        }
        Ok(self.chars[start..self.at].iter().collect())
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip(false);
        match self.peek() {
            Some(quote @ ('"' | '\'')) => Ok(Value::Str(self.string(quote)?)),
            Some('[') => self.array(),
            Some('{') => self.inline_table(),
            _ => {
                let start = self.at;
                while matches!(self.peek(), Some(c) if !c.is_whitespace() && !matches!(c, ',' | ']' | '}' | '#')) {
                    self.at += 1;
                }
                if self.at == start {
                    return Err(self.error("a value"));
                }
                let word: String = self.chars[start..self.at].iter().collect();
                match word.as_str() {
                    "true" => return Ok(Value::Bool(true)),
                    "false" => return Ok(Value::Bool(false)),
                    _ => {}
                }
                let cleaned: String = word.chars().filter(|c| *c != '_').collect();
                Ok(match number(&cleaned) {
                    Some(value) => Value::Num(value),
                    None => Value::Str(word),
                })
            }
        }
    }

    fn string(&mut self, quote: char) -> Result<String, String> {
        let triple: String = std::iter::repeat(quote).take(3).collect();
        let multi = self.starts_with(&triple);
        let close = if multi { triple } else { quote.to_string() };
        self.at += close.chars().count();
        // A newline right after the opening of a multi-line string is not part of it.
        if multi {
            if self.peek() == Some('\r') && self.chars.get(self.at + 1) == Some(&'\n') {
                self.at += 2;
            } else if self.peek() == Some('\n') {
                self.at += 1;
            }
        }
        let mut out = String::new();
        loop {
            if self.at >= self.chars.len() {
                return Err(self.error(&format!("the closing {close}")));
            }
            if self.starts_with(&close) {
                self.at += close.chars().count();
                return Ok(out);
            }
            let char = self.chars[self.at];
            if char == '\n' && !multi {
                return Err(self.error(&format!("the closing {close}")));
            }
            if char == '\\' && quote == '"' {
                out.push_str(&self.escape()?);
                continue;
            }
            out.push(char);
            self.at += 1;
        }
    }

    fn escape(&mut self) -> Result<String, String> {
        let plain = match self.chars.get(self.at + 1) {
            Some('"') => Some('"'),
            Some('\\') => Some('\\'),
            Some('b') => Some('\u{8}'),
            Some('f') => Some('\u{c}'),
            Some('n') => Some('\n'),
            Some('r') => Some('\r'),
            Some('t') => Some('\t'),
            _ => None,
        };
        if let Some(plain) = plain {
            self.at += 2;
            return Ok(plain.to_string());
        }
        let digits = match self.chars.get(self.at + 1) {
            Some('u') => 4,
            Some('U') => 8,
            _ => return Err(self.error("a valid escape")),
        };
        let start = self.at + 2;
        let end = start + digits;
        if end > self.chars.len() {
            return Err(self.error("a valid escape"));
        }
        let hex: String = self.chars[start..end].iter().collect();
        if !hex.chars().all(|digit| digit.is_ascii_hexdigit()) {
            return Err(self.error("a valid escape"));
        }
        // The digits are hex and at most eight of them, so the parse cannot fail.
        let code = u32::from_str_radix(&hex, 16).unwrap_or(0);
        if code > 0x10ffff {
            return Err(self.error("a valid escape"));
        }
        self.at = end;
        // A lone surrogate cannot live in a Rust string; it reads as U+FFFD.
        Ok(char::from_u32(code).unwrap_or('\u{fffd}').to_string())
    }

    fn array(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut items = Vec::new();
        loop {
            self.skip(true);
            if self.peek() == Some(']') {
                self.at += 1;
                return Ok(Value::Arr(items));
            }
            items.push(self.value()?);
            self.skip(true);
            if self.peek() != Some(']') {
                self.expect(',')?;
            }
        }
    }

    fn inline_table(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut table = Object::new();
        self.skip(false);
        if self.peek() == Some('}') {
            self.at += 1;
            return Ok(Value::Obj(table));
        }
        loop {
            let path = self.key_path("=")?;
            let value = self.value()?;
            assign(&mut table, &[], &path, value)?;
            self.skip(false);
            if self.peek() == Some('}') {
                self.at += 1;
                return Ok(Value::Obj(table));
            }
            self.expect(',')?;
        }
    }

    /// Skips spaces and comments, and newlines too when `lines` is set.
    fn skip(&mut self, lines: bool) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\r') => self.at += 1,
                Some('\n') if lines => self.at += 1,
                Some('#') => {
                    while self.at < self.chars.len() && self.peek() != Some('\n') {
                        self.at += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn end_of_line(&mut self) -> Result<(), String> {
        self.skip(false);
        if self.at < self.chars.len() {
            self.expect('\n')?;
        }
        Ok(())
    }

    fn expect(&mut self, char: char) -> Result<(), String> {
        if self.peek() != Some(char) {
            return Err(self.error(&format!("'{char}'")));
        }
        self.at += 1;
        Ok(())
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn starts_with(&self, text: &str) -> bool {
        let mut at = self.at;
        for char in text.chars() {
            if self.chars.get(at) != Some(&char) {
                return false;
            }
            at += 1;
        }
        true
    }

    fn error(&self, wanted: &str) -> String {
        let line = self.chars[..self.at.min(self.chars.len())].iter().filter(|char| **char == '\n').count() + 1;
        format!("line {line}: expected {wanted}")
    }
}

/// The table at `path` under `root`, created where missing; under an array of tables, its latest table.
fn table_at<'a>(root: &'a mut Object, path: &[String]) -> Result<&'a mut Object, String> {
    let mut table = root;
    for key in path {
        if table.get(key).is_none() {
            table.set(key, Value::Obj(Object::new()));
        }
        let value = table.get_mut(key).expect("the table at the key");
        let resolved = match value {
            Value::Arr(items) => items.last_mut(),
            other => Some(other),
        };
        match resolved {
            Some(Value::Obj(object)) => table = object,
            _ => return Err(format!("a table at {key}")),
        }
    }
    Ok(table)
}

/// The array of tables at `parent`/`last`, created where missing.
fn array_at<'a>(root: &'a mut Object, parent: &[String], last: &str) -> Result<&'a mut Vec<Value>, String> {
    let table = table_at(root, parent)?;
    if table.get(last).is_none() {
        table.set(last, Value::Arr(Vec::new()));
    }
    match table.get_mut(last).expect("the array at the key") {
        Value::Arr(items) => Ok(items),
        _ => Err(format!("an array of tables at {last}")),
    }
}

/// Sets `path` under the table at `base`, refusing a key that is already there.
fn assign(root: &mut Object, base: &[String], path: &[String], value: Value) -> Result<(), String> {
    let (last, parent) = path.split_last().expect("a key path names at least one key");
    let table = table_at(root, base)?;
    let parent = table_at(table, parent)?;
    if parent.get(last).is_some() {
        return Err(format!("a key not already defined ({last})"));
    }
    parent.set(last, value);
    Ok(())
}

/// `Number(word)` for a TOML word: the number JavaScript reads, or `None` where it reads NaN.
fn number(word: &str) -> Option<f64> {
    let (sign, rest) = match word.as_bytes().first() {
        Some(b'-') => (-1.0, &word[1..]),
        Some(b'+') => (1.0, &word[1..]),
        _ => (1.0, word),
    };
    if rest.eq_ignore_ascii_case("infinity") {
        return Some(sign * f64::INFINITY);
    }
    let radix = match rest.get(..2).map(str::to_ascii_lowercase).as_deref() {
        Some("0x") => Some(16),
        Some("0o") => Some(8),
        Some("0b") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        // JavaScript reads a radix prefix only without a sign.
        if rest.len() != word.len() || rest.len() == 2 {
            return None;
        }
        let mut value = 0.0;
        for digit in rest[2..].chars() {
            value = value * f64::from(radix) + f64::from(digit.to_digit(radix)?);
        }
        return Some(value);
    }
    decimal(word)
}

/// `Number(word)` for a decimal word: the number JavaScript reads, or `None` where it reads NaN.
fn decimal(word: &str) -> Option<f64> {
    let bytes = word.as_bytes();
    let mut at = 0;
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        at += 1;
    }
    let mut digits = 0;
    while matches!(bytes.get(at), Some(byte) if byte.is_ascii_digit()) {
        at += 1;
        digits += 1;
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        while matches!(bytes.get(at), Some(byte) if byte.is_ascii_digit()) {
            at += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(at), Some(b'e') | Some(b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+') | Some(b'-')) {
            at += 1;
        }
        let start = at;
        while matches!(bytes.get(at), Some(byte) if byte.is_ascii_digit()) {
            at += 1;
        }
        if at == start {
            return None;
        }
    }
    if at != bytes.len() {
        return None;
    }
    word.parse::<f64>().ok()
}
