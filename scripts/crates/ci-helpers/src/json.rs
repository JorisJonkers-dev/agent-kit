//! A strict JSON reader for what upstream writes: it refuses a duplicate key
//! (Claude Code might read it differently from this check) and keeps the order
//! keys were written in, so a report lists things as the file does.

use serde::de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

struct Reader;

impl<'de> Visitor<'de> for Reader {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_unit<E: Error>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_bool<E: Error>(self, v: bool) -> Result<Json, E> {
        Ok(Json::Bool(v))
    }

    fn visit_i64<E: Error>(self, v: i64) -> Result<Json, E> {
        Ok(Json::Num(v.to_string()))
    }

    fn visit_u64<E: Error>(self, v: u64) -> Result<Json, E> {
        Ok(Json::Num(v.to_string()))
    }

    fn visit_f64<E: Error>(self, v: f64) -> Result<Json, E> {
        Ok(Json::Num(v.to_string()))
    }

    fn visit_str<E: Error>(self, v: &str) -> Result<Json, E> {
        Ok(Json::Str(v.to_string()))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Arr(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut entries: Vec<(String, Json)> = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, Json>()? {
            if entries.iter().any(|(seen, _)| *seen == key) {
                return Err(A::Error::custom(format!("duplicate key '{key}'")));
            }
            entries.push((key, value));
        }
        Ok(Json::Obj(entries))
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Json, D::Error> {
        d.deserialize_any(Reader)
    }
}

/// Parse `text`, or say why it cannot be read.
pub fn parse(text: &str) -> Result<Json, String> {
    serde_json::from_str::<Json>(text).map_err(|error| error.to_string())
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(text) => Some(text),
            _ => None,
        }
    }

    /// Python's truth of the value: null, false, zero and empty are false.
    pub fn truthy(&self) -> bool {
        match self {
            Json::Null => false,
            Json::Bool(flag) => *flag,
            Json::Num(text) => text.parse::<f64>() != Ok(0.0),
            Json::Str(text) => !text.is_empty(),
            Json::Arr(items) => !items.is_empty(),
            Json::Obj(entries) => !entries.is_empty(),
        }
    }

    /// The value as text a person reads: a string as it is, anything else as JSON.
    pub fn show(&self) -> String {
        match self {
            Json::Str(text) => text.clone(),
            other => other.canonical(),
        }
    }

    /// A stable text for the value, keys sorted: what two equal values share.
    pub fn canonical(&self) -> String {
        match self {
            Json::Null => "null".to_string(),
            Json::Bool(flag) => flag.to_string(),
            Json::Num(text) => text.clone(),
            Json::Str(text) => format!("{text:?}"),
            Json::Arr(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(Json::canonical)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Json::Obj(entries) => {
                let mut sorted: Vec<&(String, Json)> = entries.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                let body: Vec<String> = sorted
                    .iter()
                    .map(|(key, value)| format!("{key:?}: {}", value.canonical()))
                    .collect();
                format!("{{{}}}", body.join(", "))
            }
        }
    }
}

#[cfg(test)]
pub mod testing {
    /// The reader, so a test can ask what it says it expects.
    pub fn reader() -> impl for<'de> serde::de::Visitor<'de, Value = super::Json> {
        super::Reader
    }
}
