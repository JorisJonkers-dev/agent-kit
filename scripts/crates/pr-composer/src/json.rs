//! An insertion-ordered JSON value and the writer for `json.dump(x, indent=2)`
//! as Python writes it: keys in insertion order, non-ASCII escaped, floats as
//! `repr`.

use crate::text::float_repr;

#[derive(PartialEq)]
pub enum J {
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn s(text: &str) -> J {
        J::Str(text.to_string())
    }

    pub fn obj(items: Vec<(&str, J)>) -> J {
        J::Obj(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn get(&self, key: &str) -> &J {
        match self {
            J::Obj(items) => items
                .iter()
                .find(|(k, _)| k == key)
                .map_or(&J::Null, |(_, v)| v),
            _ => &J::Null,
        }
    }

    pub fn list(&self) -> &[J] {
        match self {
            J::Arr(items) => items,
            _ => &[],
        }
    }

    pub fn entries(&self) -> &[(String, J)] {
        match self {
            J::Obj(items) => items,
            _ => &[],
        }
    }

    pub fn number(&self) -> f64 {
        match self {
            J::Int(n) => *n as f64,
            J::Float(f) => *f,
            _ => 0.0,
        }
    }

    /// The text an f-string prints for the value.
    pub fn show(&self) -> String {
        match self {
            J::Null => "None".to_string(),
            J::Int(n) => n.to_string(),
            J::Float(f) => float_repr(*f),
            J::Str(s) => s.clone(),
            J::Arr(_) | J::Obj(_) => self.dump(),
        }
    }

    pub fn is_empty_list(&self) -> bool {
        self.list().is_empty()
    }

    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        match self {
            J::Null => out.push_str("null"),
            J::Int(n) => out.push_str(&n.to_string()),
            J::Float(f) => out.push_str(&float_repr(*f)),
            J::Str(s) => quote(s, out),
            J::Arr(items) if items.is_empty() => out.push_str("[]"),
            J::Obj(items) if items.is_empty() => out.push_str("{}"),
            J::Arr(items) => {
                let pad = "  ".repeat(depth + 1);
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(if i == 0 { "" } else { ",\n" });
                    out.push_str(&pad);
                    item.write(out, depth + 1);
                }
                out.push('\n');
                out.push_str(&"  ".repeat(depth));
                out.push(']');
            }
            J::Obj(items) => {
                let pad = "  ".repeat(depth + 1);
                out.push_str("{\n");
                for (i, (key, value)) in items.iter().enumerate() {
                    out.push_str(if i == 0 { "" } else { ",\n" });
                    out.push_str(&pad);
                    quote(key, out);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                }
                out.push('\n');
                out.push_str(&"  ".repeat(depth));
                out.push('}');
            }
        }
    }
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (' '..='~').contains(&c) => out.push(c),
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
