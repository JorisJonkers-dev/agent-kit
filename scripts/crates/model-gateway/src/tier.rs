//! `run --tier <tier>`: a LiteLLM tier is a callable model name on the proxy, and the defaults state one delegation
//! profile per tier (rendered from the tier snapshot, which is litellm's policy; agent-kit has none of its own). The
//! tier names that profile and its Pi route, so the run is `--profile <tier> --harness pi`: the Pi client sends the
//! tier's name as the model, with the key of the route that holds the tier. A tier the defaults do not state is an
//! error, never a default.

use crate::config_parse::load_config;
use crate::error::GatewayError;
use crate::config::DelegationProfile;
use crate::json::{self, Object, Value};
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
    if let Some(missing) = missing_provider(runtime, profile) {
        let needs = profile.key_models.as_ref().map_or(String::new(), |keys| format!(" holding {}", keys.join(", ")));
        return Err(GatewayError::new(
            "tier-key-missing",
            format!("no LiteLLM key for {tier} on this machine: Pi's models.json has no {missing} provider, and the tier needs a key{needs}; a tier no workstation key calls (an escalation tier) is run only once such a key is provisioned"),
        ));
    }
    let mut object = input.clone();
    object.remove("tier");
    object.set("profile", Value::str(&profile.name));
    object.set("harness", Value::str("pi"));
    Ok(object)
}

/// The Pi provider of the tier's route when Pi's `models.json` is readable and does not have it: no key for the
/// tier was ever written here. An unreadable file says nothing, and the launch reports its own refusal.
fn missing_provider(runtime: &Runtime, profile: &DelegationProfile) -> Option<String> {
    let client = profile.models.first()?.split(':').next()?;
    let provider = crate::run::pi_provider(client);
    let path = crate::text::join(&[&crate::pi_launch::home_agent_dir(&runtime.env), "models.json"]);
    let parsed = json::parse(&runtime.files.read_text(&path).ok()?).ok()?;
    let providers = parsed.as_object()?.get("providers")?.as_object()?;
    providers.get(&provider).is_none().then_some(provider)
}
