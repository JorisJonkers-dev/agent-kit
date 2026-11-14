//! What the budget reads back from the ledger: one critic launch, and the spend
//! and agy count a flow's cap is measured with.

use crate::fields::Fields;
use model_gateway::json;
use model_gateway::timestamps::to_iso;

/// The clients a launch can go through.
pub const KINDS: [&str; 5] = ["vibe", "agy", "codex", "claude", "pi"];

/// What the budget reads back from one invocation line, and the run it was
/// launched for; setup's checks launch for no run.
#[derive(Clone, Debug, PartialEq)]
pub struct LoggedInvocation {
    pub recorded_at: String,
    pub repository: String,
    pub flow: String,
    pub kind: String,
    pub charged_usd: f64,
    pub critic_id: String,
    pub run_id: Option<String>,
}

/// The invocation lines of a ledger. Any other line, and a line that is not a
/// JSON object of the right shape (a write cut short, say), is skipped.
pub fn invocation_lines(ledger: &str) -> Vec<LoggedInvocation> {
    ledger.split('\n').filter_map(read_invocation).collect()
}

fn read_invocation(text: &str) -> Option<LoggedInvocation> {
    let value = json::parse(text).ok()?;
    let line = Fields::of(&value, "").ok()?;
    if !line.has("type") || line.string("type").ok()? != "invocation" {
        return None;
    }
    Some(LoggedInvocation {
        charged_usd: line.number("charged_usd").ok()?,
        critic_id: line.string("critic_id").ok()?,
        flow: line.string("flow").ok()?,
        kind: line.one_of("kind", &KINDS).ok()?,
        recorded_at: line.string("recorded_at").ok()?,
        repository: line.string("repository").ok()?,
        run_id: line.optional_string("run_id").ok()?,
    })
}

/// Everything the Vibe critics of one flow of one repository were charged.
pub fn flow_spend(lines: &[LoggedInvocation], repository: &str, flow: &str) -> f64 {
    lines
        .iter()
        .filter(|line| line.kind == "vibe" && line.repository == repository && line.flow == flow)
        .fold(0.0, |total, line| total + line.charged_usd)
}

/// A feature's Vibe spend under one of its flow keys.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowSpend {
    pub flow: String,
    pub spent_usd: f64,
}

/// What each of a feature's flow keys was charged, in the order of `flows`: the
/// spec's flow, then its draft's.
pub fn spend_by_flow(lines: &[LoggedInvocation], repository: &str, flows: &[String]) -> Vec<FlowSpend> {
    flows.iter().map(|flow| FlowSpend { flow: flow.clone(), spent_usd: flow_spend(lines, repository, flow) }).collect()
}

/// The UTC day of `now`, as `YYYY-MM-DD`: the day the Antigravity quota counts.
pub fn utc_day(now_ms: i64) -> String {
    to_iso(now_ms).chars().take(10).collect()
}

/// agy launches on this machine on the UTC day of `now`.
pub fn agy_invocations_on(lines: &[LoggedInvocation], now_ms: i64) -> usize {
    let day = utc_day(now_ms);
    lines.iter().filter(|line| line.kind == "agy" && line.recorded_at.chars().take(10).eq(day.chars())).count()
}
