//! The routing a run uses: the routing agent-kit ships, with this machine's
//! override applied key by key when there is one. The node runner's
//! `application/routing.ts`.

use crate::error::CritiqueError;
use crate::merge_patch::merge_patch;
use crate::routing::{parse_routing, Routing};
use crate::runtime::Runtime;
use model_gateway::json::{self, Value};
use model_gateway::text::join;

/// The routing a run uses: the shipped routing, with this machine's override applied key by key when there is one.
/// The override is a JSON merge patch, so it names only what it changes, and `null` removes a key. The merged
/// routing is checked as a whole, so an override cannot leave it malformed.
pub fn load_routing(runtime: &Runtime) -> Result<Routing, CritiqueError> {
    let shipped_path = join(&[&runtime.assets_root, "routing.json"]);
    let shipped = parse_json(&shipped_path, model_gateway::SHIPPED_ROUTING)?;
    let override_path = override_path(runtime)?;
    // Node treats any failure to look as "not there"; the read that follows would say what is wrong.
    if !runtime.files.exists(&override_path).unwrap_or(false) {
        return reread(&shipped_path, &shipped);
    }
    let text = runtime.files.read_text(&override_path).map_err(CritiqueError::from_fs)?;
    let patch = parse_json(&override_path, &text)?;
    if patch.as_object().is_none() {
        return Err(invalid(&override_path, "the override must be a JSON object"));
    }
    let merged = merge_patch(&shipped, &patch);
    let place = format!("{shipped_path} with the override {override_path}");
    reread(&place, &merged)
}

/// The routing a value holds, read as the node runner reads it: from the JSON text the value prints as.
fn reread(place: &str, value: &Value) -> Result<Routing, CritiqueError> {
    let text = json::compact(value);
    parse_routing(&parse_json(place, &text)?).map_err(|error| error.into_error("invalid-routing", place))
}

/// The JSON a file holds; text that is not JSON is the message V8 gives, under the place the file is named by.
fn parse_json(place: &str, text: &str) -> Result<Value, CritiqueError> {
    json::parse(text).map_err(|message| invalid(place, &message))
}

/// The error a file that does not parse earns: the place, then the message.
fn invalid(place: &str, message: &str) -> CritiqueError {
    CritiqueError::new("invalid-routing", format!("{place}: {message}"))
}

/// This machine's routing override, resolved the way the model gateway resolves it.
fn override_path(runtime: &Runtime) -> Result<String, CritiqueError> {
    model_gateway::locations::routing_override_path(&runtime.env).map_err(|error| CritiqueError::new(&error.code, error.message))
}
