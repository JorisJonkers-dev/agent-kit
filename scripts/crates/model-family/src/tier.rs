//! The LiteLLM tier a Gold Band binding names (#914), resolved through the same profiles
//! `model-gateway run --tier` runs: `registry/litellm-tiers.json` says which tiers exist and
//! the steps of each chain, and the model gateway's `defaults.json` holds the profile of each
//! tier (`tier-free` is `pi-free:tier-free`), whose client says the Pi provider that calls it.
//!
//! A binding names a tier as the agent `pi` with the model `litellm-<route>/<tier>`, where
//! `litellm-<route>` is the provider of the tier's profile. A tier that is not in the snapshot,
//! or a provider that is not the tier's, is a problem the workflow lint reports and the
//! run-start shim refuses.

use std::collections::BTreeMap;

use serde_json::Value;

const TIERS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../registry/litellm-tiers.json"
));
const DEFAULTS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../skills/workstation/model-gateway/defaults.json"
));

/// The prefix of every tier name.
const PREFIX: &str = "tier-";

/// The vendor of a step's model, by a word its id carries. The critique routing places the clients
/// the critique pipeline runs; a tier's chain holds more models than those, so this table places
/// the others. A model it does not place has no family, and no allowlist entry can vouch for it.
const STEP_FAMILIES: [(&str, &str); 13] = [
    ("deepseek", "deepseek"),
    ("glm", "zhipu"),
    ("mistral", "mistral"),
    ("qwen", "alibaba"),
    ("gemma", "google"),
    ("gemini", "google"),
    ("nemotron", "nvidia"),
    ("gpt-oss", "openai"),
    ("minimax", "minimax"),
    ("mimo", "xiaomi"),
    ("longcat", "meituan"),
    ("kimi", "moonshot"),
    ("gpt-6", "openai"),
];

/// A binding that names a tier: the tier, and what is wrong with the binding, if anything.
#[derive(Debug, PartialEq, Eq)]
pub struct Named {
    pub tier: String,
    pub problem: Option<String>,
}

/// The tiers of the snapshot and the Pi provider of each.
pub struct Tiers {
    root: Value,
    pub(crate) providers: BTreeMap<String, String>,
}

impl Tiers {
    /// The agent a tier binding uses: Pi sends the tier's name as the model.
    pub const AGENT: &'static str = "pi";

    /// The tiers as shipped, parsed once.
    pub fn shipped() -> &'static Tiers {
        static SHIPPED: std::sync::OnceLock<Tiers> = std::sync::OnceLock::new();
        SHIPPED.get_or_init(Tiers::load)
    }

    /// The tiers as shipped.
    pub fn load() -> Tiers {
        let root: Value = serde_json::from_str(TIERS).expect("litellm-tiers.json is JSON");
        let defaults: Value = serde_json::from_str(DEFAULTS).expect("defaults.json is JSON");
        let none = serde_json::Map::new();
        let providers = defaults["profiles"]
            .as_object()
            .unwrap_or(&none)
            .values()
            .filter_map(|profile| {
                let tier = profile.get("tier")?.as_str()?;
                let client = profile["models"][0].as_str()?.split(':').next()?;
                Some((
                    tier.to_string(),
                    format!("litellm-{}", client.strip_prefix("pi-")?),
                ))
            })
            .collect();
        Tiers { root, providers }
    }

    /// Every tier name, in order.
    pub fn names(&self) -> Vec<String> {
        self.root["tier_groups"]
            .as_object()
            .map_or_else(Vec::new, |g| g.keys().cloned().collect())
    }

    /// The tier a binding names, with what is wrong with it. `None` when the binding names no
    /// tier: another agent, or a model whose name does not start with `tier-`.
    pub fn named(&self, agent: &str, model: &str) -> Option<Named> {
        let (provider, name) = model.rsplit_once('/').unwrap_or(("", model));
        if agent != Self::AGENT || !name.starts_with(PREFIX) {
            return None;
        }
        let known = self.root["tier_groups"].get(name).is_some();
        let problem = match (known, self.providers.get(name)) {
            (false, _) => Some(format!(
                "tier {name} is not a LiteLLM tier (known: {})",
                self.names().join(", ")
            )),
            (true, None) => Some(format!(
                "tier {name} has no profile in the model gateway's defaults"
            )),
            (true, Some(expected)) if expected != provider => Some(format!(
                "tier {name} is called through the provider {expected}, not {}",
                if provider.is_empty() {
                    "none"
                } else {
                    provider
                }
            )),
            (true, Some(_)) => None,
        };
        Some(Named {
            tier: name.to_string(),
            problem,
        })
    }

    /// What is wrong with each tier a node binds, one line each: `agents` and `models` are the
    /// agent ids and model ids the node binds. Nothing is wrong with a node whose agents do not
    /// include `pi`, or whose models name no tier.
    pub fn problems(&self, agents: &[&str], models: &[&str]) -> Vec<String> {
        let mut distinct: Vec<&str> = models.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        let pi = agents.contains(&Self::AGENT);
        distinct
            .into_iter()
            .filter(|_| pi)
            .filter_map(|model| {
                let named = self.named(Self::AGENT, model)?;
                Some(format!(
                    "binds {model} ({}): {}",
                    named.tier, named.problem?
                ))
            })
            .collect()
    }

    /// The family of one step of a chain, from a word its model id carries.
    pub fn step_family(model: &str) -> Option<&'static str> {
        let id = model.to_ascii_lowercase();
        STEP_FAMILIES
            .iter()
            .find(|(word, _)| id.contains(word))
            .map(|(_, family)| *family)
    }

    /// The models of a tier's chain, the chains it falls to included, in order.
    pub fn chain(&self, tier: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.walk(tier, &mut out, 0);
        out
    }

    fn walk(&self, tier: &str, out: &mut Vec<String>, depth: usize) {
        let steps = self.root["tier_groups"][tier]["steps"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        for step in steps {
            if let Some(model) = step["model"].as_str() {
                out.push(model.to_string());
            }
            if let Some(next) = step["tier"].as_str().filter(|_| depth < 8) {
                self.walk(next, out, depth + 1);
            }
        }
    }
}
