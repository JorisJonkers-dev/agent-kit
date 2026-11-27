//! OpenCode's own `opencode-go/<id>` provider names no LiteLLM proxy, and the gateway's sandbox allows only the
//! proxy a route names, so a run on it could never reach its models. The same models are served, budgeted, by
//! LiteLLM's `litellm-opencode-go/<id>` route, which OpenCode reaches through the proxy. A run that asks for the
//! direct name is run on the LiteLLM one when this machine offers it, and refused with the LiteLLM name to use
//! when it does not (#964).

use crate::error::GatewayError;
use crate::json::{self, Value};
use crate::models::ClientListing;
use crate::ports::Runtime;
use crate::run::RunInput;
use crate::sources::list_client;

/// OpenCode's own Go provider, which the sandbox cannot reach.
pub const DIRECT_PROVIDER: &str = "opencode-go";

/// The LiteLLM route of the same models, which OpenCode reaches through the proxy.
pub const LITELLM_PROVIDER: &str = "litellm-opencode-go";

/// A direct `opencode-go/<id>` model run on the LiteLLM route of the same model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewrite {
    pub from: String,
    pub to: String,
}

impl Rewrite {
    /// What the run's events and result say of the rewrite.
    pub fn to_json(&self) -> Value {
        json::object(vec![("from", Value::str(&self.from)), ("to", Value::str(&self.to)), ("why", Value::str(&self.why()))])
    }

    /// Why the model was rewritten, in one line.
    pub fn why(&self) -> String {
        format!("{DIRECT_PROVIDER} names no LiteLLM proxy address, which the sandbox requires; {} serves the same model through the proxy, budgeted", self.to)
    }
}

/// The LiteLLM name of a direct OpenCode Go model; `None` for any other model.
pub fn litellm_name(model: &str) -> Option<String> {
    model.strip_prefix(&format!("{DIRECT_PROVIDER}/")).filter(|id| !id.is_empty()).map(|id| format!("{LITELLM_PROVIDER}/{id}"))
}

/// Whether OpenCode's own listing on this machine names the model.
pub fn opencode_offers(runtime: &Runtime, model: &str) -> bool {
    match list_client(runtime, "opencode") {
        Some(ClientListing::Models(listed)) => listed.iter().any(|offered| offered.model == model),
        _ => false,
    }
}

/// The run on the LiteLLM route when it asks for OpenCode's direct Go provider, with what was rewritten; the
/// input untouched (`None`) for any other run. A direct model LiteLLM has no copy of on this machine is refused,
/// naming the LiteLLM model to use.
pub fn reroute(runtime: &Runtime, input: &RunInput) -> Result<Option<(RunInput, Rewrite)>, GatewayError> {
    if input.client != "opencode" {
        return Ok(None);
    }
    let Some(target) = litellm_name(&input.model) else {
        return Ok(None);
    };
    if !opencode_offers(runtime, &target) {
        return Err(GatewayError::new(
            "provider-not-allowed",
            format!(
                "{}: OpenCode's own {DIRECT_PROVIDER} provider names no LiteLLM proxy address, which the gateway's sandbox requires, and this machine's OpenCode lists no {target}; name {target} once LiteLLM serves it, or another model `models` lists on {LITELLM_PROVIDER}",
                input.model
            ),
        ));
    }
    let rewrite = Rewrite { from: input.model.clone(), to: target.clone() };
    Ok(Some((RunInput { model: target, ..input.clone() }, rewrite)))
}
