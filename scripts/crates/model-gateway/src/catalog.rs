//! The catalog view: one row per canonical model the proxy serves, answering "what can I run, where, and for how
//! much". A row says what the model is (modality, family, tier, context length, free), what it costs, which provider
//! serves it through which route and key in fallback order, whether each deployment's probe found it usable and the
//! provider's own reason when not, and which clients and delegation profiles can run it now. A model that cannot run
//! is a row with its reason, never a missing one.

use crate::commands::{self, Gathered};
use crate::config::Tier;
use crate::error::GatewayError;
use crate::json::{self, object, Object, Value};
use crate::litellm_catalogue::{deployment_probe, family_from, group_probe, names_of, tier_from, CatalogueGroup, LiteLlmCatalogue};
use crate::models::model_part;
use crate::model_facts;
use crate::ports::{ProcessCommand, Runtime};
use crate::run::is_pi_client;
use std::collections::BTreeSet;

/// The LiteLLM routes the estate mints keys for, with each key's monthly cap (`registry/generated/litellm/routes.json`).
const SHIPPED_ROUTES: &str = include_str!("../../../../registry/generated/litellm/routes.json");

/// The program that asks the proxy for a key's spend (`measure-litellm-budget`, the litellm-budget crate).
const BUDGET_PROGRAM: &str = "measure-litellm-budget";

/// How long the spend lookup may take before the pool's spend is `unknown`.
const BUDGET_TIMEOUT_MS: u64 = 15_000;

/// The status of a deployment the catalogue holds no probe for.
const UNPROBED: &str = "unprobed";

/// What a catalog call narrows the rows to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub route: Option<String>,
    pub free: bool,
    pub status: Option<String>,
    pub modality: Option<String>,
}

/// One deployment behind a model: the provider, the route and key that reach it, and what its probe found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentRow {
    pub order: i64,
    pub provider: String,
    pub route: String,
    pub key: Option<String>,
    pub upstream: String,
    pub status: String,
    pub reason: String,
    /// The LiteLLM key (its environment variable) whose budget pool the deployment draws on, when the route has one.
    pub pool: Option<String>,
}

/// A key's budget pool: what the registry caps it at, and what it has spent when the proxy says.
#[derive(Clone, Debug, PartialEq)]
pub struct Pool {
    pub key: String,
    pub route: String,
    /// The monthly cap in EUR; `None` for a key with no cap.
    pub cap_eur: Option<f64>,
    pub duration: Option<String>,
    /// The spend so far in the current window; `None` until the proxy has answered, which prints as `unknown`.
    pub spend_eur: Option<f64>,
}

/// One canonical model.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub model: String,
    pub modality: String,
    pub family: String,
    pub tier: Option<Tier>,
    pub context_length: Option<i64>,
    pub free: bool,
    pub cost: Value,
    pub status: String,
    pub reason: String,
    pub deployments: Vec<DeploymentRow>,
    pub clients: Vec<String>,
    pub profiles: Vec<String>,
}

/// The catalog: its rows and what surrounds them.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub policy: String,
    pub updated_at: Option<String>,
    pub warnings: Vec<String>,
    pub missing: Vec<(String, String)>,
    pub rows: Vec<Row>,
    /// The budget pool of every key a shown deployment draws on, by key.
    pub pools: Vec<Pool>,
}

/// A text filter: a non-empty string, or nothing.
fn read_text(input: &Object, key: &str) -> Result<Option<String>, GatewayError> {
    match input.get(key) {
        None => Ok(None),
        Some(Value::Str(text)) if !text.is_empty() => Ok(Some(text.clone())),
        Some(_) => Err(GatewayError::invalid_input(format!("{key} must be a non-empty string"))),
    }
}

/// The filter an input object holds.
pub fn read_filter(input: &Object) -> Result<Filter, GatewayError> {
    let free = match input.get("free") {
        None => false,
        Some(Value::Bool(free)) => *free,
        Some(_) => return Err(GatewayError::invalid_input("free must be true or false")),
    };
    Ok(Filter { route: read_text(input, "route")?, free, status: read_text(input, "status")?, modality: read_text(input, "modality")? })
}

/// The provider a LiteLLM route reaches, by the name the owner knows it by.
pub fn provider_name(route: &str) -> String {
    match route {
        "google" => "Google".to_string(),
        "anthropic" => "Anthropic".to_string(),
        "openrouter" => "OpenRouter".to_string(),
        "opencode-go" => "OpenCode Go".to_string(),
        "local" => "local".to_string(),
        route if route.starts_with("vibe") => "Mistral".to_string(),
        route => route.to_string(),
    }
}

/// Every name a group goes by: its own and the id each deployment serves it under.
fn group_names(group_name: &str, group: &CatalogueGroup) -> BTreeSet<String> {
    let mut names = BTreeSet::from([group_name.to_string()]);
    for deployment in &group.deployments {
        names.insert(deployment.upstream.clone());
        names.insert(model_part(&deployment.upstream).to_string());
    }
    names
}

/// The LiteLLM routes of the registry.
fn registry_routes() -> Vec<RegistryRoute> {
    parse_routes(SHIPPED_ROUTES)
}

/// One route of the registry: `(route, key variable, monthly cap, budget duration)`.
type RegistryRoute = (String, String, Option<f64>, Option<String>);

/// The routes a `routes.json` text names; text that is not that names none.
pub(crate) fn parse_routes(text: &str) -> Vec<RegistryRoute> {
    let Ok(Value::Obj(table)) = json::parse(text) else {
        return Vec::new();
    };
    let Some(Value::Arr(routes)) = table.get("routes") else {
        return Vec::new();
    };
    routes
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|route| {
            let text = |key: &str| route.get(key).and_then(Value::as_str).map(str::to_string);
            let cap = match route.get("budget_eur") {
                Some(Value::Num(cap)) => Some(*cap),
                _ => None,
            };
            Some((text("agent")?, text("key_env")?, cap, text("budget_duration")))
        })
        .collect()
}

/// The key a route's deployments are reached through on the proxy, when the registry mints one for the route.
fn route_key(route: &str) -> Option<String> {
    registry_routes().into_iter().find(|(agent, ..)| agent == route).map(|(_, key, ..)| key)
}

/// The pools of the keys the rows' deployments draw on, in the order the rows first name them. Spend is not read here.
pub(crate) fn pools_of(rows: &[Row]) -> Vec<Pool> {
    let routes = registry_routes();
    let mut pools: Vec<Pool> = Vec::new();
    for key in rows.iter().flat_map(|row| &row.deployments).filter_map(|deployment| deployment.pool.as_deref()) {
        if pools.iter().any(|pool| pool.key == key) {
            continue;
        }
        if let Some((route, key, cap_eur, duration)) = routes.iter().find(|(_, known, ..)| known == key) {
            pools.push(Pool { key: key.clone(), route: route.clone(), cap_eur: *cap_eur, duration: duration.clone(), spend_eur: None });
        }
    }
    pools
}

/// What the proxy says a route's key has spent: the `spend:` line of `measure-litellm-budget --agent ROUTE`, or
/// nothing when the program is not installed, the proxy is unreachable or the answer holds no number.
pub(crate) fn spend_of(runtime: &Runtime, route: &str) -> Option<f64> {
    runtime.process.resolve(BUDGET_PROGRAM)?;
    let mut command = ProcessCommand::new(BUDGET_PROGRAM, &["--agent", route]);
    command.timeout_ms = Some(BUDGET_TIMEOUT_MS);
    let done = runtime.process.run(&command).ok().filter(|done| done.exit_code == 0 && !done.timed_out)?;
    done.stdout.lines().find_map(|line| line.trim().strip_prefix("spend:")?.trim().parse::<f64>().ok())
}

/// A group's deployments in fallback order (lowest `order` first, the catalogue's order among equals).
fn deployment_rows(catalogue: &LiteLlmCatalogue, group: &CatalogueGroup) -> Vec<DeploymentRow> {
    let mut rows: Vec<DeploymentRow> = group
        .deployments
        .iter()
        .map(|deployment| {
            let probe = deployment_probe(catalogue, deployment);
            DeploymentRow {
                order: deployment.order,
                provider: provider_name(&deployment.route),
                route: deployment.route.clone(),
                key: catalogue.keys.get(&deployment.route).cloned(),
                upstream: deployment.upstream.clone(),
                status: probe.map_or_else(|| UNPROBED.to_string(), |probe| probe.status.clone()),
                reason: probe.map_or_else(|| "the catalogue holds no probe for it".to_string(), |probe| probe.reason.clone()),
                pool: route_key(&deployment.route),
            }
        })
        .collect();
    rows.sort_by_key(|row| row.order);
    rows
}

/// A model one client lists or skipped, to be joined to the row it belongs to.
struct Seen {
    client: String,
    model: String,
}

/// Whether a client's model is one of the names a row goes by.
fn belongs(config: &crate::config::GatewayConfig, seen: &Seen, names: &BTreeSet<String>) -> bool {
    if is_pi_client(&seen.client) {
        return names.contains(&seen.model);
    }
    names_of(config, &seen.client, &seen.model).iter().any(|name| names.contains(name)) || names.contains(model_part(&seen.model))
}

/// The names of the profiles that allow any of the clients' models of a row.
fn profiles_of(profiles: &Value, runs: &[(String, String)]) -> Vec<String> {
    let Value::Arr(profiles) = profiles else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for profile in profiles.iter().filter_map(Value::as_object) {
        let name = profile.get("name").and_then(Value::as_str).unwrap_or_default();
        let Some(Value::Arr(models)) = profile.get("models") else {
            continue;
        };
        let allows = models.iter().filter_map(Value::as_object).any(|model| {
            let text = |key: &str| model.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            runs.contains(&(text("client"), text("model")))
        });
        if allows {
            found.push(name.to_string());
        }
    }
    found.sort();
    found
}

/// What a row's own listings say: the clients that run it now, and why the ones that cannot do not.
#[derive(Default)]
struct Placed {
    runs: Vec<(String, String)>,
    listed: Vec<(String, String, String, bool, Option<Tier>, String)>,
    /// Each client that cannot run the model, with why.
    refused: Vec<(String, String)>,
}

fn place(gathered: &Gathered, names: &BTreeSet<String>) -> Placed {
    let config = &gathered.config;
    let mut placed = Placed::default();
    for permitted in &gathered.permitted.models {
        let listed = &permitted.listed;
        if belongs(config, &Seen { client: listed.client.clone(), model: listed.model.clone() }, names) {
            placed.runs.push((listed.client.clone(), listed.model.clone()));
            placed.listed.push((listed.client.clone(), listed.model.clone(), listed.cost_class.as_str().to_string(), listed.free, listed.tier, listed.family.clone()));
        }
    }
    for skipped in &gathered.permitted.skipped {
        if belongs(config, &Seen { client: skipped.client.clone(), model: skipped.model.clone() }, names) {
            placed.refused.push((skipped.client.clone(), skipped.reason.clone()));
        }
    }
    placed
}

/// The cost of a row: the class of the first client that runs it (a Pi client's, which is subsidised, when none
/// does), and the prices `models` gives it.
fn cost_of(gathered: &Gathered, placed: &Placed, name: &str, free: bool) -> Value {
    let (client, model, class) = match placed.listed.first() {
        Some((client, model, class, _, _, _)) => (client.clone(), model.clone(), class.clone()),
        None => ("pi-vibe".to_string(), name.to_string(), if free { "free" } else { "subsidised" }.to_string()),
    };
    let [(_, cost), _] = model_facts::facts(&gathered.config, gathered.reading.catalogue.as_ref(), &client, &model, &class, free);
    cost
}

/// The text of a reason line: what the provider said, then why clients cannot run it, one sentence per reason.
fn reason_of(provider: &str, refused: &[(String, String)]) -> String {
    let mut by_reason: Vec<(&str, Vec<&str>)> = Vec::new();
    for (client, reason) in refused {
        match by_reason.iter_mut().find(|(known, _)| known == reason) {
            Some((_, clients)) => clients.push(client),
            None => by_reason.push((reason, vec![client])),
        }
    }
    std::iter::once(provider.to_string())
        .filter(|reason| !reason.is_empty())
        .chain(by_reason.iter().map(|(reason, clients)| format!("{}: {reason}", clients.join(", "))))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The row of a canonical group.
fn group_row(gathered: &Gathered, catalogue: &LiteLlmCatalogue, name: &str, group: &CatalogueGroup) -> (Row, BTreeSet<String>) {
    let names = group_names(name, group);
    let placed = place(gathered, &names);
    let probe = group_probe(catalogue, group).map(|(_, probe)| probe);
    let modality = group.modality.clone().or_else(|| probe.as_ref().and_then(|probe| probe.modality.clone())).unwrap_or_else(|| "unknown".to_string());
    let canonical = probe.as_ref().and_then(|probe| probe.canonical.as_deref());
    let entry = gathered.config.models.iter().find(|entry| entry.all_names().iter().any(|known| names.contains(*known)));
    let family = placed
        .listed
        .first()
        .map(|(_, _, _, _, _, family)| family.clone())
        .or_else(|| entry.map(|entry| entry.family.clone()))
        .unwrap_or_else(|| family_from(canonical, name));
    let tier = placed.listed.first().and_then(|(_, _, _, _, tier, _)| *tier).or_else(|| entry.map(|entry| entry.tier)).or_else(|| {
        (modality == "chat").then(|| tier_from(canonical.unwrap_or(name)))
    });
    let free = probe.as_ref().is_some_and(|probe| probe.free) || placed.listed.iter().any(|(_, _, _, free, _, _)| *free);
    let status = probe.as_ref().map_or_else(|| UNPROBED.to_string(), |probe| probe.status.clone());
    let provider_reason = if status == "usable" { String::new() } else { probe.as_ref().map(|probe| probe.reason.clone()).unwrap_or_default() };
    let clients: Vec<String> = placed.runs.iter().map(|(client, _)| client.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    let row = Row {
        model: name.to_string(),
        modality,
        family,
        tier,
        context_length: probe.as_ref().and_then(|probe| probe.context_length),
        free,
        cost: cost_of(gathered, &placed, name, free),
        status,
        reason: reason_of(&provider_reason, &placed.refused),
        deployments: deployment_rows(catalogue, group),
        profiles: profiles_of(&gathered.profiles, &placed.runs),
        clients,
    };
    (row, names)
}

/// The rows of the models no catalogue group covers: every listed or skipped client model whose id no group goes by,
/// one row per model id.
fn loose_rows(gathered: &Gathered, covered: &BTreeSet<String>) -> Vec<Row> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let unmatched = |client: &str, model: &str| !belongs(&gathered.config, &Seen { client: client.to_string(), model: model.to_string() }, covered);
    ids.extend(gathered.permitted.models.iter().filter(|permitted| unmatched(&permitted.listed.client, &permitted.listed.model)).map(|permitted| permitted.listed.model.clone()));
    ids.extend(gathered.permitted.skipped.iter().filter(|skipped| unmatched(&skipped.client, &skipped.model)).map(|skipped| skipped.model.clone()));
    ids.iter()
        .map(|id| {
            let placed = place(gathered, &BTreeSet::from([id.clone()]));
            let listed = placed.listed.first();
            let free = placed.listed.iter().any(|(_, _, _, free, _, _)| *free);
            let usable = listed.is_some();
            let providers = listed.map(|(client, model, class, _, _, _)| {
                let [_, (_, providers)] = model_facts::facts(&gathered.config, gathered.reading.catalogue.as_ref(), client, model, class, free);
                providers
            });
            let deployments = match providers {
                Some(Value::Arr(providers)) => providers
                    .iter()
                    .filter_map(Value::as_object)
                    .enumerate()
                    .map(|(at, provider)| {
                        let text = |key: &str| provider.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
                        DeploymentRow {
                            order: at as i64 + 1,
                            provider: provider_name(&text("provider")),
                            route: text("route"),
                            key: None,
                            upstream: text("upstream_id"),
                            status: UNPROBED.to_string(),
                            reason: String::new(),
                            pool: route_key(&text("route")),
                        }
                    })
                    .collect(),
                _ => Vec::new(),
            };
            Row {
                model: id.clone(),
                modality: if usable { "chat" } else { "unknown" }.to_string(),
                family: listed.map_or_else(|| family_from(None, id), |(_, _, _, _, _, family)| family.clone()),
                tier: listed.and_then(|(_, _, _, _, tier, _)| *tier),
                context_length: None,
                free,
                cost: cost_of(gathered, &placed, id, free),
                status: if usable { "usable" } else { "unusable" }.to_string(),
                reason: reason_of("", &placed.refused),
                deployments,
                clients: placed.runs.iter().map(|(client, _)| client.clone()).collect::<BTreeSet<_>>().into_iter().collect(),
                profiles: profiles_of(&gathered.profiles, &placed.runs),
            }
        })
        .collect()
}

/// Whether a row passes the filter: a route matches a deployment's route or a client's.
fn passes(row: &Row, filter: &Filter) -> bool {
    let route = filter.route.as_deref().is_none_or(|route| {
        row.deployments.iter().any(|deployment| deployment.route == route) || row.clients.iter().any(|client| client == route || client.strip_prefix("pi-") == Some(route))
    });
    let status = filter.status.as_deref().is_none_or(|status| row.status == status || row.deployments.iter().any(|deployment| deployment.status == status));
    let modality = filter.modality.as_deref().is_none_or(|modality| row.modality == modality);
    route && status && modality && (!filter.free || row.free)
}

/// The catalog for the repository the caller works in, narrowed by the filter.
pub fn view(input: &Object, runtime: &Runtime) -> Result<View, GatewayError> {
    let filter = read_filter(input)?;
    let mut view = build(&commands::gather(input, runtime, false, true)?, &filter);
    // Only a capped pool's spend is read: the question a cap answers is how much of it is left.
    for pool in view.pools.iter_mut().filter(|pool| pool.cap_eur.is_some()) {
        pool.spend_eur = spend_of(runtime, &pool.route);
    }
    Ok(view)
}

/// The catalog over what the gateway gathered, narrowed by the filter.
pub fn build(gathered: &Gathered, filter: &Filter) -> View {
    let mut covered = BTreeSet::new();
    let mut rows = Vec::new();
    if let Some(catalogue) = gathered.reading.catalogue.as_ref() {
        for (name, group) in &catalogue.groups {
            let (row, names) = group_row(gathered, catalogue, name, group);
            covered.extend(names);
            rows.push(row);
        }
    }
    rows.extend(loose_rows(gathered, &covered));
    rows.retain(|row| passes(row, filter));
    rows.sort_by(|left, right| left.model.cmp(&right.model));
    let pools = pools_of(&rows);
    View {
        policy: gathered.policy_reason.clone(),
        updated_at: gathered.reading.catalogue.as_ref().map(|catalogue| catalogue.updated_at.clone()),
        warnings: gathered.reading.warning.iter().cloned().collect(),
        missing: gathered.permitted.missing.iter().map(|missing| (missing.client.clone(), missing.reason.clone())).collect(),
        rows,
        pools,
    }
}

/// The catalog as the gateway prints it: one JSON object.
pub fn run(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    view(input, runtime).map(|view| view.to_json())
}

fn strings(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

impl DeploymentRow {
    fn to_json(&self) -> Value {
        object(vec![
            ("order", Value::Num(self.order as f64)),
            ("provider", Value::str(&self.provider)),
            ("route", Value::str(&self.route)),
            ("key", self.key.as_deref().map_or(Value::Null, Value::str)),
            ("upstream_id", Value::str(&self.upstream)),
            ("status", Value::str(&self.status)),
            ("reason", Value::str(&self.reason)),
            ("pool", self.pool.as_deref().map_or(Value::Null, Value::str)),
        ])
    }
}

impl Pool {
    fn to_json(&self) -> Value {
        object(vec![
            ("key", Value::str(&self.key)),
            ("route", Value::str(&self.route)),
            ("monthly_cap_eur", self.cap_eur.map_or_else(|| Value::str("none"), Value::Num)),
            ("budget_duration", self.duration.as_deref().map_or(Value::Null, Value::str)),
            ("spend_eur", self.spend_eur.map_or_else(|| Value::str("unknown"), Value::Num)),
        ])
    }

    /// The pool's line of the table.
    pub(crate) fn line(&self) -> String {
        let cap = self.cap_eur.map_or("no cap".to_string(), |cap| format!("cap {} EUR per {}", json::number_text(cap), self.duration.as_deref().unwrap_or("window")));
        let spend = self.spend_eur.map_or("unknown".to_string(), json::number_text);
        format!("budget pool {} (route {}): {cap}, spend {spend}\n", self.key, self.route)
    }
}

impl Row {
    fn to_json(&self) -> Value {
        object(vec![
            ("model", Value::str(&self.model)),
            ("modality", Value::str(&self.modality)),
            ("family", Value::str(&self.family)),
            ("tier", self.tier.map_or(Value::Null, |tier| Value::str(tier.as_str()))),
            ("context_length", self.context_length.map_or(Value::Null, |length| Value::Num(length as f64))),
            ("free", Value::Bool(self.free)),
            ("cost", self.cost.clone()),
            ("status", Value::str(&self.status)),
            ("reason", Value::str(&self.reason)),
            ("deployments", Value::Arr(self.deployments.iter().map(DeploymentRow::to_json).collect())),
            ("clients", strings(&self.clients)),
            ("profiles", strings(&self.profiles)),
        ])
    }
}

impl View {
    pub fn to_json(&self) -> Value {
        let missing = self.missing.iter().map(|(client, reason)| object(vec![("client", Value::str(client)), ("reason", Value::str(reason))])).collect();
        object(vec![
            ("status", Value::str("ok")),
            ("policy", Value::str(&self.policy)),
            ("updated_at", self.updated_at.as_deref().map_or(Value::Null, Value::str)),
            ("warnings", strings(&self.warnings)),
            ("missing", Value::Arr(missing)),
            ("pools", Value::Arr(self.pools.iter().map(Pool::to_json).collect())),
            ("models", Value::Arr(self.rows.iter().map(Row::to_json).collect())),
        ])
    }

    /// The catalog as a plain table: a line per model with its facts, then what serves it and what runs it.
    pub fn to_table(&self) -> String {
        let cells: Vec<[String; 8]> = self.rows.iter().map(Row::cells).collect();
        let head = ["MODEL", "MODALITY", "FAMILY", "TIER", "CONTEXT", "FREE", "COST $/Mtok in/out", "STATUS"].map(String::from);
        let widths: Vec<usize> = (0..8).map(|at| std::iter::once(&head).chain(cells.iter()).map(|line| line[at].chars().count()).max().unwrap_or(0)).collect();
        let line = |cells: &[String; 8]| {
            let padded: Vec<String> = cells.iter().zip(&widths).map(|(cell, width)| format!("{cell:<width$}")).collect();
            padded.join("  ").trim_end().to_string()
        };
        let mut out = String::new();
        out.push_str(&format!("{}\n", line(&head)));
        for (row, cells) in self.rows.iter().zip(&cells) {
            out.push_str(&format!("{}\n", line(cells)));
            for deployment in &row.deployments {
                let key = deployment.key.as_deref().map_or(String::new(), |key| format!(", key {key}"));
                let reason = if deployment.reason.is_empty() { String::new() } else { format!(": {}", deployment.reason) };
                let pool = deployment.pool.as_deref().map_or(String::new(), |pool| format!(", pool {pool}"));
                out.push_str(&format!(
                    "    {}. {} via route {}{key}{pool} as {} [{}{reason}]\n",
                    deployment.order, deployment.provider, deployment.route, deployment.upstream, deployment.status
                ));
            }
            let runs = if row.clients.is_empty() { "nothing".to_string() } else { row.clients.join(", ") };
            let profiles = if row.profiles.is_empty() { "none".to_string() } else { row.profiles.join(", ") };
            out.push_str(&format!("    runs on: {runs}; profiles: {profiles}\n"));
            if !row.reason.is_empty() {
                out.push_str(&format!("    reason: {}\n", row.reason));
            }
        }
        for pool in &self.pools {
            out.push_str(&pool.line());
        }
        for (client, reason) in &self.missing {
            out.push_str(&format!("missing client {client}: {reason}\n"));
        }
        for warning in &self.warnings {
            out.push_str(&format!("warning: {warning}\n"));
        }
        out.push_str(&format!("{} models; policy: {}\n", self.rows.len(), self.policy));
        out
    }
}

impl Row {
    /// The cells of the model's table line.
    fn cells(&self) -> [String; 8] {
        let member = |key: &str| self.cost.as_object().and_then(|cost| cost.get(key));
        let price = |key: &str| match member(key) {
            Some(Value::Num(price)) => json::number_text(*price),
            _ => "unknown".to_string(),
        };
        let cost = format!("{} {}/{}", member("class").and_then(Value::as_str).unwrap_or_default(), price("input_per_mtok"), price("output_per_mtok"));
        [
            self.model.clone(),
            self.modality.clone(),
            self.family.clone(),
            self.tier.map_or("-", Tier::as_str).to_string(),
            self.context_length.map_or("-".to_string(), |length| length.to_string()),
            if self.free { "yes" } else { "no" }.to_string(),
            cost,
            self.status.clone(),
        ]
    }
}
