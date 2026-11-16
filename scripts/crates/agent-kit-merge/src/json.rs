//! JSON as Python's `json` module reads and writes it: objects keep their key
//! order, `dumps(indent=2)` escapes everything outside ASCII, and a number is
//! kept as the text it was written with (so an integer never turns into a float).

use crate::Failure;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number as Python prints it: an integer as written, a float as `repr` shows it.
    Num(String),
    Str(String),
    Arr(Vec<Value>),
    Obj(Object),
}

/// An object's members in insertion order. Setting an existing key keeps its place.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Object {
    members: Vec<(String, Value)>,
}

impl Object {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.members.iter().find(|(name, _)| name == key).map(|(_, value)| value)
    }

    pub fn set(&mut self, key: &str, value: Value) {
        match self.members.iter_mut().find(|(name, _)| name == key) {
            Some(slot) => slot.1 = value,
            None => self.members.push((key.to_string(), value)),
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.members.iter_mut().find(|(name, _)| name == key).map(|(_, value)| value)
    }

    /// Removes a member, as `dict.pop(key, None)` does.
    pub fn remove(&mut self, key: &str) {
        self.members.retain(|(name, _)| name != key);
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn keys(&self) -> Vec<String> {
        self.members.iter().map(|(name, _)| name.clone()).collect()
    }

    pub fn entries(&self) -> &[(String, Value)] {
        &self.members
    }
}

impl Value {
    /// Python's truthiness: what `x or {}` would replace.
    pub fn is_falsy(&self) -> bool {
        match self {
            Value::Null | Value::Bool(false) => true,
            Value::Num(text) => text.parse::<f64>().map(|n| n == 0.0).unwrap_or(false),
            Value::Str(text) => text.is_empty(),
            Value::Arr(items) => items.is_empty(),
            Value::Obj(object) => object.members.is_empty(),
            Value::Bool(true) => false,
        }
    }

    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::Obj(object) => Some(object),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Object> {
        match self {
            Value::Obj(object) => Some(object),
            _ => None,
        }
    }
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
}

/// Python's `json.loads`, with its error wording.
pub fn parse(text: &str) -> Result<Value, Failure> {
    let mut reader = Reader { text, at: 0 };
    reader.skip();
    let value = reader.value()?;
    reader.skip();
    if reader.at < text.len() {
        return Err(reader.fail("Extra data"));
    }
    Ok(value)
}

impl Reader<'_> {
    fn fail(&self, what: &str) -> Failure {
        let before = &self.text[..self.at];
        let line = before.matches('\n').count() + 1;
        let column = self.at - before.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;
        Failure(format!("{what}: line {line} column {column} (char {})", self.at))
    }

    fn peek(&self) -> Option<char> {
        self.text[self.at..].chars().next()
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, word: &str) -> bool {
        let found = self.text[self.at..].starts_with(word);
        if found {
            self.at += word.len();
        }
        found
    }

    fn value(&mut self) -> Result<Value, Failure> {
        match self.peek() {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => Ok(Value::Str(self.string()?)),
            _ if self.eat("true") => Ok(Value::Bool(true)),
            _ if self.eat("false") => Ok(Value::Bool(false)),
            _ if self.eat("null") => Ok(Value::Null),
            _ => self.number(),
        }
    }

    fn number(&mut self) -> Result<Value, Failure> {
        let rest = &self.text[self.at..];
        let end = rest.find(|c: char| !matches!(c, '0'..='9' | '-' | '+' | '.' | 'e' | 'E')).unwrap_or(rest.len());
        let token = &rest[..end];
        let digits = token.strip_prefix('-').unwrap_or(token);
        let leading_zero = digits.len() > 1 && digits.starts_with('0') && !digits.starts_with("0.") && !digits.starts_with("0e") && !digits.starts_with("0E");
        if token.is_empty() || token.parse::<f64>().is_err() || leading_zero {
            return Err(self.fail("Expecting value"));
        }
        self.at += end;
        Ok(Value::Num(normal_number(token)))
    }

    fn array(&mut self) -> Result<Value, Failure> {
        self.at += 1;
        let mut items = Vec::new();
        self.skip();
        if self.peek() == Some(']') {
            self.at += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            self.skip();
            items.push(self.value()?);
            self.skip();
            match self.peek() {
                Some(',') => self.at += 1,
                Some(']') => {
                    self.at += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return Err(self.fail("Expecting ',' delimiter")),
            }
        }
    }

    fn object(&mut self) -> Result<Value, Failure> {
        self.at += 1;
        let mut object = Object::default();
        self.skip();
        if self.peek() == Some('}') {
            self.at += 1;
            return Ok(Value::Obj(object));
        }
        loop {
            self.skip();
            if self.peek() != Some('"') {
                return Err(self.fail("Expecting property name enclosed in double quotes"));
            }
            let key = self.string()?;
            self.skip();
            if self.peek() != Some(':') {
                return Err(self.fail("Expecting ':' delimiter"));
            }
            self.at += 1;
            self.skip();
            let value = self.value()?;
            object.set(&key, value);
            self.skip();
            match self.peek() {
                Some(',') => self.at += 1,
                Some('}') => {
                    self.at += 1;
                    return Ok(Value::Obj(object));
                }
                _ => return Err(self.fail("Expecting ',' delimiter")),
            }
        }
    }

    /// The four hex digits after `\u`; Python reports a bad escape at its `u`.
    fn unit(&mut self) -> Result<u32, Failure> {
        let digits = self.text.get(self.at..self.at + 4).filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()));
        let Some(unit) = digits.and_then(|d| u32::from_str_radix(d, 16).ok()) else {
            self.at -= 1;
            return Err(self.fail("Invalid \\uXXXX escape"));
        };
        self.at += 4;
        Ok(unit)
    }

    fn string(&mut self) -> Result<String, Failure> {
        let start = self.at;
        self.at += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                self.at = start;
                return Err(self.fail("Unterminated string starting at"));
            };
            self.at += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some(e) = self.peek() else {
                        self.at = start;
                        return Err(self.fail("Unterminated string starting at"));
                    };
                    self.at += e.len_utf8();
                    match e {
                        '"' | '\\' | '/' => out.push(e),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let high = self.unit()?;
                            let mut code = high;
                            if (0xD800..0xDC00).contains(&high) {
                                let resume = self.at;
                                match (self.eat("\\u"), self.unit()) {
                                    (true, Ok(low)) if (0xDC00..0xE000).contains(&low) => {
                                        code = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                                    }
                                    _ => self.at = resume,
                                }
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => {
                            self.at -= e.len_utf8() + 1;
                            return Err(self.fail("Invalid \\escape"));
                        }
                    }
                }
                c if (c as u32) < 0x20 => {
                    self.at -= 1;
                    return Err(self.fail("Invalid control character at"));
                }
                c => out.push(c),
            }
        }
    }
}

fn quote(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            '\u{7f}' => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

fn write(value: &Value, depth: usize, out: &mut String) {
    let pad = |level: usize| "  ".repeat(level);
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Num(text) => out.push_str(text),
        Value::Str(text) => quote(text, out),
        Value::Arr(items) if items.is_empty() => out.push_str("[]"),
        Value::Arr(items) => {
            out.push_str("[\n");
            for (at, item) in items.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                write(item, depth + 1, out);
                out.push_str(if at + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push(']');
        }
        Value::Obj(object) if object.members.is_empty() => out.push_str("{}"),
        Value::Obj(object) => {
            out.push_str("{\n");
            for (at, (key, item)) in object.members.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                quote(key, out);
                out.push_str(": ");
                write(item, depth + 1, out);
                out.push_str(if at + 1 < object.members.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push('}');
        }
    }
}

/// Python's `json.dumps(value, indent=2)`.
pub fn dumps(value: &Value) -> String {
    let mut out = String::new();
    write(value, 0, &mut out);
    out
}

/// An integer stays as written (without a minus on zero); a float prints as Python's `repr`.
fn normal_number(token: &str) -> String {
    if !token.contains(['.', 'e', 'E']) {
        return if token == "-0" { "0".to_string() } else { token.to_string() };
    }
    let number: f64 = token.parse().unwrap_or(0.0);
    if number.is_infinite() {
        return if number < 0.0 { "-Infinity" } else { "Infinity" }.to_string();
    }
    let scientific = format!("{number:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if (-4..16).contains(&exponent) {
        let plain = format!("{number}");
        return if plain.contains('.') { plain } else { format!("{plain}.0") };
    }
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.abs())
}
