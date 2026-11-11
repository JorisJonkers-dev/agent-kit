//! JSON with the behaviour of JavaScript's `JSON.parse` and
//! `JSON.stringify(value, null, 2)`: objects keep their key order (integer
//! keys first, ascending, as a JavaScript object orders them), numbers print
//! as JavaScript prints them, and a syntax error reads as V8 words it.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Object),
}

/// An object's members in insertion order; `entries` gives the order JavaScript enumerates them in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Object {
    members: Vec<(String, Value)>,
}

fn array_index(key: &str) -> Option<u32> {
    let canonical = key == "0" || (!key.starts_with('0') && key.bytes().all(|b| b.is_ascii_digit()) && !key.is_empty());
    canonical.then(|| key.parse::<u32>().ok().filter(|n| *n != u32::MAX)).flatten()
}

impl Object {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.members.iter().find(|(name, _)| name == key).map(|(_, value)| value)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.members.iter_mut().find(|(name, _)| name == key).map(|(_, value)| value)
    }

    /// Sets a member: in place when the key exists, else last.
    pub fn set(&mut self, key: &str, value: Value) {
        match self.get_mut(key) {
            Some(slot) => *slot = value,
            None => self.members.push((key.to_string(), value)),
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.members.retain(|(name, _)| name != key);
    }

    /// The members as a JavaScript object enumerates them.
    pub fn entries(&self) -> Vec<(&str, &Value)> {
        let mut indexed: Vec<(u32, &str, &Value)> = Vec::new();
        let mut named: Vec<(&str, &Value)> = Vec::new();
        for (key, value) in &self.members {
            match array_index(key) {
                Some(index) => indexed.push((index, key, value)),
                None => named.push((key, value)),
            }
        }
        indexed.sort_by_key(|(index, _, _)| *index);
        indexed.into_iter().map(|(_, key, value)| (key, value)).chain(named).collect()
    }
}

impl Value {
    pub fn str(text: &str) -> Value {
        Value::Str(text.to_string())
    }

    pub fn count(number: usize) -> Value {
        Value::Num(number as f64)
    }

    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::Obj(object) => Some(object),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(text) => Some(text),
            _ => None,
        }
    }
}

/// A JSON array of whole numbers.
pub fn numbers(values: &[u64]) -> Value {
    Value::Arr(values.iter().map(|number| Value::Num(*number as f64)).collect())
}

/// An object from `(key, value)` pairs, in order.
pub fn object(pairs: Vec<(&str, Value)>) -> Value {
    let mut result = Object::new();
    for (key, value) in pairs {
        result.set(key, value);
    }
    Value::Obj(result)
}

/// `Number.prototype.toString` for a JSON number; `JSON.stringify` writes a non-finite one as `null`.
pub fn number_text(number: f64) -> String {
    if !number.is_finite() {
        return "null".to_string();
    }
    if number == 0.0 {
        return "0".to_string();
    }
    let sign = if number < 0.0 { "-" } else { "" };
    let scientific = format!("{:e}", number.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let point = exponent.parse::<i32>().unwrap_or(0) + 1;
    let count = digits.len() as i32;
    let body = if count <= point && point <= 21 {
        format!("{digits}{}", "0".repeat((point - count) as usize))
    } else if 0 < point && point <= 21 {
        format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
    } else if -6 < point && point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else {
        let power = point - 1;
        let sign_of_power = if power < 0 { "-" } else { "+" };
        let lead = if count == 1 { digits.clone() } else { format!("{}.{}", &digits[..1], &digits[1..]) };
        format!("{lead}e{sign_of_power}{}", power.abs())
    };
    format!("{sign}{body}")
}

fn quote(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write(value: &Value, indent: Option<usize>, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Num(number) => out.push_str(&number_text(*number)),
        Value::Str(text) => quote(text, out),
        Value::Arr(items) => {
            let rows = items.iter().map(|item| (None, item)).collect();
            container(('[', ']'), rows, indent, out);
        }
        Value::Obj(members) => {
            let rows = members.entries().into_iter().map(|(key, member)| (Some(key), member)).collect();
            container(('{', '}'), rows, indent, out);
        }
    }
}

fn container(brackets: (char, char), rows: Vec<(Option<&str>, &Value)>, indent: Option<usize>, out: &mut String) {
    out.push(brackets.0);
    for (position, (key, member)) in rows.iter().enumerate() {
        out.push_str(if position == 0 { "" } else { "," });
        if let Some(level) = indent {
            out.push('\n');
            out.push_str(&"  ".repeat(level + 1));
        }
        if let Some(key) = key {
            quote(key, out);
            out.push_str(if indent.is_some() { ": " } else { ":" });
        }
        write(member, indent.map(|level| level + 1), out);
    }
    if let (Some(level), false) = (indent, rows.is_empty()) {
        out.push('\n');
        out.push_str(&"  ".repeat(level));
    }
    out.push(brackets.1);
}

/// `JSON.stringify(value, null, 2)`.
pub fn pretty(value: &Value) -> String {
    let mut out = String::new();
    write(value, Some(0), &mut out);
    out
}

/// `JSON.stringify(value)`.
pub fn compact(value: &Value) -> String {
    let mut out = String::new();
    write(value, None, &mut out);
    out
}

/// `JSON.parse`; the error is the message V8 gives.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser { chars: text.chars().collect(), at: 0 };
    parser.skip_space();
    let value = parser.value()?;
    parser.skip_space();
    match parser.peek() {
        None => Ok(value),
        Some(_) => Err(parser.fail("Unexpected non-whitespace character after JSON")),
    }
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.at += 1;
        }
    }

    /// `<what> at position <n> (line <l> column <c>)`; `what` ends in `JSON`.
    fn fail(&self, what: &str) -> String {
        let (mut line, mut column, mut position) = (1, 1, 0);
        for (index, c) in self.chars[..self.at].iter().enumerate() {
            position += c.len_utf16();
            if *c == '\n' || (*c == '\r' && self.chars.get(index + 1) != Some(&'\n')) {
                line += 1;
                column = 1;
            } else if *c != '\r' {
                column += c.len_utf16();
            }
        }
        format!("{what} at position {position} (line {line} column {column})")
    }

    /// V8's `Unexpected token`, with the source around it, or the end of input.
    fn unexpected(&self) -> String {
        let Some(c) = self.peek() else {
            return "Unexpected end of JSON input".to_string();
        };
        let length = self.chars.len();
        let slice = |from: usize, to: usize| self.chars[from..to].iter().collect::<String>();
        let context = if length < 21 {
            format!("\"{}\"", slice(0, length))
        } else {
            let (start, end) = (self.at.saturating_sub(10), (self.at + 10).min(length));
            format!(
                "{}\"{}\"{}",
                if start == 0 { "" } else { "..." },
                slice(start, end),
                if end == length { "" } else { "..." }
            )
        };
        format!("Unexpected token '{c}', {context} is not valid JSON")
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            Some('"') => self.string().map(Value::Str),
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('-' | '0'..='9') => self.number(),
            Some('t') => self.literal("true", Value::Bool(true)),
            Some('f') => self.literal("false", Value::Bool(false)),
            Some('n') => self.literal("null", Value::Null),
            _ => Err(self.unexpected()),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, String> {
        for expected in word.chars() {
            if self.peek() != Some(expected) {
                return Err(self.unexpected());
            }
            self.at += 1;
        }
        Ok(value)
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.at;
        if self.peek() == Some('-') {
            self.at += 1;
            if self.peek().is_none_or(|c| !c.is_ascii_digit()) {
                return Err(self.fail("No number after minus sign in JSON"));
            }
        }
        if self.peek() == Some('0') {
            self.at += 1;
            if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(self.fail("Unexpected number in JSON"));
            }
        } else {
            self.digits();
        }
        if self.peek() == Some('.') {
            self.at += 1;
            if self.digits() == 0 {
                return Err(self.fail("Unterminated fractional number in JSON"));
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.at += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err(self.fail("Exponent part is missing a number in JSON"));
            }
        }
        let text: String = self.chars[start..self.at].iter().collect();
        Ok(Value::Num(text.parse().expect("a checked JSON number reads as a float")))
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.fail("Unterminated string in JSON"));
            };
            match c {
                '"' => {
                    self.at += 1;
                    return Ok(out);
                }
                '\\' => {
                    self.at += 1;
                    out.push(self.escape()?);
                }
                c if (c as u32) < 0x20 => return Err(self.fail("Bad control character in string literal in JSON")),
                c => {
                    out.push(c);
                    self.at += 1;
                }
            }
        }
    }

    fn escape(&mut self) -> Result<char, String> {
        let Some(c) = self.peek() else {
            return Err(self.unexpected());
        };
        let simple = match c {
            '"' | '\\' | '/' => Some(c),
            'b' => Some('\u{8}'),
            'f' => Some('\u{c}'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            _ => None,
        };
        if let Some(plain) = simple {
            self.at += 1;
            return Ok(plain);
        }
        if c != 'u' {
            return Err(self.fail("Bad escaped character in JSON"));
        }
        self.at += 1;
        let code = self.hex4()?;
        let low = (0xd800..0xdc00).contains(&code) && self.peek() == Some('\\') && self.chars.get(self.at + 1) == Some(&'u');
        if low {
            let saved = self.at;
            self.at += 2;
            let pair = self.hex4()?;
            if (0xdc00..0xe000).contains(&pair) {
                return Ok(char::from_u32(0x10000 + ((code - 0xd800) << 10) + (pair - 0xdc00)).expect("a surrogate pair is a scalar value"));
            }
            self.at = saved;
        }
        // A lone surrogate cannot live in a Rust string; it reads as U+FFFD.
        Ok(char::from_u32(code).unwrap_or('\u{fffd}'))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut code = 0;
        for _ in 0..4 {
            let Some(digit) = self.peek().and_then(|d| d.to_digit(16)) else {
                return Err(self.fail("Bad Unicode escape in JSON"));
            };
            code = code * 16 + digit;
            self.at += 1;
        }
        Ok(code)
    }

    fn array(&mut self) -> Result<Value, String> {
        self.at += 1;
        self.skip_space();
        let mut items = Vec::new();
        if self.peek() == Some(']') {
            self.at += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_space();
            match self.peek() {
                Some(',') => {
                    self.at += 1;
                    self.skip_space();
                }
                Some(']') => {
                    self.at += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return Err(self.fail("Expected ',' or ']' after array element in JSON")),
            }
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.at += 1;
        self.skip_space();
        let mut members = Object::new();
        if self.peek() == Some('}') {
            self.at += 1;
            return Ok(Value::Obj(members));
        }
        if self.peek() != Some('"') {
            return Err(self.fail("Expected property name or '}' in JSON"));
        }
        loop {
            let key = self.string()?;
            self.skip_space();
            if self.peek() != Some(':') {
                return Err(self.fail("Expected ':' after property name in JSON"));
            }
            self.at += 1;
            self.skip_space();
            members.set(&key, self.value()?);
            self.skip_space();
            match self.peek() {
                Some(',') => {
                    self.at += 1;
                    self.skip_space();
                    if self.peek() != Some('"') {
                        return Err(self.fail("Expected double-quoted property name in JSON"));
                    }
                }
                Some('}') => {
                    self.at += 1;
                    return Ok(Value::Obj(members));
                }
                _ => return Err(self.fail("Expected ',' or '}' after property value in JSON")),
            }
        }
    }
}
