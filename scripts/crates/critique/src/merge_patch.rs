//! A JSON merge patch (RFC 7386) as the node runner applies it: an object merges
//! into an object key by key, `null` removes the key, and any other value (a list
//! included) replaces what was there.

use model_gateway::json::{Object, Value};

/// Applies `patch` to `target` the way a JSON merge patch does; the keys keep the order JavaScript enumerates them in.
pub fn merge_patch(target: &Value, patch: &Value) -> Value {
    let patch_object = match patch.as_object() {
        Some(object) => object,
        None => return patch.clone(),
    };
    let base = target.as_object();
    let mut merged = Object::new();
    // The base's keys first, in the order JavaScript enumerates them, then the patch's own.
    for (key, previous) in base.map(|object| object.entries()).unwrap_or_default() {
        match patch_object.get(key) {
            Some(Value::Null) => {}
            Some(value) => merged.set(key, merge_patch(previous, value)),
            None => merged.set(key, previous.clone()),
        }
    }
    for (key, value) in patch_object.entries() {
        if !matches!(value, Value::Null) && base.map_or(true, |object| object.get(key).is_none()) {
            merged.set(key, merge_patch(&Value::Null, value));
        }
    }
    Value::Obj(merged)
}
