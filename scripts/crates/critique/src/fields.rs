//! Reads the fields of one JSON object. Every reader checks the field's type and
//! names its path when the type is wrong, so nothing read from a command's input
//! or a file is trusted: the node runner's `JsonFields`.

use crate::error::ShapeError;
use model_gateway::json::{Object, Value};
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
}
