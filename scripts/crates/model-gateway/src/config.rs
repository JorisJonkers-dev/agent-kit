//! The gateway's configuration: the clients on this machine, the models each
//! one offers, and the settings a delegation is held to. This file holds the
//! data model; `config_parse` reads and merges it from JSON.

use std::collections::BTreeMap;

pub const CONFIG_VERSION: f64 = 1.0;

/// What a run on a client costs the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostClass {
    /// A model the live catalogue marks free (every deployment of it), or, where the catalogue is silent, whose Pi
    /// provider entry prices every field at zero. Never a guess.
    Free,
    Subsidised,
    MeteredPersonal,
    MeteredEmployer,
}

impl CostClass {
    pub const ALL: [CostClass; 4] = [CostClass::Free, CostClass::Subsidised, CostClass::MeteredPersonal, CostClass::MeteredEmployer];

    /// The name the JSON files and the output use: `free`, `subsidised`, `metered-personal`, `metered-employer`.
    pub fn as_str(self) -> &'static str {
        match self {
            CostClass::Free => "free",
            CostClass::Subsidised => "subsidised",
            CostClass::MeteredPersonal => "metered-personal",
            CostClass::MeteredEmployer => "metered-employer",
        }
    }

    /// The classes a configuration or the ledger may name. `free` is never among them: it is only ever found by
    /// reading the Pi provider's prices, so no file can declare a client or a run free.
    pub const DECLARABLE: [CostClass; 3] = [CostClass::Subsidised, CostClass::MeteredPersonal, CostClass::MeteredEmployer];

    pub fn parse(text: &str) -> Option<CostClass> {
        Self::DECLARABLE.into_iter().find(|class| class.as_str() == text)
    }
}

/// How hard a task a model covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Trivial,
    Routine,
    Hard,
    Frontier,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Tier::Trivial, Tier::Routine, Tier::Hard, Tier::Frontier];

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Trivial => "trivial",
            Tier::Routine => "routine",
            Tier::Hard => "hard",
            Tier::Frontier => "frontier",
        }
    }

}

/// Where a client's model list comes from: the client itself, its config, or the declared list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelSource {
    Live,
    Config,
    Declared,
}

impl ModelSource {
    pub const ALL: [ModelSource; 3] = [ModelSource::Live, ModelSource::Config, ModelSource::Declared];

    pub fn as_str(self) -> &'static str {
        match self {
            ModelSource::Live => "live",
            ModelSource::Config => "config",
            ModelSource::Declared => "declared",
        }
    }

}

/// The spans a plan limit may name; each reaches back from now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanWindow {
    FiveHours,
    Day,
    Week,
    Month,
}

impl PlanWindow {
    pub const ALL: [PlanWindow; 4] = [PlanWindow::FiveHours, PlanWindow::Day, PlanWindow::Week, PlanWindow::Month];

    /// `5h`, `day`, `week`, `month`.
    pub fn as_str(self) -> &'static str {
        match self {
            PlanWindow::FiveHours => "5h",
            PlanWindow::Day => "day",
            PlanWindow::Week => "week",
            PlanWindow::Month => "month",
        }
    }


    /// How far back the window reaches from now, in milliseconds (5h, 24h, 7 days, 30 days).
    pub fn span_ms(self) -> i64 {
        match self {
            PlanWindow::FiveHours => 5 * 60 * 60 * 1000,
            PlanWindow::Day => 24 * 60 * 60 * 1000,
            PlanWindow::Week => 7 * 24 * 60 * 60 * 1000,
            PlanWindow::Month => 30 * 24 * 60 * 60 * 1000,
        }
    }
}

/// One window of a client's plan: the dollars it allows, or the runs it allows, never both.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanLimit {
    Usd { window: PlanWindow, limit_usd: f64 },
    Runs { window: PlanWindow, limit_runs: f64 },
}

impl PlanLimit {
    pub fn window(&self) -> PlanWindow {
        match self {
            PlanLimit::Usd { window, .. } | PlanLimit::Runs { window, .. } => *window,
        }
    }
}

/// One client on this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientConfig {
    /// The program that runs it, as it is named on PATH.
    pub binary: String,
    pub cost_class: CostClass,
    pub model_source: ModelSource,
    /// The model families this client may reach; `None` when it is not restricted.
    pub families: Option<Vec<String>>,
    /// The cost class of each provider a client's models run on, by provider name; `None` when the client's own class covers every model.
    pub provider_cost_classes: Option<BTreeMap<String, CostClass>>,
    /// The providers a run may reach through the client; `None` when the client has no such allowlist.
    pub providers: Option<Vec<String>>,
    /// What the client's plan allows, window by window; `None` when the owner has set none.
    pub plan_limits: Option<Vec<PlanLimit>>,
    /// The provider route of the LiteLLM catalogue whose probes judge this client's models; `None` when it covers none.
    pub litellm_route: Option<String>,
    /// `data_scope: public` in the JSON: the client's route is for public work only, so a delegation that reads a
    /// repository is sent to it only when GitHub says the repository is public. A private repository, and one whose
    /// visibility cannot be read, are refused whatever their allowlist entry lists, and `models` with a `repo` leaves
    /// the client's models out for them.
    pub public_only: bool,
}

/// What one model costs, in USD per million tokens.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPrices {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
}

/// One model a client offers, as the declared list describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelEntry {
    pub client: String,
    pub model: String,
    /// The names the client's own configuration gives this model, where `model` is the alias the client accepts.
    pub names: Option<Vec<String>>,
    pub family: String,
    pub tier: Tier,
    pub notes: String,
    pub prices: Option<ModelPrices>,
}

impl ModelEntry {
    /// The id and every other name of the model.
    pub fn all_names(&self) -> Vec<&str> {
        std::iter::once(self.model.as_str()).chain(self.names.iter().flatten().map(String::as_str)).collect()
    }
}

/// The dollar caps a metered-personal client is held to.
#[derive(Clone, Debug, PartialEq)]
pub struct MeteredPersonalCaps {
    pub per_workflow_usd: f64,
    pub per_day_usd: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub plan_warning_ratio: f64,
    pub summary_max_chars: f64,
    pub max_parallel_per_client: f64,
    pub max_batch_delegations: f64,
    pub availability_window_minutes: f64,
    pub metered_personal_caps: MeteredPersonalCaps,
    /// The output tokens a run of each tier is allowed, in `Tier::ALL` order (trivial, routine, hard, frontier).
    pub output_allowance_tokens: [f64; 4],
    /// What `run --tier` reads before it chooses a tier, and how many runs a tier takes at once. Every field has a
    /// default, so the shipped defaults and a machine file need not state it.
    pub quota: crate::quota::QuotaSettings,
    /// How long a run's client may be silent before it is called stalled; every field has a default.
    pub stall: crate::stall::StallSettings,
}

impl Settings {
    pub fn allowance(&self, tier: Tier) -> f64 {
        self.output_allowance_tokens[Tier::ALL.iter().position(|candidate| *candidate == tier).unwrap_or(0)]
    }
}

/// The kinds of tool a profile scopes, in the order the output names them.
pub const TOOL_KINDS: [&str; 3] = ["mcp", "plugins", "skills"];

/// The names a profile allows of one kind of tool: some, or `any` for no limit. An empty list is deny-all.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolList {
    Any,
    Only(Vec<String>),
}

/// A profile's tool scope: deny-all unless it lists a kind or says `any`.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolScope {
    pub mcp: ToolList,
    pub plugins: ToolList,
    pub skills: ToolList,
}

impl ToolScope {
    /// The list of one kind, by its name in `TOOL_KINDS`.
    pub fn of(&self, kind: &str) -> &ToolList {
        match kind {
            "mcp" => &self.mcp,
            "plugins" => &self.plugins,
            _ => &self.skills,
        }
    }
}

/// One delegation profile, as the shipped defaults state it.
#[derive(Clone, Debug, PartialEq)]
pub struct DelegationProfile {
    pub name: String,
    pub description: String,
    /// The `<client>:<model>` keys the profile allows, each a model of the catalogue.
    pub models: Vec<String>,
    pub pin: Option<String>,
    /// The models of `models` a writer climbs through, cheapest first.
    pub ladder: Option<Vec<String>>,
    pub distinct_families: Option<f64>,
    /// The writer's family is left out of the allowed models.
    pub exclude_writer_family: Option<bool>,
    pub tools: ToolScope,
    /// The most a run may spend, in USD; held from #788, validated now.
    pub max_spend_usd: Option<f64>,
    /// Non-Pi clients the profile may run on although the gateway cannot enforce its tool scope there: the
    /// profile's own statement that the launch restrictions the gateway applies (read-only flags or agent, no MCP, a
    /// per-run home for OpenCode and Hermes) are accepted for the scope. It is not a statement of OS confinement.
    pub unenforced_clients: Option<Vec<String>>,
    /// The LiteLLM tier the profile is, which is also the model name the proxy is sent; `run --tier` names it.
    pub tier: Option<String>,
    /// What a LiteLLM key must hold to call the tier, which a refusal names when this machine has no such key.
    pub key_models: Option<Vec<String>>,
    /// The network reach a run of the profile has in a Docker Sandbox; none is the narrowest, the route alone.
    pub network: Option<crate::network::Reach>,
    /// The sbx service secrets the profile names: a sandbox starts while one of them is stored.
    pub sbx_secrets: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GatewayConfig {
    pub version: f64,
    /// Keyed by client id; iterate in key order, which is what the gateway's `sortedClients` gives.
    pub clients: BTreeMap<String, ClientConfig>,
    pub models: Vec<ModelEntry>,
    pub settings: Settings,
    /// The `<client>:<model>` keys the owner left out.
    pub disabled: Vec<String>,
    /// The delegation profiles the shipped defaults state, by name; a machine file does not add or change them.
    pub profiles: BTreeMap<String, DelegationProfile>,
}
