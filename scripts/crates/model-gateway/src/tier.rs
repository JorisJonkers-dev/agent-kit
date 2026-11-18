//! `run --tier <tier>`: a LiteLLM tier is a callable model name on the proxy, and the defaults state one delegation
//! profile per tier (rendered from the tier snapshot, which is litellm's policy; agent-kit has none of its own). The
//! tier names that profile and its Pi route, so the run is `--profile <tier> --harness pi`: the Pi client sends the
//! tier's name as the model, with the key of the route that holds the tier. A tier the defaults do not state is an
//! error, never a default.

use crate::config_parse::load_config;
use crate::error::GatewayError;
use crate::json::{Object, Value};
use crate::ports::Runtime;

/// The run input with `profile` and `harness` filled in from `tier`, or the input untouched when it names none.
pub fn choose(input: &Object, runtime: &Runtime) -> Result<Object, GatewayError> {
    let Some(raw) = input.get("tier") else {
        return Ok(input.clone());
    };
    let tier = match raw {
        Value::Str(text) if !text.is_empty() => text.as_str(),
        _ => return Err(GatewayError::invalid_input("tier must be a non-empty string")),
    };
    if ["profile", "harness", "client", "model"].iter().any(|name| input.get(name).is_some()) {
        return Err(GatewayError::invalid_input("tier picks the profile, the client and the model: name tier alone, not with profile, harness, client or model"));
    }
    let config = load_config(runtime)?;
    let named = config.profiles.values().find(|profile| profile.tier.as_deref() == Some(tier));
    let Some(profile) = named else {
        let known: Vec<&str> = config.profiles.values().filter_map(|profile| profile.tier.as_deref()).collect();
        let known = if known.is_empty() { "none".to_string() } else { known.join(", ") };
        return Err(GatewayError::new("unknown-tier", format!("no tier {tier}: the defaults state {known}")));
    };
    let mut object = input.clone();
    object.remove("tier");
    object.set("profile", Value::str(&profile.name));
    object.set("harness", Value::str("pi"));
    Ok(object)
}
