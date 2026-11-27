//! What the sbx backend reads of Vibe's route. Vibe's run home holds a `config.toml` whose `[[models]]` name a
//! provider and whose `[[providers]]` may carry an `api_base` (a LiteLLM route). A model whose provider has none is
//! Mistral's own, which Vibe reaches at `api.mistral.ai`, so that host is allowed for it explicitly.

use crate::error::GatewayError;
use crate::json::Value;
use crate::sbx_launch::{host_rules, HOST_FROM_SANDBOX};
use crate::toml::read_toml;

/// Where Vibe talks to Mistral when the model's provider names no base URL.
const MISTRAL_BASE: &str = "https://api.mistral.ai";

/// The model a Vibe launch names: the word after `--model`, or none when an agent file picks it.
fn model_of(args: &[String]) -> Option<&str> {
    args.iter().position(|word| word == "--model").and_then(|at| args.get(at + 1)).map(String::as_str)
}

/// The `api_base` of the provider of a model, `None` when the model or its provider names none (a direct Mistral run).
fn base_of(config: &Value, model: Option<&str>) -> Option<String> {
    let model = model?;
    let models = config.as_object()?.get("models")?;
    let Value::Arr(models) = models else { return None };
    let provider = models.iter().filter_map(Value::as_object).find(|entry| ["alias", "name"].iter().any(|key| entry.get(key).and_then(Value::as_str) == Some(model)))?.get("provider")?.as_str()?;
    providers(config).into_iter().find(|(name, _)| name == provider).and_then(|(_, base)| base)
}

/// Every `[[providers]]` entry's name and its `api_base`, when it has one.
fn providers(config: &Value) -> Vec<(String, Option<String>)> {
    let Some(Value::Arr(entries)) = config.as_object().and_then(|object| object.get("providers")) else { return Vec::new() };
    entries.iter().filter_map(Value::as_object).filter_map(|entry| Some((entry.get("name")?.as_str()?.to_string(), entry.get("api_base").and_then(Value::as_str).map(str::to_string)))).collect()
}

/// The `host:port` rules a Vibe launch needs: its LiteLLM route's host (a loopback proxy by both names), or
/// `api.mistral.ai:443` for a model on Mistral's own provider. A config that does not parse is refused.
pub fn allowed(route_text: &str, args: &[String]) -> Result<Vec<String>, GatewayError> {
    let config = read_toml(route_text).map_err(|reason| GatewayError::new("sandbox-unsupported", format!("vibe: the run home's config.toml does not parse: {reason}")))?;
    let base = base_of(&config, model_of(args)).unwrap_or_else(|| MISTRAL_BASE.to_string());
    host_rules(&base)
}

/// The config with every loopback `api_base` rewritten to the host name the sandbox sees; `None` when none is.
pub fn rewritten(route_text: &str) -> Option<String> {
    let config = read_toml(route_text).ok()?;
    let mut text = route_text.to_string();
    for base in providers(&config).into_iter().filter_map(|(_, base)| base) {
        let host = base.split_once("://").and_then(|(_, rest)| rest.split(['/', ':']).next()).unwrap_or_default();
        if host == "localhost" || host == "127.0.0.1" {
            text = text.replace(&base, &base.replacen(host, HOST_FROM_SANDBOX, 1));
        }
    }
    (text != route_text).then_some(text)
}
