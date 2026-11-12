//! The domain of a run: its input and what it is checked against, the prompt
//! the delegate receives, how its answer, tokens and dollars are read, why it
//! failed, which OpenCode providers it may reach, its id, and the read-only
//! proof that lets it go in place. Pure functions and types, no IO: the
//! launchers and the run command call them.

use crate::config::ModelPrices;
use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::text::{js_slice, js_trim, js_trim_start};
use crate::timestamps::to_iso;
use crate::workflows::require_name;

/// What a run may do: read the repository, or change it in a worktree of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    ReadOnly,
    Write,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RunMode::ReadOnly => "read-only",
            RunMode::Write => "write",
        }
    }
}

/// The clients a delegation can be handed to: Vibe, Antigravity, Claude Code, OpenCode, Hermes and Codex.
pub const RUN_CLIENTS: [&str; 6] = ["vibe", "agy", "claude", "opencode", "hermes", "codex"];

/// One of `RUN_CLIENTS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunClient {
    Vibe,
    Agy,
    Claude,
    Opencode,
    Hermes,
    Codex,
}

impl RunClient {
    pub const ALL: [RunClient; 6] = [RunClient::Vibe, RunClient::Agy, RunClient::Claude, RunClient::Opencode, RunClient::Hermes, RunClient::Codex];

    pub fn as_str(self) -> &'static str {
        RUN_CLIENTS[self as usize]
    }

    pub fn parse(text: &str) -> Option<RunClient> {
        RunClient::ALL.into_iter().find(|client| client.as_str() == text)
    }
}

/// The clients that run on a provider the caller names, and whose provider this
/// machine must allow: Hermes reaches many providers, and the owner decides
/// which of them may be spent on.
pub const PROVIDER_CLIENTS: [&str; 1] = ["hermes"];

/// The providers the gateway never runs on, whatever a machine file says:
/// OpenRouter is forbidden by the owner, so no configuration can turn it on.
pub const FORBIDDEN_PROVIDERS: [&str; 1] = ["openrouter"];

/// A run with no time limit of its own gets this many seconds.
pub const DEFAULT_TIME_LIMIT_S: f64 = 900.0;

/// What a run's prompt carries: code, and history such as commit messages and issue text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Carries {
    pub code: bool,
    pub history: bool,
}

/// One delegation: a named workflow's task handed to one client model. The
/// input is exactly what the caller sends; everything optional has a default
/// the run applies, never a second way to say the same thing. The numbers stay
/// `f64`, as the JSON they came from holds them.
#[derive(Clone, Debug, PartialEq)]
pub struct RunInput {
    pub workflow: String,
    pub client: String,
    pub model: String,
    pub mode: RunMode,
    pub prompt: String,
    pub repo: Option<String>,
    pub carries: Option<Carries>,
    pub time_limit_s: f64,
    pub max_price_usd: Option<f64>,
    /// A writing run's starting point: the ref its worktree is created off.
    pub base: Option<String>,
    /// A writing run's check: a command the gateway runs with `sh -c` in the worktree once the delegate ends.
    pub check: Option<String>,
    /// The provider a client that runs on one must use; the machine file decides which are allowed.
    pub provider: Option<String>,
    /// `false` turns off the retry, the climb and the fall-through; they are on unless the caller says so.
    pub escalate: Option<bool>,
    /// Whether the model's family is binding: it is unless the caller says `false`. A binding delegation is
    /// only ever answered by a model of the family it asked for, and a delegation that no model of the family
    /// can answer fails with its cause; `false` lets the gateway move to another family.
    pub family_binding: Option<bool>,
    /// Whether nobody is there to answer: a headless run is refused for want of a grant like any other,
    /// and its task is then the orchestrator's to keep or split. Nothing else about the run changes.
    pub headless: Option<bool>,
    /// How many attempts the delegation gets in all, the first included; 3 unless the caller says otherwise.
    pub max_attempts: Option<f64>,
    /// The members that sit last in the object the node runner holds, in order: an attempt on a candidate moves
    /// `provider` there, and a reservation adds `max_price_usd` there when the caller named none.
    pub tail: Vec<&'static str>,
}

/// Reads a run's input, or says what is wrong with it. The checks run in the order the node runner runs them,
/// so a input with two faults is refused for the same one.
pub fn read_run_input(raw: &Value) -> Result<RunInput, GatewayError> {
    let Some(raw) = raw.as_object() else {
        return Err(GatewayError::invalid_input("the input must be a JSON object"));
    };
    if raw.get("extras").is_some() {
        return Err(GatewayError::invalid_input("extras are for in-process callers only"));
    }
    let workflow = require_name(raw.get("workflow"), "a run must name its workflow")?;
    let client = match raw.get("client") {
        Some(Value::Str(client)) if RunClient::parse(client).is_some() => client.clone(),
        other => {
            return Err(GatewayError::new(
                "not-supported-yet",
                format!("only {} can run a delegation, not {}", RUN_CLIENTS.join(", "), shown(other)),
            ))
        }
    };
    let mode = match raw.get("mode") {
        Some(Value::Str(mode)) if mode == "read-only" => RunMode::ReadOnly,
        Some(Value::Str(mode)) if mode == "write" => RunMode::Write,
        other => {
            return Err(GatewayError::new(
                "not-supported-yet",
                format!("only read-only and write runs are supported, not {}", shown(other)),
            ))
        }
    };
    let Some(Value::Str(model)) = raw.get("model").filter(|value| value.as_str() != Some("")) else {
        return Err(GatewayError::invalid_input("model must be a non-empty string"));
    };
    let Some(Value::Str(prompt)) = raw.get("prompt").filter(|value| value.as_str() != Some("")) else {
        return Err(GatewayError::invalid_input("prompt must be a non-empty string"));
    };
    let repo = read_optional_string(raw.get("repo"), "repo")?;
    let carries = read_carries(raw.get("carries"))?;
    let time_limit_s = match raw.get("time_limit_s") {
        None => DEFAULT_TIME_LIMIT_S,
        value => required_whole_number(value, "time_limit_s")?,
    };
    if time_limit_s <= 0.0 {
        return Err(GatewayError::invalid_input("time_limit_s must be a whole number of seconds above 0"));
    }
    let max_price_usd = match raw.get("max_price_usd") {
        None => None,
        value => Some(required_number_above_0(value, "max_price_usd")?),
    };
    let (base, check) = read_writing(raw, mode, repo.as_deref())?;
    let provider = read_optional_string(raw.get("provider"), "provider")?;
    let Escalation { escalate, family_binding, max_attempts } = read_escalation(raw)?;
    let headless = match raw.get("headless") {
        None => None,
        Some(Value::Bool(flag)) => Some(*flag),
        Some(_) => return Err(GatewayError::invalid_input("headless must be true or false")),
    };
    Ok(RunInput {
        workflow,
        client,
        model: model.clone(),
        mode,
        prompt: prompt.clone(),
        repo,
        carries,
        time_limit_s,
        max_price_usd,
        base,
        check,
        provider,
        escalate,
        family_binding,
        headless,
        max_attempts,
        tail: Vec::new(),
    })
}

/// The input as the node runner holds it as an object, which is what `input.json` is written from: the members in
/// the order `readRunInput` builds them, those in `tail` last, and the ones the caller left out not at all.
pub fn input_json(input: &RunInput) -> Value {
    let mut members: Vec<(&'static str, Value)> = vec![
        ("client", Value::str(&input.client)),
        ("mode", Value::str(input.mode.as_str())),
        ("model", Value::str(&input.model)),
        ("prompt", Value::str(&input.prompt)),
        ("time_limit_s", Value::Num(input.time_limit_s)),
        ("workflow", Value::str(&input.workflow)),
    ];
    let optional = |key: &'static str, value: Option<Value>| value.map(|value| (key, value));
    let carries = input.carries.map(|carries| json::object(vec![("code", Value::Bool(carries.code)), ("history", Value::Bool(carries.history))]));
    members.extend(
        [
            optional("carries", carries),
            optional("max_price_usd", input.max_price_usd.map(Value::Num)),
            optional("provider", input.provider.as_deref().map(Value::str)),
            optional("repo", input.repo.as_deref().map(Value::str)),
            optional("escalate", input.escalate.map(Value::Bool)),
            optional("family_binding", input.family_binding.map(Value::Bool)),
            optional("max_attempts", input.max_attempts.map(Value::Num)),
            optional("base", input.base.as_deref().map(Value::str)),
            optional("check", input.check.as_deref().map(Value::str)),
            optional("headless", input.headless.map(Value::Bool)),
        ]
        .into_iter()
        .flatten(),
    );
    let (last, mut first): (Vec<_>, Vec<_>) = members.into_iter().partition(|(key, _)| input.tail.contains(key));
    first.extend(input.tail.iter().filter_map(|key| last.iter().find(|(name, _)| name == key).cloned()));
    json::object(first)
}

/// A flag the caller may leave out: `true` or `false`, and nothing else.
fn read_flag(value: Option<&Value>, name: &str) -> Result<Option<bool>, GatewayError> {
    match value {
        None => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(GatewayError::invalid_input(format!("{name} must be true or false"))),
    }
}

/// What a run says about its own escalation, as `read_escalation` reads it.
struct Escalation {
    escalate: Option<bool>,
    family_binding: Option<bool>,
    max_attempts: Option<f64>,
}

/// What a run says about its own escalation: whether it is on, whether the family binds, and how many attempts it may take.
fn read_escalation(raw: &Object) -> Result<Escalation, GatewayError> {
    let escalate = read_flag(raw.get("escalate"), "escalate")?;
    let family_binding = read_flag(raw.get("family_binding"), "family_binding")?;
    let attempts = match raw.get("max_attempts") {
        None => None,
        value => Some(required_whole_number(value, "max_attempts")?),
    };
    if attempts.is_some_and(|attempts| attempts < 1.0) {
        return Err(GatewayError::invalid_input("max_attempts must be a whole number above 0"));
    }
    Ok(Escalation { escalate, family_binding, max_attempts: attempts })
}

/// What a writing run adds to the input: its base and its check, and a
/// repository to work in. A read-only run has neither, and one that names
/// them is refused rather than having them ignored.
fn read_writing(raw: &Object, mode: RunMode, repo: Option<&str>) -> Result<(Option<String>, Option<String>), GatewayError> {
    if mode != RunMode::Write {
        if raw.get("base").is_some() || raw.get("check").is_some() {
            return Err(GatewayError::invalid_input("base and check are for writing runs only"));
        }
        return Ok((None, None));
    }
    if repo.is_none() {
        return Err(GatewayError::new("write-needs-repository", "a writing run changes a repository, so it must name its repo"));
    }
    let Some(Value::Str(base)) = raw.get("base").filter(|value| value.as_str() != Some("")) else {
        return Err(GatewayError::invalid_input("a writing run must name its base, a ref the worktree starts from"));
    };
    let Some(Value::Str(check)) = raw.get("check").filter(|value| value.as_str() != Some("")) else {
        return Err(GatewayError::invalid_input("a writing run must name its check, a command that tests the change"));
    };
    Ok((Some(base.clone()), Some(check.clone())))
}

fn read_optional_string(value: Option<&Value>, name: &str) -> Result<Option<String>, GatewayError> {
    match value {
        None => Ok(None),
        Some(Value::Str(text)) if !text.is_empty() => Ok(Some(text.clone())),
        Some(_) => Err(GatewayError::invalid_input(format!("{name} must be a non-empty string"))),
    }
}

/// What the run claims its prompt carries; the caller marks it, the gateway checks the marks against the policy.
fn read_carries(value: Option<&Value>) -> Result<Option<Carries>, GatewayError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let marks = value.as_object().and_then(|object| match (object.get("code"), object.get("history")) {
        (Some(Value::Bool(code)), Some(Value::Bool(history))) => Some(Carries { code: *code, history: *history }),
        _ => None,
    });
    marks
        .map(Some)
        .ok_or_else(|| GatewayError::invalid_input("carries must name code and history, each true or false"))
}

/// `Number.isInteger`.
fn is_integer(number: f64) -> bool {
    number.is_finite() && number.fract() == 0.0
}

fn required_whole_number(value: Option<&Value>, name: &str) -> Result<f64, GatewayError> {
    match value {
        Some(Value::Num(number)) if is_integer(*number) => Ok(*number),
        _ => Err(GatewayError::invalid_input(format!("{name} must be a whole number"))),
    }
}

fn required_number_above_0(value: Option<&Value>, name: &str) -> Result<f64, GatewayError> {
    match value {
        Some(Value::Num(number)) if number.is_finite() && *number > 0.0 => Ok(*number),
        _ => Err(GatewayError::invalid_input(format!("{name} must be a number above 0"))),
    }
}

/// What a refused input named, as the message shows it.
fn shown(value: Option<&Value>) -> String {
    value.map_or_else(|| "missing".to_string(), json::compact)
}

/// What a writing delegate is told: where it may edit, and that committing is the gateway's caller's.
pub const WRITE_RULE: &str = "Edit only files inside the working directory. Do not commit.";

/// What a writing delegate that commits its own change is told: where it may
/// edit, that it commits the change on the branch it is on, and that pushing is
/// never its to do.
pub const CODEX_WRITE_RULE: &str =
    "Edit only files inside the working directory. When the change is done, commit it on the branch you are on with a Conventional Commits message. Never push.";

/// A client that commits its own change: its rule says so, and the brief the
/// repository keeps for it, when there is one, comes first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Worker {
    pub brief: Option<String>,
}

/// The prompt the delegate receives: the caller's prompt, for a writing run
/// the rule about where it edits, and the instruction that ends its answer
/// with a SUMMARY line the gateway can cut to the summary's limit. A `worker`
/// is a client that commits its own change: its rule says so, and the brief
/// the repository keeps for it comes first, above the caller's prompt.
pub fn wrapped_prompt(prompt: &str, summary_max_chars: f64, mode: RunMode, worker: Option<&Worker>) -> String {
    let writing = mode == RunMode::Write;
    let rule = if writing { format!("\n\n{}", if worker.is_none() { WRITE_RULE } else { CODEX_WRITE_RULE }) } else { String::new() };
    let brief = if writing { worker.and_then(|worker| worker.brief.as_deref()) } else { None };
    let body = match brief {
        None | Some("") => prompt.to_string(),
        Some(brief) => format!("{brief}\n\n---\n\n{prompt}"),
    };
    format!(
        "{body}{rule}\n\nEnd your answer with a line \"SUMMARY:\" followed by at most {} characters that summarise the result.",
        json::number_text(summary_max_chars)
    )
}

/// The run's summary: the text after the delegate's last SUMMARY line, cut at
/// the limit; when it wrote none, the last characters of its final answer.
pub fn summary_of(answer: &str, summary_max_chars: f64) -> String {
    match answer.rfind("SUMMARY:") {
        None => js_slice(answer, -summary_max_chars, None),
        Some(at) => js_slice(js_trim_start(&answer[at + "SUMMARY:".len()..]), 0.0, Some(summary_max_chars)),
    }
}

/// The last assistant message of a Vibe journal, or `None` when stdout holds
/// none: the journal is one JSON array of entries, an assistant message being
/// one whose content joins into its text.
pub fn final_answer_of(stdout: &str) -> Option<String> {
    let Ok(Value::Arr(journal)) = json::parse(stdout) else {
        return None;
    };
    let last = journal.iter().filter_map(Value::as_object).rfind(|entry| {
        entry.get("type").and_then(Value::as_str) == Some("message") && entry.get("role").and_then(Value::as_str) == Some("assistant")
    })?;
    let Some(Value::Arr(content)) = last.get("content") else {
        return None;
    };
    let text: String = content
        .iter()
        .map(|part| part.as_object().and_then(|part| part.get("text")).and_then(Value::as_str).unwrap_or(""))
        .collect();
    (!text.is_empty()).then_some(text)
}

/// The final answer a one-shot client printed: its whole stdout, trimmed, since
/// `hermes -z` prints only the response text. `None` when it printed nothing,
/// which is a run the gateway cannot read.
pub fn one_shot_answer(stdout: &str) -> Option<String> {
    let answer = js_trim(stdout);
    (!answer.is_empty()).then(|| answer.to_string())
}

/// What a Hermes usage report says the run cost and whether it failed.
#[derive(Clone, Debug, PartialEq)]
pub struct HermesUsage {
    pub tokens: RunTokens,
    /// The dollars Hermes estimated, or `None` when it priced nothing.
    pub cost: Option<f64>,
    pub failed: bool,
}

/// A count in a usage block; anything but a finite number from 0 up is no count.
pub(crate) fn count(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Num(number)) if number.is_finite() && *number >= 0.0 => *number,
        _ => 0.0,
    }
}

/// The usage report `hermes -z --usage-file` writes: its token counts, the
/// dollars it estimated, and whether it counts the run as failed. A report that
/// is missing or holds nonsense counts no tokens and no dollars, and a run that
/// reported no failure is not failed.
pub fn hermes_usage(report: Option<&Value>) -> HermesUsage {
    let Some(report) = report.and_then(Value::as_object) else {
        return HermesUsage { cost: None, failed: false, tokens: NO_TOKENS };
    };
    let cost = match report.get("estimated_cost_usd") {
        Some(Value::Num(number)) if number.is_finite() && *number >= 0.0 => Some(*number),
        _ => None,
    };
    HermesUsage {
        cost,
        failed: report.get("failed") == Some(&Value::Bool(true)),
        tokens: RunTokens {
            cached: count(report.get("cache_read_tokens")),
            input: count(report.get("input_tokens")),
            output: count(report.get("output_tokens")),
        },
    }
}

/// The tokens a run used, as Vibe's export counts them; a run that reported none used none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunTokens {
    pub input: f64,
    pub output: f64,
    pub cached: f64,
}

pub const NO_TOKENS: RunTokens = RunTokens { cached: 0.0, input: 0.0, output: 0.0 };

/// The export's token counts, or none when the export holds none.
pub fn run_tokens(exported: Option<&Value>) -> RunTokens {
    let Some(usage) = exported.and_then(Value::as_object).and_then(|object| object.get("usage")).and_then(Value::as_object) else {
        return NO_TOKENS;
    };
    RunTokens {
        cached: count(usage.get("cached_input_tokens")),
        input: count(usage.get("input_tokens")),
        output: count(usage.get("output_tokens")),
    }
}

/// The dollars Vibe's export says it charged; `None` when it priced nothing.
pub fn charged_of(exported: Option<&Value>) -> Option<f64> {
    match exported.and_then(Value::as_object).and_then(|object| object.get("cost_usd")) {
        Some(Value::Num(number)) if number.is_finite() && *number > 0.0 => Some(*number),
        _ => None,
    }
}

/// The run's dollars: the client's own figure when it priced the run, else the
/// tokens times the catalogue price, else 0 when nothing prices it.
pub fn run_cost(exported: Option<&Value>, tokens: RunTokens, prices: Option<&ModelPrices>) -> f64 {
    price_run(charged_of(exported), tokens, prices)
}

/// The run's dollars whatever the client: the figure the client charged when
/// it priced the run, else the tokens times the catalogue price, else 0.
pub fn price_run(charged: Option<f64>, tokens: RunTokens, prices: Option<&ModelPrices>) -> f64 {
    if let Some(charged) = charged {
        return charged;
    }
    let Some(prices) = prices else {
        return 0.0;
    };
    let uncached = (tokens.input - tokens.cached).max(0.0);
    (uncached * prices.input + tokens.cached * prices.cached_input + tokens.output * prices.output) / 1_000_000.0
}

/// The error a client's own structured output carries: the text it printed
/// about it, and the HTTP status where it names one.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientError {
    pub text: String,
    pub status: Option<f64>,
}

/// What a client's own output reported about a run's failure: the failure slug
/// its structured output names, and the error that output carries. Each
/// client's parser in `outputs` fills it in from that client's own output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientReport {
    pub failure: Option<String>,
    pub error: Option<ClientError>,
}

/// How a client's process ended, as far as a failure's reason reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchExit {
    pub exit_code: i32,
    pub timed_out: bool,
    pub stderr: String,
}

/// Why a run failed: a slug the result's `reason` and the ledger's outcome
/// carry. A client that says it was rate-limited (its account's quota
/// included) or refused the credentials gets that slug rather than its exit
/// code or its own failure slug, so availability can find it in the ledger
/// afterwards; one that says the model cannot run for this account at all
/// (`model-unavailable`) is remembered even longer, until a run of the client
/// model succeeds. The client's own report of a failure (`timeout`,
/// `denied-action`, `client-error`) counts when it exited 0, and comes before
/// a missing answer.
///
/// The slug is read from the client's own error channel only: its stderr, the
/// error its structured output carries, and its exit code. Never from the
/// delegate's answer, which is the model's text and may quote any of these
/// words without meaning them.
pub fn run_failure_reason(launch: &LaunchExit, answer: Option<&str>, reported: Option<&ClientReport>) -> Option<String> {
    if launch.timed_out {
        return Some("timeout".to_string());
    }
    if let Some(refusal) = refusal_of(launch, reported.and_then(|report| report.error.as_ref())) {
        return Some(refusal.to_string());
    }
    if launch.exit_code != 0 {
        return Some(format!("exit-{}", launch.exit_code));
    }
    if let Some(failure) = reported.and_then(|report| report.failure.as_ref()) {
        return Some(failure.clone());
    }
    answer.is_none().then(|| "parse-error".to_string())
}

/// The providers an OpenCode run may reach when the machine names none.
pub const DEFAULT_OPENCODE_PROVIDERS: [&str; 1] = ["opencode-go"];

/// OpenCode's own pay-per-token providers (`opencode/<model>` is its Zen). The
/// owner's rule is that a run never reaches one, so no allowlist can open it.
const PAY_PER_TOKEN_PROVIDERS: [&str; 1] = ["opencode"];

/// The provider of an OpenCode model named `<provider>/<model>`; `None` when it names none.
pub fn open_code_provider(model: &str) -> Option<&str> {
    let slash = model.find('/')?;
    (slash != 0 && slash != model.len() - 1).then(|| &model[..slash])
}

/// Whether a run may reach the provider: the allowlist (else `opencode-go` alone) names it and it is not pay-per-token.
pub fn is_provider_allowed(provider: Option<&str>, allowed: Option<&[String]>) -> bool {
    let Some(provider) = provider else {
        return false;
    };
    !PAY_PER_TOKEN_PROVIDERS.contains(&provider)
        && match allowed {
            Some(allowed) => allowed.iter().any(|name| name == provider),
            None => DEFAULT_OPENCODE_PROVIDERS.contains(&provider),
        }
}

/// Refuses an OpenCode model whose provider the machine's
/// `clients.opencode.providers` (else `opencode-go` alone) does not name, and
/// every pay-per-token one whatever it names.
pub fn require_allowed_provider(model: &str, allowed: Option<&[String]>) -> Result<(), GatewayError> {
    let providers = match allowed {
        Some(allowed) => allowed.join(", "),
        None => DEFAULT_OPENCODE_PROVIDERS.join(", "),
    };
    let Some(provider) = open_code_provider(model) else {
        return Err(GatewayError::new(
            "provider-not-allowed",
            format!("{model}: an OpenCode model is named <provider>/<model>, and the allowed providers are {providers}"),
        ));
    };
    if !is_provider_allowed(Some(provider), allowed) {
        return Err(GatewayError::new(
            "provider-not-allowed",
            format!(
                "{model}: the provider {provider} is not allowed (clients.opencode.providers: {providers}), and OpenCode\u{2019}s pay-per-token models are never reached"
            ),
        ));
    }
    Ok(())
}

/// What a client's own error channel and exit code, read as UTF-16 the way a
/// JavaScript regular expression reads them, say about it.
fn said_by(launch: &LaunchExit, error: Option<&ClientError>) -> Vec<u16> {
    let text = error.map_or("", |error| error.text.as_str());
    format!("{}\n{}", launch.stderr, text).encode_utf16().map(|unit| if unit < 128 { u16::from((unit as u8).to_ascii_lowercase()) } else { unit }).collect()
}

/// Whether `needle` (lower case ASCII) stands in `units` at `at`.
fn stands_at(units: &[u16], at: usize, needle: &str) -> bool {
    needle.len() <= units.len().saturating_sub(at) && needle.bytes().zip(&units[at..]).all(|(byte, unit)| u16::from(byte) == *unit)
}

fn contains(units: &[u16], needle: &str) -> bool {
    (0..units.len()).any(|at| stands_at(units, at, needle))
}

/// `.` in a JavaScript regular expression: any unit but a line terminator.
fn is_dot(unit: u16) -> bool {
    !matches!(unit, 0x0a | 0x0d | 0x2028 | 0x2029)
}

/// What a client prints when it is out of allowance for now, its account's quota included:
/// `/rate.?limit|too many requests|quota (exceeded|reached)|resource_exhausted/i`.
fn is_rate_limited(said: &[u16]) -> bool {
    let rate_limit = (0..said.len()).any(|at| {
        stands_at(said, at, "rate") && (stands_at(said, at + 4, "limit") || (said.get(at + 4).copied().is_some_and(is_dot) && stands_at(said, at + 5, "limit")))
    });
    rate_limit || ["too many requests", "quota exceeded", "quota reached", "resource_exhausted"].iter().any(|needle| contains(said, needle))
}

/// What a client prints when the account may not use the model it named at all, however long it waits: `/is hosted outside/i`.
fn is_model_unavailable(said: &[u16]) -> bool {
    contains(said, "is hosted outside")
}

/// What a client prints when it will not run without a login or a valid key:
/// `/unauthori[sz]ed|unauthenticated|not logged in|log ?in required|invalid api key|authentication failed|forbidden/i`.
fn is_refused_credentials(said: &[u16]) -> bool {
    [
        "unauthorised",
        "unauthorized",
        "unauthenticated",
        "not logged in",
        "login required",
        "log in required",
        "invalid api key",
        "authentication failed",
        "forbidden",
    ]
    .iter()
    .any(|needle| contains(said, needle))
}

/// What a client's own error channel says refused the run, when it says so.
fn refusal_of(launch: &LaunchExit, error: Option<&ClientError>) -> Option<&'static str> {
    let said = said_by(launch, error);
    let status = error.and_then(|error| error.status);
    if is_model_unavailable(&said) {
        return Some("model-unavailable");
    }
    if is_rate_limited(&said) || status == Some(429.0) || launch.exit_code == 429 {
        return Some("rate-limited");
    }
    if is_refused_credentials(&said) || status == Some(401.0) || status == Some(403.0) || launch.exit_code == 401 || launch.exit_code == 403 {
        return Some("auth");
    }
    None
}

/// The latest moment a JavaScript `Date` holds, in milliseconds.
const MAX_DATE_MS: f64 = 8.64e15;

/// When the client's own error channel said its client model is usable again:
/// the reset its refusal named (`Resets in 29h2m9s`), measured from the run's
/// start (`at_ms`, epoch milliseconds). A refusal that names no reset gives
/// none, so the gateway remembers it only until a run of the client model
/// succeeded. A reset past the last moment a `Date` holds also gives none.
pub fn named_reset(launch: &LaunchExit, error: Option<&ClientError>, at_ms: i64) -> Option<i64> {
    let said = said_by(launch, error);
    // /resets? in (?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?/i, the first place it matches.
    let after = (0..said.len()).find_map(|at| {
        if !stands_at(&said, at, "reset") {
            return None;
        }
        let at = at + if stands_at(&said, at + 5, "s") { 6 } else { 5 };
        stands_at(&said, at, " in ").then_some(at + 4)
    })?;
    let mut at = after;
    let mut parts = [0.0f64; 3];
    for (part, unit) in parts.iter_mut().zip(["h", "m", "s"]) {
        let digits = said[at..].iter().take_while(|digit| (u16::from(b'0')..=u16::from(b'9')).contains(digit)).count();
        if digits > 0 && stands_at(&said, at + digits, unit) {
            let number: String = said[at..at + digits].iter().map(|digit| char::from(*digit as u8)).collect();
            *part = number.parse().unwrap_or(0.0);
            at += digits + 1;
        }
    }
    let ms = parts[0] * 3_600_000.0 + parts[1] * 60_000.0 + parts[2] * 1000.0;
    let reset = at_ms as f64 + ms;
    (ms != 0.0 && reset <= MAX_DATE_MS).then_some(reset as i64)
}

/// A UTC moment as a run id's first half: `20261104T105215Z`.
pub fn compact_timestamp(at_ms: i64) -> String {
    let iso: String = to_iso(at_ms).chars().filter(|c| !matches!(c, '-' | ':')).collect();
    // The milliseconds are the one `.` and three digits.
    let (head, tail) = iso.split_at(iso.find('.').unwrap_or(iso.len()));
    format!("{head}{}", tail.get(4..).unwrap_or(""))
}

/// A run's id: the UTC moment it started, and a short suffix that keeps two runs of one second apart.
pub fn run_id(at_ms: i64, suffix: &str) -> String {
    format!("{}-{suffix}", compact_timestamp(at_ms))
}

/// Whether a client model has a read-only proof that passed on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOnlyProof {
    Proven { critic: String },
    Unproven { reason: String },
}

/// The files setup and critique leave on this machine, as they were read; any of them may be missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProofFiles {
    /// Critique's routing: the critic clients, each with the model it runs.
    pub routing: Option<String>,
    /// This machine's routing override, merged over the shipped routing client by client.
    pub routing_override: Option<String>,
    /// Setup's recorded results, one per check id.
    pub setup_results: Option<String>,
}

/// The critic client kind whose read-only proof counts for a client, and what
/// the messages call it. Only Vibe and Antigravity have a critic: the others
/// (Claude Code, OpenCode, Codex, Hermes) are never proven.
fn critic_kind(client: &str) -> Option<(&'static str, &'static str)> {
    match client {
        "agy" => Some(("agy", "Antigravity")),
        "vibe" => Some(("vibe", "Vibe")),
        _ => None,
    }
}

/// The read-only proof of a client model: setup's `read-only:<critic client>`
/// result for the critic client of the client's own kind (Vibe, or Antigravity
/// as the Gemini critic) whose model this is in critique's routing. A run names
/// a model by the alias `models` prints, while the routing names it by the
/// model's own id, so a critic client matches on the alias or on any name the
/// catalogue keeps for it. A client with no critic kind, a model no critic
/// client runs, and a proof that is missing, failed or unproven, are all
/// unproven: a run goes in place only on a proof that passed.
pub fn read_only_proof(files: &ProofFiles, model: &str, names: &[&str], client: &str) -> ReadOnlyProof {
    let Some((kind, label)) = critic_kind(client) else {
        return unproven(format!("{client} has no critic client in critique's routing, so nothing proves it read-only"));
    };
    let Some(routing) = parse_json_object(files.routing.as_deref()) else {
        return unproven(format!("critique's routing is not installed, so no {label} critic client proves a model read-only"));
    };
    let routing_override = parse_json_object(files.routing_override.as_deref());
    let models: Vec<&str> = std::iter::once(model).chain(names.iter().copied()).collect();
    let Some(name) = critic_for(routing.get("clients"), routing_override.as_ref().and_then(|object| object.get("clients")), kind, &models) else {
        return unproven(format!("no {label} critic client in critique's routing runs {model}"));
    };
    let results = parse_json_object(files.setup_results.as_deref());
    let result = check_result(results.as_ref().and_then(|object| object.get("checks")), &format!("read-only:{name}"));
    if result.as_deref() != Some("pass") {
        return unproven(format!("setup's read-only proof for {name} is {}", result.as_deref().unwrap_or("not recorded")));
    }
    ReadOnlyProof::Proven { critic: name }
}

fn unproven(reason: String) -> ReadOnlyProof {
    ReadOnlyProof::Unproven { reason }
}

/// The first critic client of a kind, in name order, whose model is one of `models`, with the override applied over the shipped routing.
fn critic_for(routing: Option<&Value>, over: Option<&Value>, kind: &str, models: &[&str]) -> Option<String> {
    let mut clients: Vec<(String, Object)> = Vec::new();
    for source in [routing, over] {
        let Some(source) = source.and_then(Value::as_object) else {
            continue;
        };
        for (name, entry) in source.entries() {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            match clients.iter_mut().find(|(known, _)| known == name) {
                Some((_, merged)) => {
                    for (key, value) in entry.entries() {
                        merged.set(key, value.clone());
                    }
                }
                None => clients.push((name.to_string(), entry.clone())),
            }
        }
    }
    let mut names: Vec<&str> = clients
        .iter()
        .filter(|(_, entry)| {
            entry.get("kind").and_then(Value::as_str) == Some(kind)
                && entry.get("model").and_then(Value::as_str).is_some_and(|model| models.contains(&model))
        })
        .map(|(name, _)| name.as_str())
        .collect();
    // `Array.prototype.sort` orders strings by UTF-16 code unit.
    names.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
    names.first().map(|name| (*name).to_string())
}

/// One check's recorded result, or `None` when setup recorded none.
fn check_result(checks: Option<&Value>, id: &str) -> Option<String> {
    let check = checks?.as_object()?.get(id)?.as_object()?;
    check.get("result").and_then(Value::as_str).map(str::to_string)
}

/// Parsed JSON that is an object, or `None` when the text is missing or is not one.
fn parse_json_object(text: Option<&str>) -> Option<Object> {
    match json::parse(text?) {
        Ok(Value::Obj(object)) => Some(object),
        _ => None,
    }
}
