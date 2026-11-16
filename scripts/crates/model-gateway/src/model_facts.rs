//! What `models` says of each model beyond its place: what it costs and which provider serves it. Both are read
//! from the catalogue and the defaults, never guessed; a price nobody knows says `"unknown"`.

use crate::config::GatewayConfig;
use crate::json::{object, Value};
use crate::litellm_catalogue::{CatalogueDeployment, LiteLlmCatalogue};
use crate::models::open_code_provider;
use crate::run::is_pi_client;

/// The word for a price the catalogue and the defaults do not give.
const UNKNOWN: &str = "unknown";

/// The deployments that serve a Pi model, in fallback order (lowest `order` first, the catalogue's order among equals).
fn deployments<'a>(catalogue: Option<&'a LiteLlmCatalogue>, model: &str) -> Vec<&'a CatalogueDeployment> {
    let mut found: Vec<&CatalogueDeployment> = catalogue.and_then(|catalogue| catalogue.groups.get(model)).map(|group| group.deployments.iter().collect()).unwrap_or_default();
    found.sort_by_key(|deployment| deployment.order);
    found
}

/// The input and output price per million tokens: zero for a free model, else the defaults' price for the model or,
/// for a Pi model, for the model one of its deployments serves (a Pi group is named differently from the client
/// model behind it); `None` when nobody states one.
fn prices(config: &GatewayConfig, deployments: &[&CatalogueDeployment], client: &str, model: &str, free: bool) -> Option<(f64, f64)> {
    if free {
        return Some((0.0, 0.0));
    }
    let named = config.models.iter().find(|entry| entry.client == client && entry.all_names().contains(&model)).and_then(|entry| entry.prices.as_ref());
    let served = || {
        deployments.iter().find_map(|deployment| {
            config.models.iter().find(|entry| entry.all_names().contains(&deployment.upstream.as_str())).and_then(|entry| entry.prices.as_ref())
        })
    };
    named.or_else(served).map(|prices| (prices.input, prices.output))
}

fn price(value: Option<f64>) -> Value {
    value.map_or_else(|| Value::str(UNKNOWN), Value::Num)
}

/// One listed model's `cost` and `providers` members.
pub fn facts(config: &GatewayConfig, catalogue: Option<&LiteLlmCatalogue>, client: &str, model: &str, cost_class: &str, free: bool) -> [(&'static str, Value); 2] {
    let pi = is_pi_client(client);
    let served = if pi { deployments(catalogue, model) } else { Vec::new() };
    let known = prices(config, &served, client, model, free);
    let cost = object(vec![
        ("class", Value::str(cost_class)),
        ("input_per_mtok", price(known.map(|(input, _)| input))),
        ("output_per_mtok", price(known.map(|(_, output)| output))),
    ]);
    let route = if pi {
        client.strip_prefix("pi-").unwrap_or(client).to_string()
    } else {
        config.clients.get(client).and_then(|config| config.litellm_route.clone()).unwrap_or_else(|| client.to_string())
    };
    let providers: Vec<Value> = if pi {
        served
            .iter()
            .map(|deployment| {
                object(vec![
                    ("provider", Value::str(&deployment.route)),
                    ("upstream_id", Value::str(&deployment.upstream)),
                    ("route", Value::str(&route)),
                ])
            })
            .collect()
    } else {
        let provider = open_code_provider(model).unwrap_or_else(|| route.clone());
        vec![object(vec![("provider", Value::str(&provider)), ("upstream_id", Value::str(model)), ("route", Value::str(&route))])]
    };
    [("cost", cost), ("providers", Value::Arr(providers))]
}

/// The `models` result with `cost` and `providers` added to every model it lists.
pub fn annotate(config: &GatewayConfig, catalogue: Option<&LiteLlmCatalogue>, mut result: Value) -> Value {
    if let Value::Obj(members) = &mut result {
        if let Some(Value::Arr(models)) = members.get_mut("models") {
            for model in models.iter_mut() {
                let Value::Obj(entry) = model else {
                    continue;
                };
                let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
                let free = matches!(entry.get("free"), Some(Value::Bool(true)));
                let (client, name, class) = (text("client"), text("model"), text("cost_class"));
                for (key, value) in facts(config, catalogue, &client, &name, &class, free) {
                    entry.set(key, value);
                }
            }
        }
    }
    result
}
