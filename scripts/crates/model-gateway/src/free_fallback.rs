//! A run on a LiteLLM free model (`free/<model>`) that fails because no free copy of it is available falls
//! through once to OpenCode's own free copy of the same model (`opencode/<model>-free`), and says so: that it fell
//! back, when the free copies are available again, and what the next calls should name instead (ADR-0018 keeps a
//! pinned client from ever being swapped, so a run that named its client never falls back).
//!
//! The fallback stays in the free cost class, so it does not depend on `escalate`; `free_fallback: false` turns
//! it off. It runs only where the data policy lets a public free model carry the run's content: a public
//! repository, or inline content the caller declares public.

use crate::config::GatewayConfig;
use crate::config_parse::load_config;
use crate::error::GatewayError;
use crate::escalation::carries;
use crate::go_route::opencode_offers;
use crate::json::{self, Value};
use crate::models::family_of_model_id;
use crate::policy::{decide_policy, resolve_scope, DataScope, PolicySubject};
use crate::ports::Runtime;
use crate::run::{is_pi_client, opencode_litellm_group, RunInput};
use crate::timestamps::to_iso;

/// What the fallback tells the caller: which run it replaced, what replaced it, why, and when the free copies
/// are back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackReport {
    /// `<client> <model>` of the run that found no free copy.
    pub from: String,
    /// `<client> <model>` the run fell back to.
    pub to: String,
    /// The refusal the first run ended with, as its client printed it.
    pub reason: String,
    /// The run id of the refused run.
    pub from_run_id: String,
    /// Seconds until a free copy is available again, when the refusal says.
    pub available_again_in_s: Option<i64>,
    /// The moment a free copy is available again (UTC, ISO 8601), when the refusal says.
    pub available_at: Option<String>,
    /// What the next calls should name.
    pub recommendation: String,
}

impl FallbackReport {
    /// The `fallback` object of a result and of the run's event.
    pub fn to_json(&self) -> Value {
        json::object(vec![
            ("from", Value::str(&self.from)),
            ("from_run_id", Value::str(&self.from_run_id)),
            ("reason", Value::str(&self.reason)),
            ("to", Value::str(&self.to)),
        ])
    }

    /// The members a result gains beside `fallback`: when the free copies are back, and the recommendation.
    pub fn members(&self) -> Vec<(&'static str, Value)> {
        let mut members = vec![("fallback", self.to_json())];
        members.extend(self.available_again_in_s.map(|seconds| ("available_again_in_s", Value::Num(seconds as f64))));
        members.extend(self.available_at.as_deref().map(|at| ("available_at", Value::str(at))));
        members.push(("recommendation", Value::str(&self.recommendation)));
        members
    }

    /// What the event of the run that fell back says: every member of the result.
    pub fn event(&self) -> Value {
        json::object(self.members())
    }

    /// The line the summary opens with.
    pub fn notice(&self) -> String {
        let back = match (&self.available_at, self.available_again_in_s) {
            (Some(at), Some(seconds)) => format!("; free copies are back in {seconds}s (at {at})"),
            _ => "; the refusal names no time the free copies are back".to_string(),
        };
        format!("fell back from {} to {}: {}{back}. {}", self.from, self.to, self.reason, self.recommendation)
    }
}

/// How much of a refusal a report keeps.
const REASON_CHARS: usize = 400;

/// What a failed run's own words say when no free copy of its model is available: the proxy's limiter refusal, a
/// daily limit (a 402 or 429 whose text says the plan's allowance is used), every copy blocked, or a rate limit of the client itself. The first text that says so, else
/// the first text of a rate-limited run; `None` for any other failure, which falls back to nothing.
pub fn refusal_in(reason: Option<&str>, texts: &[&str]) -> Option<String> {
    const MARKERS: [&str; 10] = ["limiter refusal", "every free", "all free", "daily limit", "daily quota", "no free copy", "free plan limit", "all used", "402:", "429:"];
    let said = |text: &str| {
        let lower = text.to_ascii_lowercase();
        MARKERS.iter().any(|marker| lower.contains(marker))
    };
    let found = texts.iter().find(|text| said(text)).or_else(|| texts.iter().find(|text| reason == Some("rate-limited") && !text.trim().is_empty()));
    match found {
        Some(text) => Some(text.trim().chars().take(REASON_CHARS).collect()),
        None => (reason == Some("rate-limited")).then(|| "rate-limited".to_string()),
    }
}

/// The seconds a duration such as `3600s`, `2h3m` or `45` names at the start of `text`; a bare number counts only
/// when `bare_is_seconds`.
fn duration_s(text: &str, bare_is_seconds: bool) -> Option<i64> {
    let mut rest = text.trim_start();
    let mut total = 0;
    let mut parts = 0;
    loop {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            break;
        }
        let number: i64 = rest[..digits].parse().ok()?;
        let tail = &rest[digits..];
        let (seconds, used) = match tail.as_bytes().first() {
            Some(b'h') => (number * 3600, 1),
            Some(b'm') => (number * 60, 1),
            Some(b's') => (number, 1),
            _ if bare_is_seconds && parts == 0 => (number, 0),
            _ => break,
        };
        total += seconds;
        parts += 1;
        rest = &tail[used..];
        if used == 0 {
            break;
        }
    }
    (total > 0).then_some(total)
}

/// The text after the first of `needles`, lower-cased input.
fn after<'a>(lower: &'a str, needles: &[&str]) -> Option<&'a str> {
    needles.iter().filter_map(|needle| lower.find(needle).map(|at| &lower[at + needle.len()..])).next()
}

/// The seconds from `now_ms` to the next `HH:MM` UTC the text names after `resets at`.
fn until_clock(text: &str, now_ms: i64) -> Option<i64> {
    let digits = |from: &str, count: usize| -> Option<i64> { from.get(..count).filter(|part| part.bytes().all(|byte| byte.is_ascii_digit())).and_then(|part| part.parse().ok()) };
    let text = text.trim_start();
    let (hour, minute) = (digits(text, 2)?, digits(text.get(3..)?, 2)?);
    if text.as_bytes().get(2) != Some(&b':') || hour > 23 || minute > 59 {
        return None;
    }
    const DAY_MS: i64 = 86_400_000;
    let wanted = (hour * 60 + minute) * 60_000;
    let ahead = (wanted - now_ms.rem_euclid(DAY_MS)).rem_euclid(DAY_MS);
    Some(if ahead == 0 { DAY_MS } else { ahead } / 1000)
}

/// When a refusal says the free copies are back, from `now_ms`: its `back in 3600s`, `Retry-After: 120`,
/// `resets in 2h3m` or `resets at 00:00 UTC`, the first of them it names. Seconds and the moment as ISO 8601.
pub fn available_after(text: &str, now_ms: i64) -> (Option<i64>, Option<String>) {
    let lower = text.to_ascii_lowercase();
    let seconds = after(&lower, &["back in "])
        .and_then(|rest| duration_s(rest, false))
        .or_else(|| after(&lower, &["retry-after:", "retry-after "]).and_then(|rest| duration_s(rest, true)))
        .or_else(|| after(&lower, &["resets in ", "reset in "]).and_then(|rest| duration_s(rest, false)))
        .or_else(|| after(&lower, &["resets at ", "reset at "]).and_then(|rest| until_clock(rest, now_ms)));
    (seconds, seconds.map(|seconds| to_iso(now_ms + seconds * 1000)))
}

/// The canonical model a free LiteLLM address names: Pi's `free/<model>`, or the same group through OpenCode's
/// own LiteLLM provider. `None` for any other model, a LiteLLM tier included, since a tier is a chain the proxy
/// walks and names no one model.
pub fn free_group<'a>(client: &str, model: &'a str) -> Option<&'a str> {
    let group = if is_pi_client(client) { Some(model) } else { opencode_litellm_group(client, model) };
    group.and_then(|group| group.strip_prefix("free/")).filter(|canonical| !canonical.is_empty())
}

/// Whether the data policy lets `target` (a public free model) carry what the run carries in this scope: a public
/// repository, or inline content the caller declares public. A private or nedap repository never does.
pub fn allows(config: &GatewayConfig, scope: &DataScope, input: &RunInput, target: &str, family: &str) -> bool {
    let subject = PolicySubject { client: "opencode".to_string(), model: target.to_string(), family: family.to_string() };
    decide_policy(config, scope, &[subject]).permissions.first().is_some_and(|permission| carries(permission, input))
}

/// The run to try instead of `input`, which found no free copy (`refusal` is what it said), and what to tell the
/// caller; `None` when it may not fall back: it turned the fallback off or named its client, it is not on a free
/// LiteLLM model, OpenCode offers no free copy of the same model, or the data policy keeps its content from a
/// public free model.
pub fn plan(runtime: &Runtime, input: &RunInput, from_run_id: &str, refusal: &str) -> Result<Option<(RunInput, FallbackReport)>, GatewayError> {
    if input.free_fallback == Some(false) || input.client_pinned == Some(true) {
        return Ok(None);
    }
    let Some(canonical) = free_group(&input.client, &input.model) else {
        return Ok(None);
    };
    let target = format!("opencode/{canonical}-free");
    let config = load_config(runtime)?;
    if !config.clients.contains_key("opencode") || config.disabled.contains(&format!("opencode:{target}")) || !opencode_offers(runtime, &target) {
        return Ok(None);
    }
    // The same model under another name is the same family, so the family stays binding.
    let family = config.models.iter().find(|entry| entry.client == input.client && entry.model == input.model).map(|entry| entry.family.as_str()).or_else(|| family_of_model_id(canonical)).unwrap_or_default();
    if !allows(&config, &resolve_scope(runtime, input.repo.as_deref())?, input, &target, family) {
        return Ok(None);
    }
    let now = runtime.clock.now_ms();
    let (available_again_in_s, available_at) = available_after(refusal, now);
    let when = available_at.as_deref().unwrap_or("an unknown time");
    let report = FallbackReport {
        from: format!("{} {}", input.client, input.model),
        to: format!("opencode {target}"),
        reason: refusal.to_string(),
        from_run_id: from_run_id.to_string(),
        available_again_in_s,
        available_at: available_at.clone(),
        recommendation: format!("{} has no free copy until {when}; name opencode:{target} (public data) or tier-free for the next calls", input.model),
    };
    let fallen = RunInput { client: "opencode".to_string(), model: target, provider: None, tail: Vec::new(), ..input.clone() };
    Ok(Some((fallen, report)))
}
