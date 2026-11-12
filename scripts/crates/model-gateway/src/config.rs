//! The gateway's configuration: the clients on this machine, the models each
//! one offers, and the settings a delegation is held to. This file holds the
//! data model; `config_parse` reads and merges it from JSON.

use std::collections::BTreeMap;

pub const CONFIG_VERSION: f64 = 1.0;

/// What a run on a client costs the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostClass {
    Subsidised,
    MeteredPersonal,
    MeteredEmployer,
}

impl CostClass {
    pub const ALL: [CostClass; 3] = [CostClass::Subsidised, CostClass::MeteredPersonal, CostClass::MeteredEmployer];

    /// The name the JSON files and the output use: `subsidised`, `metered-personal`, `metered-employer`.
    pub fn as_str(self) -> &'static str {
        match self {
            CostClass::Subsidised => "subsidised",
            CostClass::MeteredPersonal => "metered-personal",
            CostClass::MeteredEmployer => "metered-employer",
        }
    }

    pub fn parse(text: &str) -> Option<CostClass> {
        Self::ALL.into_iter().find(|class| class.as_str() == text)
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
}

impl Settings {
    pub fn allowance(&self, tier: Tier) -> f64 {
        self.output_allowance_tokens[Tier::ALL.iter().position(|candidate| *candidate == tier).unwrap_or(0)]
    }
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
}
