//! Routing: the clients a critic may run through, the roster each stage fills,
//! and the limits, packet caps, triage rules and gate patterns a run reads. The
//! node runner's `domain/routing.ts`, its data types and its reader.

use crate::depth::{no_depth_signals, read_depth_signals_config, DepthSignalsConfig};
use crate::error::{CritiqueError, ShapeError};
use crate::family::MODEL_FAMILIES;
use crate::fields::Fields;
use crate::gate::{read_gate_routing, GateRouting};
use crate::limits::{read_limits, CritiqueLimits};
use crate::pricing::{read_token_prices, TokenPrices};
use crate::repository_packet::{default_packet_limits, read_packet_limits, PacketLimits};
use crate::tickets::{default_ticket_size, read_ticket_size, TicketSize};
use crate::triage::read_triage_rules;
use model_gateway::json::Value;

/// The client kinds routing may name, in the order its reader accepts them.
pub const CLIENT_KINDS: [&str; 4] = ["vibe", "agy", "codex", "claude"];

/// The gateway client model a critic client is pinned to: the client the gateway
/// runs the critic on, and the alias the gateway lists that model by.
#[derive(Clone, Debug, PartialEq)]
pub struct GatewayModel {
    pub client: String,
    pub model: String,
}

/// One CLI pinned to one model. Model names live here and in the critic agent files only.
#[derive(Clone, Debug, PartialEq)]
pub struct VibeCriticClient {
    pub binary: String,
    /// The read-only `critic-<model>` Vibe agent that pins the model and its prices.
    pub agent: String,
    pub model: String,
    pub family: String,
    /// The model's prices, the same as its agent file's; without them the runner takes Vibe's own figure.
    pub prices: Option<TokenPrices>,
    /// The gateway client model this critic client is pinned to; without it the client runs its own kind and model.
    pub gateway: Option<GatewayModel>,
}

/// Antigravity in print mode: a Gemini model, in the Google family.
#[derive(Clone, Debug, PartialEq)]
pub struct AgyCriticClient {
    pub binary: String,
    pub model: String,
    pub family: String,
    pub gateway: Option<GatewayModel>,
}

/// Codex on the nedap account. Only the work roster of a `nedap` repository names it.
#[derive(Clone, Debug, PartialEq)]
pub struct CodexCriticClient {
    pub binary: String,
    pub model: String,
    pub family: String,
    pub gateway: Option<GatewayModel>,
}

/// The work Claude agent. Only the work roster of a `nedap` repository names it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeCriticClient {
    pub binary: String,
    pub model: String,
    pub family: String,
    pub gateway: Option<GatewayModel>,
}

/// A client a critic runs through; the variant is the client's kind.
#[derive(Clone, Debug, PartialEq)]
pub enum CriticClient {
    Vibe(VibeCriticClient),
    Agy(AgyCriticClient),
    Codex(CodexCriticClient),
    Claude(ClaudeCriticClient),
}

impl CriticClient {
    /// The client's kind, as routing names it.
    pub fn kind(&self) -> &'static str {
        match self {
            CriticClient::Vibe(_) => "vibe",
            CriticClient::Agy(_) => "agy",
            CriticClient::Codex(_) => "codex",
            CriticClient::Claude(_) => "claude",
        }
    }

    /// The family of the model the client runs.
    pub fn family(&self) -> &str {
        match self {
            CriticClient::Vibe(client) => &client.family,
            CriticClient::Agy(client) => &client.family,
            CriticClient::Codex(client) => &client.family,
            CriticClient::Claude(client) => &client.family,
        }
    }

    /// The model the client runs.
    pub fn model(&self) -> &str {
        match self {
            CriticClient::Vibe(client) => &client.model,
            CriticClient::Agy(client) => &client.model,
            CriticClient::Codex(client) => &client.model,
            CriticClient::Claude(client) => &client.model,
        }
    }
}

/// Whether a client belongs to the work roster, and so to no other.
fn is_work_client(client: &CriticClient) -> bool {
    matches!(client, CriticClient::Codex(_) | CriticClient::Claude(_))
}

/// Routing's `triage`, with the Vibe client its model runs through.
#[derive(Clone, Debug, PartialEq)]
pub struct TriageRouting {
    pub changed_lines_threshold: f64,
    pub labels: Vec<String>,
    pub model: TriageModelWithClient,
    pub risk_paths: Vec<String>,
}

/// The triage model, with the Vibe client it asks its yes/no question through.
#[derive(Clone, Debug, PartialEq)]
pub struct TriageModelWithClient {
    pub client: String,
    pub client_config: VibeCriticClient,
    pub max_price_usd: f64,
    pub time_limit_s: f64,
}

/// Routing's `cold_read`: the client a rewritten decision record is cold-read
/// through, and the caps its one launch per record runs under.
#[derive(Clone, Debug, PartialEq)]
pub struct ColdReadRouting {
    /// A Vibe client of routing's `clients`.
    pub client: String,
    pub client_config: VibeCriticClient,
    pub max_price_usd: f64,
    pub time_limit_s: f64,
}

/// One critic of a stage's roster: the client it runs through, its role and its lenses.
#[derive(Clone, Debug, PartialEq)]
pub struct RosterSlot {
    pub client: String,
    pub role: String,
    pub lenses: Vec<String>,
    /// Vibe's price cap. Antigravity is quota, not money, so agy ignores it.
    pub max_price_usd: f64,
    pub time_limit_s: f64,
}

/// One stage's rosters and what makes it go deep.
#[derive(Clone, Debug, PartialEq)]
pub struct StageRouting {
    pub distinct_families: bool,
    pub base: Vec<RosterSlot>,
    /// The critics deep depth adds after the base roster, each subject to the same family rules.
    pub deep: Vec<RosterSlot>,
    /// The stage's work-only roster, which a `nedap` repository uses instead of base and deep.
    pub work: Vec<RosterSlot>,
    /// The critics added at either depth when the artifact holds a decision record; only the decisions stage reads it.
    pub adr: Vec<RosterSlot>,
    /// What makes the stage go deep when the caller names no depth.
    pub depth_signals: DepthSignalsConfig,
    /// When a ticket is likely too big for one `/implement` context; only the tickets stage reads it.
    pub ticket_size: TicketSize,
}

/// When critique stats may draw a conclusion. They live in routing, beside what they tune.
#[derive(Clone, Debug, PartialEq)]
pub struct StatsThresholds {
    /// A cell with fewer accepted plus rejected findings than this gets no routing suggestion.
    pub min_adjudicated_findings: f64,
    /// How many of a stage's latest features must pass without a unique accepted catch before it is a removal candidate.
    pub removal_features: f64,
}

/// The routing a run uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Routing {
    pub version: f64,
    pub clients: Vec<(String, CriticClient)>,
    /// Per client, the clients tried in order when it may not run.
    pub fallbacks: Vec<(String, Vec<String>)>,
    pub stages: Vec<(String, StageRouting)>,
    /// Read by the stats command only, so a routing without them still prepares runs.
    pub stats: Option<StatsThresholds>,
    /// The per-flow Vibe cap and the Antigravity quota.
    pub limits: CritiqueLimits,
    /// How big a repository packet may be, and the paths it never sends.
    pub packet: PacketLimits,
    /// Ticket-review risk triage; routing without it cannot triage.
    pub triage: Option<TriageRouting>,
    /// The client a rewritten decision record is cold-read through; routing without it cannot cold-read.
    pub cold_read: Option<ColdReadRouting>,
    /// What the PR gate reads; without it, every branch a push updates is gated.
    pub gate: Option<GateRouting>,
}

/// Reads routing, in the order the node runner evaluates its fields, so the first wrong field is the one reported.
pub fn parse_routing(value: &Value) -> Result<Routing, ShapeError> {
    let file = Fields::of(value, "")?;
    let clients = file.entries("clients", |client, _| read_critic_client(client))?;
    let triage = if file.has("triage") { Some(read_triage_routing(&file.object("triage")?, &clients)?) } else { None };
    let fallbacks = if file.has("fallbacks") { open_fallbacks(&file.string_lists("fallbacks")?, &clients)? } else { Vec::new() };
    let limits = read_limits(&file)?;
    let packet = if file.has("packet") { read_packet_limits(&file.object("packet")?)? } else { default_packet_limits() };
    let stages = file.entries("stages", |stage, name| read_stage_routing(stage, name, &clients))?;
    let cold_read = if file.has("cold_read") { Some(read_cold_read_routing(&file.object("cold_read")?, &clients)?) } else { None };
    file.literal("version", 1.0)?;
    let stats = if file.has("stats") { Some(read_stats_thresholds(&file.object("stats")?)?) } else { None };
    let gate = if file.has("gate") { Some(read_gate_routing(&file.object("gate")?)?) } else { None };
    Ok(Routing { version: 1.0, clients, fallbacks, stages, stats, limits, packet, triage, cold_read, gate })
}

/// A stage's routing; a stage routing does not define is an error.
pub fn stage_routing<'a>(routing: &'a Routing, stage: &str) -> Result<&'a StageRouting, CritiqueError> {
    match routing.stages.iter().find(|(name, _)| name == stage) {
        Some((_, found)) => Ok(found),
        None => Err(CritiqueError::new("invalid-routing", format!("routing has no stage {stage}"))),
    }
}

/// One stage's rosters, read in the order the node runner builds them.
fn read_stage_routing(stage: &Fields<'_>, name: &str, clients: &[(String, CriticClient)]) -> Result<StageRouting, ShapeError> {
    let base = only_open_clients(&stage.list("base", |slot| read_roster_slot(slot))?, clients, &format!("stages.{name}.base"))?;
    let adr = only_open_clients(&optional_slots(stage, "adr")?, clients, &format!("stages.{name}.adr"))?;
    let deep = only_open_clients(&optional_slots(stage, "deep")?, clients, &format!("stages.{name}.deep"))?;
    let work = only_work_clients(&optional_slots(stage, "work")?, clients, &format!("stages.{name}.work"))?;
    let depth_signals = if stage.has("depth_signals") { read_depth_signals_config(&stage.object("depth_signals")?, name)? } else { no_depth_signals() };
    let distinct_families = stage.boolean("distinct_families")?;
    let ticket_size = if stage.has("ticket_size") { read_ticket_size(&stage.object("ticket_size")?)? } else { default_ticket_size() };
    Ok(StageRouting { distinct_families, base, deep, work, adr, depth_signals, ticket_size })
}

/// A stage's optional roster: the slots it names, or none when it names none.
fn optional_slots(stage: &Fields<'_>, key: &str) -> Result<Vec<RosterSlot>, ShapeError> {
    if stage.has(key) { stage.list(key, |slot| read_roster_slot(slot)) } else { Ok(Vec::new()) }
}

/// A client is open when a base or deep roster may use it: anything but the work clients.
fn is_open(clients: &[(String, CriticClient)], name: &str) -> bool {
    match client_named(clients, name) {
        Some(client) => !is_work_client(client),
        None => true,
    }
}

/// The client routing defines under `name`.
fn client_named<'a>(clients: &'a [(String, CriticClient)], name: &str) -> Option<&'a CriticClient> {
    clients.iter().find(|(key, _)| key == name).map(|(_, client)| client)
}

fn only_open_clients(slots: &[RosterSlot], clients: &[(String, CriticClient)], place: &str) -> Result<Vec<RosterSlot>, ShapeError> {
    match slots.iter().find(|slot| !is_open(clients, &slot.client)) {
        Some(work) => Err(ShapeError(format!(
            "{place} names {}, a work client: only a stage's work roster may name Codex or the work Claude agent",
            work.client
        ))),
        None => Ok(slots.to_vec()),
    }
}

fn only_work_clients(slots: &[RosterSlot], clients: &[(String, CriticClient)], place: &str) -> Result<Vec<RosterSlot>, ShapeError> {
    match slots.iter().find(|slot| is_open(clients, &slot.client)) {
        Some(open) => Err(ShapeError(format!("{place} names {}, which is not a codex or claude client: a work roster holds nothing else", open.client))),
        None => Ok(slots.to_vec()),
    }
}

/// The fallbacks, refused when any of them names a work client: a work client has no fallbacks.
fn open_fallbacks(fallbacks: &[(String, Vec<String>)], clients: &[(String, CriticClient)]) -> Result<Vec<(String, Vec<String>)>, ShapeError> {
    for (client, names) in fallbacks {
        let work = std::iter::once(client).chain(names.iter()).find(|name| !is_open(clients, name.as_str()));
        if let Some(work) = work {
            return Err(ShapeError(format!("fallbacks.{client} names {work}, a work client: it has no fallbacks")));
        }
    }
    Ok(fallbacks.to_vec())
}

fn read_stats_thresholds(fields: &Fields<'_>) -> Result<StatsThresholds, ShapeError> {
    Ok(StatsThresholds {
        min_adjudicated_findings: fields.positive_integer("min_adjudicated_findings")?,
        removal_features: fields.positive_integer("removal_features")?,
    })
}

/// One critic client: its kind, the binary it launches, the model it pins and the gateway model it may be pinned to.
pub fn read_critic_client(fields: &Fields<'_>) -> Result<CriticClient, ShapeError> {
    let kind = fields.one_of("kind", &CLIENT_KINDS)?;
    let gateway = if fields.has("gateway") { Some(read_gateway_model(&fields.object("gateway")?, &kind)?) } else { None };
    match kind.as_str() {
        "codex" => Ok(CriticClient::Codex(CodexCriticClient {
            binary: fields.string("binary")?,
            family: fields.literal_str("family", "openai")?,
            model: fields.string("model")?,
            gateway,
        })),
        "claude" => Ok(CriticClient::Claude(ClaudeCriticClient {
            binary: fields.string("binary")?,
            family: fields.literal_str("family", "anthropic")?,
            model: fields.string("model")?,
            gateway,
        })),
        "agy" => Ok(CriticClient::Agy(AgyCriticClient {
            binary: fields.string("binary")?,
            family: fields.literal_str("family", "google")?,
            model: fields.matching("model", is_gemini_model, "a Gemini model: agy's Claude and GPT-OSS models are never critics")?,
            gateway,
        })),
        _ => Ok(CriticClient::Vibe(VibeCriticClient {
            agent: fields.string("agent")?,
            binary: fields.string("binary")?,
            family: fields.one_of("family", &MODEL_FAMILIES)?,
            model: fields.string("model")?,
            prices: if fields.has("prices") { Some(read_token_prices(&fields.object("prices")?)?) } else { None },
            gateway,
        })),
    }
}

/// The gateway client model a critic client names: the alias the gateway lists, and a client of the critic's own kind.
fn read_gateway_model(fields: &Fields<'_>, kind: &str) -> Result<GatewayModel, ShapeError> {
    Ok(GatewayModel {
        client: fields.matching("client", |value| value == kind, &format!("{kind}, the kind of the critic client"))?,
        model: fields.string("model")?,
    })
}

/// Whether a model is a Gemini one: agy's Claude and GPT-OSS models are never critics.
fn is_gemini_model(model: &str) -> bool {
    model.starts_with("gemini-")
}

/// One critic of a stage's roster.
pub fn read_roster_slot(fields: &Fields<'_>) -> Result<RosterSlot, ShapeError> {
    Ok(RosterSlot {
        client: fields.string("client")?,
        lenses: fields.strings("lenses")?,
        max_price_usd: fields.number("max_price_usd")?,
        role: fields.string("role")?,
        time_limit_s: fields.number("time_limit_s")?,
    })
}

/// The triage rules, with the Vibe client its model runs through: the model asks a yes/no question through Vibe.
fn read_triage_routing(fields: &Fields<'_>, clients: &[(String, CriticClient)]) -> Result<TriageRouting, ShapeError> {
    let rules = read_triage_rules(fields)?;
    let config = match client_named(clients, &rules.model.client) {
        Some(CriticClient::Vibe(config)) => config.clone(),
        Some(_) => {
            return Err(ShapeError(format!(
                "triage.model.client must name a vibe client of clients; {} is an agy client",
                rules.model.client
            )))
        }
        None => {
            return Err(ShapeError(format!(
                "triage.model.client must name a vibe client of clients; {} is not defined",
                rules.model.client
            )))
        }
    };
    Ok(TriageRouting {
        changed_lines_threshold: rules.changed_lines_threshold,
        labels: rules.labels,
        model: TriageModelWithClient {
            client: rules.model.client,
            client_config: config,
            max_price_usd: rules.model.max_price_usd,
            time_limit_s: rules.model.time_limit_s,
        },
        risk_paths: rules.risk_paths,
    })
}

/// The cold reader asks its question through Vibe, so its client must be a Vibe client routing defines.
fn read_cold_read_routing(fields: &Fields<'_>, clients: &[(String, CriticClient)]) -> Result<ColdReadRouting, ShapeError> {
    let client = fields.string("client")?;
    let config = match client_named(clients, &client) {
        Some(CriticClient::Vibe(config)) => config.clone(),
        Some(other) => {
            return Err(ShapeError(format!(
                "cold_read.client must name a vibe client of clients; {client} is an {} client",
                other.kind()
            )))
        }
        None => return Err(ShapeError(format!("cold_read.client must name a vibe client of clients; {client} is not defined"))),
    };
    Ok(ColdReadRouting {
        client,
        client_config: config,
        max_price_usd: fields.number("max_price_usd")?,
        time_limit_s: fields.number("time_limit_s")?,
    })
}
