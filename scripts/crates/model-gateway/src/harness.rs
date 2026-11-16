//! `run --profile <role> --harness <h>`: the harness is the client a role runs on, and the gateway picks the
//! model. Pi is the default harness of every role, and the only one whose tool scope the gateway enforces;
//! `claude`, `codex`, `agy`, `vibe` and `hermes` name themselves, and `pi-<route>` names a Pi route (OpenCode Go
//! through Pi is `pi-opencode-go`). The model is the cheapest the profile leaves on that harness for the
//! repository, so the data policy, the writer family and the narrowing apply exactly as for any profile run.

use crate::commands::{available_models, gather};
use crate::config::DelegationProfile;
use crate::error::GatewayError;
use crate::json::{Object, Value};
use crate::ports::Runtime;
use crate::profiles::{read_narrowing, require_narrowing, require_profile, resolve_profile, Narrowing, ProfileModel};

/// Whether a client is the harness the caller named: `pi` is every Pi route, anything else is that client alone.
pub fn on_harness(client: &str, harness: &str) -> bool {
    if harness == "pi" {
        crate::run::is_pi_client(client)
    } else {
        client == harness
    }
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_input(message)
}

/// The run input with `client` and `model` filled in from `harness`, or the input untouched when it names none.
pub fn choose(input: &Object, runtime: &Runtime) -> Result<Object, GatewayError> {
    let Some(raw) = input.get("harness") else {
        return Ok(input.clone());
    };
    let harness = match raw {
        Value::Str(text) if !text.is_empty() => text.as_str(),
        _ => return Err(invalid("harness must be a non-empty string")),
    };
    let Some(Value::Str(profile)) = input.get("profile") else {
        return Err(invalid("harness needs a profile: it picks the profile's model on that harness"));
    };
    if input.get("client").is_some() || input.get("model").is_some() {
        return Err(invalid("harness picks the client and the model: name harness or client and model, not both"));
    }
    let narrow = read_narrowing(input.get("narrow"))?;
    let gathered = gather(input, runtime, false, false)?;
    let found = require_profile(&gathered.config, profile)?;
    require_narrowing(found, narrow.as_ref())?;
    let chosen = pick(&available_models(&gathered.permitted), &gathered.policy_reason, harness, narrow.as_ref(), found)?;
    let mut object = input.clone();
    object.remove("harness");
    object.set("client", Value::str(&chosen.client));
    object.set("model", Value::str(&chosen.model));
    Ok(object)
}

fn pick(available: &[ProfileModel], reason: &str, harness: &str, narrow: Option<&Narrowing>, profile: &DelegationProfile) -> Result<ProfileModel, GatewayError> {
    let name = &profile.name;
    let resolution = resolve_profile(profile, available, reason, narrow, true);
    if let Some(error) = resolution.error {
        return Err(GatewayError::new("profile-refused", error));
    }
    let left: Vec<String> = resolution.models.iter().map(|model| format!("{}:{}", model.client, model.model)).collect();
    resolution.models.into_iter().find(|model| on_harness(&model.client, harness)).ok_or_else(|| {
        GatewayError::new(
            "profile-refused",
            format!(
                "profile {name} has no usable model on harness {harness} here (a harness with no model configured, such as hermes, has none): the profile and the data policy leave {}",
                left.join(", ")
            ),
        )
    })
}
