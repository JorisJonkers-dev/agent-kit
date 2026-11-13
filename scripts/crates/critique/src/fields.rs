//! Reads the fields of one JSON object. Every reader checks the field's type and
//! names its path when the type is wrong, so nothing read from a command's input
//! or a file is trusted: the node runner's `JsonFields`.

use crate::error::ShapeError;
use model_gateway::json::{self, Object, Value};
use model_gateway::text::js_trim;

pub struct Fields<'a> {
    object: &'a Object,
    path: String,
}

fn ensure<T>(value: Option<T>, path: &str, expected: &str) -> Result<T, ShapeError> {
    value.ok_or_else(|| ShapeError(format!("{path} must be {expected}")))
}

impl<'a> Fields<'a> {
    /// Reads `value` as an object; the root's path is empty and is called "the value".
    pub fn of(value: &'a Value, path: &str) -> Result<Fields<'a>, ShapeError> {
        let named = if path.is_empty() { "the value" } else { path };
        let object = ensure(value.as_object(), named, "a JSON object")?;
        Ok(Fields { object, path: path.to_string() })
    }

    fn at(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_string()
        } else {
            format!("{}.{key}", self.path)
        }
    }

    /// Whether the key is there; a `null` counts, as it does for the node reader.
    pub fn has(&self, key: &str) -> bool {
        self.object.get(key).is_some()
    }

    /// A string with at least one character that is not white space.
    pub fn string(&self, key: &str) -> Result<String, ShapeError> {
        let text = self.object.get(key).and_then(Value::as_str).filter(|text| !js_trim(text).is_empty());
        ensure(text, &self.at(key), "a non-empty string").map(str::to_string)
    }

    pub fn boolean(&self, key: &str) -> Result<bool, ShapeError> {
        let flag = match self.object.get(key) {
            Some(Value::Bool(flag)) => Some(*flag),
            _ => None,
        };
        ensure(flag, &self.at(key), "true or false")
    }

    /// The number the key must equal.
    pub fn literal(&self, key: &str, expected: f64) -> Result<(), ShapeError> {
        let found = matches!(self.object.get(key), Some(Value::Num(number)) if *number == expected);
        ensure(found.then_some(()), &self.at(key), &model_gateway::json::number_text(expected))
    }

    fn items(&self, key: &str) -> Result<&'a [Value], ShapeError> {
        let list = match self.object.get(key) {
            Some(Value::Arr(items)) => Some(items.as_slice()),
            _ => None,
        };
        ensure(list, &self.at(key), "a list")
    }

    /// A list whose every item is one of `allowed`.
    pub fn one_of_list(&self, key: &str, allowed: &[&str]) -> Result<Vec<String>, ShapeError> {
        let expected = format!("one of {}", allowed.join(", "));
        self.items(key)?
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let found = item.as_str().filter(|text| allowed.contains(text));
                ensure(found, &format!("{}[{index}]", self.at(key)), &expected).map(str::to_string)
            })
            .collect()
    }

    pub fn object(&self, key: &str) -> Result<Fields<'a>, ShapeError> {
        let value = self.object.get(key).unwrap_or(&Value::Null);
        Fields::of(value, &self.at(key))
    }

    /// An object used as a map: every value read with `read`, under its key, in the order JavaScript enumerates them.
    pub fn entries<T>(&self, key: &str, mut read: impl FnMut(&Fields<'a>, &str) -> Result<T, ShapeError>) -> Result<Vec<(String, T)>, ShapeError> {
        let map = self.object(key)?;
        map.object
            .entries()
            .into_iter()
            .map(|(name, _)| read(&map.object(name)?, name).map(|item| (name.to_string(), item)))
            .collect()
    }

    /// A non-empty string when the key is there.
    pub fn optional_string(&self, key: &str) -> Result<Option<String>, ShapeError> {
        if self.has(key) { self.string(key).map(Some) } else { Ok(None) }
    }

    /// A finite number.
    pub fn number(&self, key: &str) -> Result<f64, ShapeError> {
        let number = match self.object.get(key) {
            Some(Value::Num(number)) if number.is_finite() => Some(*number),
            _ => None,
        };
        ensure(number, &self.at(key), "a number")
    }

    /// A whole number of at least one.
    pub fn positive_integer(&self, key: &str) -> Result<f64, ShapeError> {
        let number = self.number(key)?;
        ensure((number.fract() == 0.0 && number >= 1.0).then_some(number), &self.at(key), "a whole number of at least 1")
    }

    /// The string the key must equal.
    pub fn literal_str(&self, key: &str, expected: &str) -> Result<String, ShapeError> {
        let found = matches!(self.object.get(key), Some(Value::Str(text)) if text == expected);
        ensure(found.then(|| expected.to_string()), &self.at(key), &json::compact(&Value::str(expected)))
    }

    pub fn one_of(&self, key: &str, allowed: &[&str]) -> Result<String, ShapeError> {
        let found = self.object.get(key).and_then(Value::as_str).filter(|text| allowed.contains(text));
        ensure(found, &self.at(key), &format!("one of {}", allowed.join(", "))).map(str::to_string)
    }

    /// A non-empty string `accept` recognises.
    pub fn matching(&self, key: &str, accept: impl Fn(&str) -> bool, expected: &str) -> Result<String, ShapeError> {
        let text = self.string(key)?;
        ensure(accept(&text).then_some(()), &self.at(key), expected)?;
        Ok(text)
    }

    pub fn strings(&self, key: &str) -> Result<Vec<String>, ShapeError> {
        self.items(key)?
            .iter()
            .enumerate()
            .map(|(index, item)| ensure(item.as_str(), &format!("{}[{index}]", self.at(key)), "a string").map(str::to_string))
            .collect()
    }

    /// A list whose items are each read as an object with `read`, under the item's path.
    pub fn list<T>(&self, key: &str, mut read: impl FnMut(&Fields<'a>) -> Result<T, ShapeError>) -> Result<Vec<T>, ShapeError> {
        let path = self.at(key);
        self.items(key)?.iter().enumerate().map(|(index, item)| read(&Fields::of(item, &format!("{path}[{index}]"))?)).collect()
    }

    /// An object whose every value is a list of strings, in the order JavaScript enumerates them.
    pub fn string_lists(&self, key: &str) -> Result<Vec<(String, Vec<String>)>, ShapeError> {
        self.each(key, |map, name| map.strings(name))
    }

    /// An object whose every value is a number.
    pub fn number_map(&self, key: &str) -> Result<Vec<(String, f64)>, ShapeError> {
        self.each(key, |map, name| map.number(name))
    }

    /// An object's members, each read by `read` from the object itself under its name.
    fn each<T>(&self, key: &str, read: impl Fn(&Fields<'a>, &str) -> Result<T, ShapeError>) -> Result<Vec<(String, T)>, ShapeError> {
        let map = self.object(key)?;
        map.object.entries().into_iter().map(|(name, _)| read(&map, name).map(|item| (name.to_string(), item))).collect()
    }
}
