//! Quota-aware tier choice: before a `run --tier`, read how much of the plan the tier draws on is used, and drop
//! to the next cheaper tier on the ladder at 80% of a window.
//!
//! **The ladder** is `tier-free`, `tier-vibe`, `tier-go-cheap`, `tier-strong-open`: cheapest first, the order an
//! escalation climbs it. A tier that is out of quota drops to the *previous* rung, never to a dearer one, so a run
//! never costs more than the tier it asked for. "Next" is therefore the next cheaper rung that still serves the
//! role: `tier-strong-open` and `tier-go-cheap` both draw on Go's allowance, so a drop from the first passes over
//! the second when Go is the reason, and lands on `tier-vibe`; `tier-vibe` drops to `tier-free`, which has no plan
//! to run out of and is the floor. A tier that is not on the ladder (`tier-review`, which already tries the free
//! models first) is never moved.
//!
//! **The signals**, in the order they are trusted:
//! 1. *Go's allowance.* OpenCode Go reports what each request drew on the plan in the response body, as
//!    `usage.opencode.allowance` (USD). The routing proxy records the sum per run on the ledger line as
//!    `allowance_usd`, and the windows are summed from the ledger: the rolling 5 hours, the rolling week and the
//!    30-day cycle the account's cap is anchored to. There is no endpoint that reports the plan's remaining
//!    allowance, so the windows are the gateway's own sums against configured limits.
//! 2. *An account's monthly cap* (Vibe's four accounts), from the price LiteLLM put on each request
//!    (`x-litellm-response-cost`, written as `litellm_cost_usd`) summed per account, which the serving deployment's
//!    id names, over the calendar month from the 1st. A Vibe tier is out of quota only when all four accounts are.
//! 3. *The key's own cap*, when the ledger holds no signal for the tier: `GET /key/info` with the tier's key,
//!    which a workstation key may call, gives the key's `spend` and `max_budget`.
//!
//! With none of these the tier is chosen as asked, and the decision says `no-signal`.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ledger_file::LedgerStore;
use crate::pi_launch::{home_agent_dir, key_of};
use crate::ports::{ProcessCommand, Runtime};
use crate::text::join;
use crate::timestamps;
use std::collections::BTreeMap;

/// The tiers cheapest first. A drop goes down this list, never up it.
pub const LADDER: [&str; 4] = ["tier-free", "tier-vibe", "tier-go-cheap", "tier-strong-open"];

/// Slack for a ratio that is 80% in decimals and a hair under it in binary.
const EPSILON: f64 = 1e-9;

/// The share of a window at which the tier is dropped.
pub const DEFAULT_THRESHOLD: f64 = 0.8;

/// Go's accounts: the 30-day cycle starts at this moment and every 30 days after it.
const GO_CYCLE_ANCHOR: &str = "2026-11-04T10:41:00.000Z";
const GO_CYCLE_MS: i64 = 30 * DAY_MS;
const GO_ACCOUNT: &str = "opencode-go";
const VIBE_ACCOUNTS: [&str; 4] = ["vibe", "vibe-study", "vibe-api", "vibe-study-api"];
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// The caps of litellm's `catalog/routing.json` `accounts`, in USD: Vibe's per calendar month, Go's per cycle.
const ACCOUNT_CAPS: [(&str, f64); 5] = [("vibe", 255.0), ("vibe-study", 127.0), ("vibe-api", 25.5), ("vibe-study-api", 12.7), (GO_ACCOUNT, 40.0)];

/// How many runs a tier takes at once, unless `settings.quota.parallel` says otherwise.
const TIER_PARALLEL: [(&str, usize); 5] = [("tier-free", 4), ("tier-vibe", 3), ("tier-go-cheap", 2), ("tier-strong-open", 1), ("tier-review", 3)];

/// What the quota check reads and enforces, all of it overridable by `settings.quota`.
#[derive(Clone, Debug, PartialEq)]
pub struct QuotaSettings {
    /// The share of a window (0 to 1) at which a tier is dropped.
    pub threshold: f64,
    /// What Go's plan allows in a rolling 5 hours and a rolling week, in USD of allowance.
    pub go_five_hour_usd: f64,
    pub go_weekly_usd: f64,
    /// An account's cap per window, in USD, by account name.
    pub accounts: BTreeMap<String, f64>,
    /// How many runs a tier takes at once, by tier name.
    pub parallel: BTreeMap<String, usize>,
    /// How many a tier takes when `parallel` does not name it.
    pub default_parallel: usize,
    /// How long a run waits for a free slot of its tier before it gives up, and how old a slot is before it is
    /// taken to belong to a run that died.
    pub slot_wait_s: f64,
    pub slot_stale_s: f64,
}

impl Default for QuotaSettings {
    fn default() -> Self {
        QuotaSettings {
            threshold: DEFAULT_THRESHOLD,
            go_five_hour_usd: 12.0,
            go_weekly_usd: 30.0,
            accounts: ACCOUNT_CAPS.iter().map(|(name, cap)| (name.to_string(), *cap)).collect(),
            parallel: TIER_PARALLEL.iter().map(|(name, count)| (name.to_string(), *count)).collect(),
            default_parallel: 2,
            slot_wait_s: 600.0,
            slot_stale_s: 7200.0,
        }
    }
}

impl QuotaSettings {
    /// How many runs `tier` takes at once.
    pub fn parallel_of(&self, tier: &str) -> usize {
        self.parallel.get(tier).copied().unwrap_or(self.default_parallel)
    }

    fn cap(&self, account: &str) -> f64 {
        self.accounts.get(account).copied().unwrap_or(f64::INFINITY)
    }
}

fn invalid(message: String) -> GatewayError {
    GatewayError::new("invalid-config", message)
}

const FIELDS: [&str; 8] = ["threshold", "go_five_hour_usd", "go_weekly_usd", "accounts", "parallel", "default_parallel", "slot_wait_s", "slot_stale_s"];

/// `base` with what `raw` (a `settings.quota` object) states over it.
pub fn merge(base: &QuotaSettings, raw: Option<&Value>) -> Result<QuotaSettings, GatewayError> {
    let Some(raw) = raw else { return Ok(base.clone()) };
    let Some(object) = raw.as_object() else { return Err(invalid("settings.quota must be an object".to_string())) };
    if let Some((name, _)) = object.entries().into_iter().find(|(name, _)| !FIELDS.contains(name)) {
        return Err(invalid(format!("settings.quota.{name} is not a setting: it has {}", FIELDS.join(", "))));
    }
    let number = |name: &str, current: f64| match object.get(name) {
        None => Ok(current),
        Some(Value::Num(value)) if value.is_finite() && *value > 0.0 => Ok(*value),
        Some(_) => Err(invalid(format!("settings.quota.{name} must be a number above 0"))),
    };
    let threshold = number("threshold", base.threshold)?;
    if threshold > 1.0 {
        return Err(invalid("settings.quota.threshold must be a share of a window: above 0 and at most 1".to_string()));
    }
    let count = |name: &str, value: Option<&Value>| match value {
        Some(Value::Num(count)) if count.fract() == 0.0 && *count >= 1.0 => Ok(*count as usize),
        _ => Err(invalid(format!("settings.quota.{name} must be a whole number of at least 1"))),
    };
    let mut accounts = base.accounts.clone();
    for (name, value) in members(object, "accounts")? {
        match value {
            Value::Num(cap) if cap.is_finite() && *cap > 0.0 => accounts.insert(name.to_string(), *cap),
            _ => return Err(invalid(format!("settings.quota.accounts.{name} must be a number above 0"))),
        };
    }
    let mut parallel = base.parallel.clone();
    for (name, value) in members(object, "parallel")? {
        parallel.insert(name.to_string(), count(&format!("parallel.{name}"), Some(value))?);
    }
    let default_parallel = match object.get("default_parallel") {
        None => base.default_parallel,
        value => count("default_parallel", value)?,
    };
    Ok(QuotaSettings {
        threshold,
        go_five_hour_usd: number("go_five_hour_usd", base.go_five_hour_usd)?,
        go_weekly_usd: number("go_weekly_usd", base.go_weekly_usd)?,
        accounts,
        parallel,
        default_parallel,
        slot_wait_s: number("slot_wait_s", base.slot_wait_s)?,
        slot_stale_s: number("slot_stale_s", base.slot_stale_s)?,
    })
}

/// The members of an object-valued setting, none when it is left out.
fn members<'a>(object: &'a Object, name: &str) -> Result<Vec<(&'a str, &'a Value)>, GatewayError> {
    match object.get(name) {
        None => Ok(Vec::new()),
        Some(Value::Obj(inner)) => Ok(inner.entries()),
        Some(_) => Err(invalid(format!("settings.quota.{name} must be an object"))),
    }
}

/// One window of an account: what is used of what it allows.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub name: String,
    pub used: f64,
    pub limit: f64,
}

impl Window {
    fn new(name: &str, used: f64, limit: f64) -> Window {
        Window { name: name.to_string(), used, limit }
    }

    pub fn ratio(&self) -> f64 {
        self.used / self.limit
    }
}

/// An account and its windows. The account is out of quota when any one of its windows is.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub name: String,
    pub windows: Vec<Window>,
}

/// Where a reading came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Go's allowance, summed from the ledger.
    Allowance,
    /// An account's monthly cap, against LiteLLM's price of each request.
    Account,
    /// The key's own `spend` and `max_budget`.
    Key,
    /// Nothing says how much is used.
    None,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Allowance => "allowance",
            Source::Account => "account",
            Source::Key => "key",
            Source::None => "none",
        }
    }
}

/// What the signals say of one tier: its accounts and their windows. The tier is out of quota when it has accounts
/// and every one of them is at the threshold in some window: a tier with a route left still serves.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub tier: String,
    pub source: Source,
    pub accounts: Vec<Account>,
}

impl Reading {
    fn none(tier: &str) -> Reading {
        Reading { tier: tier.to_string(), source: Source::None, accounts: Vec::new() }
    }

    /// The windows at the threshold, one per account, when the tier is out of quota.
    fn exhausted(&self, threshold: f64) -> Option<Vec<(&Account, &Window)>> {
        let hottest: Vec<Option<(&Account, &Window)>> = self
            .accounts
            .iter()
            .map(|account| {
                let hot = account.windows.iter().filter(|window| window.ratio() >= threshold - EPSILON);
                hot.max_by(|a, b| a.ratio().total_cmp(&b.ratio())).map(|window| (account, window))
            })
            .collect();
        let all: Option<Vec<(&Account, &Window)>> = hottest.into_iter().collect();
        all.filter(|windows| !windows.is_empty())
    }
}

/// How a tier was chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub requested: String,
    pub chosen: String,
    pub reason: String,
    /// What was read for each tier looked at, the requested one first.
    pub readings: Vec<Reading>,
}

impl Decision {
    /// The ledger line's `tier_decision`.
    pub fn to_json(&self) -> Value {
        let mut signals = Vec::new();
        for reading in &self.readings {
            if reading.accounts.is_empty() {
                signals.push(json::object(vec![("tier", Value::str(&reading.tier)), ("source", Value::str(reading.source.as_str()))]));
            }
            for account in &reading.accounts {
                for window in &account.windows {
                    signals.push(json::object(vec![
                        ("tier", Value::str(&reading.tier)),
                        ("source", Value::str(reading.source.as_str())),
                        ("account", Value::str(&account.name)),
                        ("window", Value::str(&window.name)),
                        ("used", Value::Num(window.used)),
                        ("limit", Value::Num(window.limit)),
                        ("ratio", Value::Num(round4(window.ratio()))),
                    ]));
                }
            }
        }
        json::object(vec![
            ("requested", Value::str(&self.requested)),
            ("chosen", Value::str(&self.chosen)),
            ("reason", Value::str(&self.reason)),
            ("signals", Value::Arr(signals)),
        ])
    }
}

fn round4(number: f64) -> f64 {
    (number * 10_000.0).round() / 10_000.0
}

/// Chooses among `candidates` (the requested tier, then each cheaper one in turn) the first that is not out of
/// quota. `read` reads one tier's signals, and is called for a cheaper tier only when the one before it is out. When
/// every candidate is out the last one is chosen: there is nothing cheaper to drop to.
pub fn walk(candidates: &[String], threshold: f64, read: &mut dyn FnMut(&str) -> Reading) -> Decision {
    let (last, rest) = candidates.split_last().expect("a tier was asked for");
    let mut readings: Vec<Reading> = Vec::new();
    let mut outs: Vec<String> = Vec::new();
    let mut look = |tier: &str, readings: &mut Vec<Reading>| {
        let reading = read(tier);
        let said = reading.exhausted(threshold).map(|windows| {
            let each: Vec<String> = windows.iter().map(|(account, window)| format!("{} {} at {:.0}% of {}", account.name, window.name, window.ratio() * 100.0, json::number_text(window.limit))).collect();
            format!("{tier} out of quota ({}): {}", reading.source.as_str(), each.join(", "))
        });
        let source = reading.source;
        readings.push(reading);
        (said, source)
    };
    for tier in rest {
        match look(tier, &mut readings) {
            (Some(said), _) => outs.push(said),
            (None, source) => return decided(candidates, tier, source, outs, readings),
        }
    }
    match look(last, &mut readings) {
        (Some(said), _) => {
            outs.push(said);
            outs.push(format!("nothing cheaper than {last}, so it stays"));
            Decision { requested: candidates[0].clone(), chosen: last.clone(), reason: outs.join("; "), readings }
        }
        (None, source) => decided(candidates, last, source, outs, readings),
    }
}

/// The decision for a tier that has quota: as asked when nothing above it was out, else a drop.
fn decided(candidates: &[String], tier: &str, source: Source, outs: Vec<String>, readings: Vec<Reading>) -> Decision {
    let reason = if !outs.is_empty() {
        format!("{}; dropped to {tier}", outs.join("; "))
    } else if source == Source::None {
        "no-signal".to_string()
    } else {
        "within-quota".to_string()
    };
    Decision { requested: candidates[0].clone(), chosen: tier.to_string(), reason, readings }
}

/// The tiers `tier` may be run as, the tier itself first and then each cheaper rung of the ladder; a tier that is
/// not on the ladder has itself only.
pub fn candidates(tier: &str) -> Vec<String> {
    match LADDER.iter().position(|rung| *rung == tier) {
        Some(at) => LADDER[..=at].iter().rev().map(|rung| rung.to_string()).collect(),
        None => vec![tier.to_string()],
    }
}

/// What the ledger says of one finished run's draw on the plans.
#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    pub at_ms: i64,
    pub allowance_usd: Option<f64>,
    pub cost_usd: Option<f64>,
    /// The account of the deployment that served the run: the part of its id before the slash.
    pub account: Option<String>,
}

/// Every run of the ledger that drew on a plan, with when. A ledger that cannot be read says nothing.
pub fn draws(runtime: &Runtime) -> Vec<Draw> {
    let entries = (LedgerStore { runtime }).read().unwrap_or_default();
    crate::caps::run_records(&entries)
        .into_iter()
        .filter(|run| run.allowance_usd.is_some() || run.litellm_cost_usd.is_some())
        .filter_map(|run| {
            Some(Draw {
                at_ms: timestamps::parse(&run.at)?,
                allowance_usd: run.allowance_usd,
                cost_usd: run.litellm_cost_usd,
                account: run.litellm_model_id.as_deref().and_then(|id| id.split_once('/')).map(|(account, _)| account.to_string()),
            })
        })
        .collect()
}

/// The start of the 30-day cycle Go's cap runs in at `now_ms`.
fn go_cycle_start(now_ms: i64) -> i64 {
    let anchor = timestamps::parse(GO_CYCLE_ANCHOR).expect("the anchor is a timestamp");
    anchor + (now_ms - anchor).div_euclid(GO_CYCLE_MS) * GO_CYCLE_MS
}

/// The first of the month `now_ms` falls in, at 00:00 UTC.
fn month_start(now_ms: i64) -> i64 {
    let day = timestamps::to_iso(now_ms);
    timestamps::parse(&format!("{}-01T00:00:00.000Z", &day[..7])).expect("the first of a month is a timestamp")
}

/// What `GET /key/info` says of a key: what it has spent and the cap it was given, if any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyInfo {
    pub spend: f64,
    pub max_budget: Option<f64>,
}

/// The signals of one tier, from the ledger's draws and, where they say nothing, from the key.
pub fn reading_of(tier: &str, draws: &[Draw], now_ms: i64, settings: &QuotaSettings, key: &mut dyn FnMut() -> Option<KeyInfo>) -> Reading {
    let since = |start: i64, value: &dyn Fn(&Draw) -> Option<f64>, only: &dyn Fn(&Draw) -> bool| -> (bool, f64) {
        let drawn: Vec<f64> = draws.iter().filter(|draw| draw.at_ms >= start && draw.at_ms <= now_ms && only(draw)).filter_map(value).collect();
        (!drawn.is_empty(), drawn.iter().sum())
    };
    let reading = match tier {
        "tier-go-cheap" | "tier-strong-open" => {
            let cycle = since(go_cycle_start(now_ms), &|draw| draw.allowance_usd, &|_| true);
            let windows = |from: i64| since(from, &|draw| draw.allowance_usd, &|_| true).1;
            cycle.0.then(|| Reading {
                tier: tier.to_string(),
                source: Source::Allowance,
                accounts: vec![Account {
                    name: GO_ACCOUNT.to_string(),
                    windows: vec![
                        Window::new("5h", windows(now_ms - 5 * HOUR_MS), settings.go_five_hour_usd),
                        Window::new("week", windows(now_ms - 7 * DAY_MS), settings.go_weekly_usd),
                        Window::new("cycle", cycle.1, settings.cap(GO_ACCOUNT)),
                    ],
                }],
            })
        }
        "tier-vibe" => {
            let month = month_start(now_ms);
            let spent = |account: &str| since(month, &|draw| draw.cost_usd, &|draw| draw.account.as_deref() == Some(account));
            VIBE_ACCOUNTS.iter().any(|account| spent(account).0).then(|| Reading {
                tier: tier.to_string(),
                source: Source::Account,
                accounts: VIBE_ACCOUNTS.iter().map(|account| Account { name: account.to_string(), windows: vec![Window::new("month", spent(account).1, settings.cap(account))] }).collect(),
            })
        }
        _ => return Reading::none(tier),
    };
    reading.or_else(|| key_reading(tier, key())).unwrap_or_else(|| Reading::none(tier))
}

/// The key-level reading: only a key that was given a cap says how near it is.
fn key_reading(tier: &str, info: Option<KeyInfo>) -> Option<Reading> {
    let info = info?;
    let limit = info.max_budget.filter(|cap| *cap > 0.0)?;
    Some(Reading { tier: tier.to_string(), source: Source::Key, accounts: vec![Account { name: "key".to_string(), windows: vec![Window::new("key", info.spend, limit)] }] })
}

/// The key a Pi provider holds and where its proxy is, from the home's `models.json`.
fn provider_key(runtime: &Runtime, provider: &str) -> Option<(String, String)> {
    let path = join(&[&home_agent_dir(&runtime.env), "models.json"]);
    let parsed = json::parse(&runtime.files.read_text(&path).ok()?).ok()?;
    let entry = parsed.as_object()?.get("providers")?.as_object()?.get(provider)?.as_object()?;
    let (origin, _) = crate::routing::split_base(entry.get("baseUrl")?.as_str()?)?;
    let key = key_of(runtime.process.as_ref(), provider, entry.get("apiKey")?.as_str()?).ok()?;
    Some((origin, key))
}

/// What LiteLLM says of the key the Pi provider holds: `GET /key/info`, which a workstation key may call on itself.
/// The key goes to curl on its stdin, as a config line, never on a command line. `None` when any step fails: the
/// check is advice, and a proxy that cannot be asked never stops a run.
pub fn key_info(runtime: &Runtime, provider: &str) -> Option<KeyInfo> {
    let (origin, key) = provider_key(runtime, provider)?;
    if key.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return None;
    }
    let mut command = ProcessCommand::new("curl", &["-sS", "-m", "10", "--config", "-", &format!("{origin}/key/info")]);
    command.stdin = Some(format!("header = \"Authorization: Bearer {}\"\n", crate::routing::quoted(&key)));
    command.timeout_ms = Some(15_000);
    let done = runtime.process.run(&command).ok().filter(|done| done.exit_code == 0 && !done.timed_out)?;
    let parsed = json::parse(&done.stdout).ok()?;
    let info = parsed.as_object()?.get("info")?.as_object()?;
    let number = |name: &str| match info.get(name) {
        Some(Value::Num(value)) => Some(*value),
        _ => None,
    };
    Some(KeyInfo { spend: number("spend")?, max_budget: number("max_budget") })
}
