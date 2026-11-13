//! The runner's own record of what critics cost: every launch is appended to
//! the user's ledger as an invocation line as soon as it ends, and to the model
//! gateway's ledger as one record of the run, so the gateway counts a critic run
//! the way it counts a delegation.

use crate::budget::{agy_invocations_on, spend_by_flow, FlowSpend, LoggedInvocation};
use crate::error::CritiqueError;
use crate::ledger_source::LedgerSource;
use crate::runtime::Runtime;
use model_gateway::json::{compact, Object, Value};
use model_gateway::ledger_file::LedgerStore;
use model_gateway::text::dirname;
use model_gateway::timestamps::to_iso;

/// The runner's own record of what critics cost.
pub struct FlowBudget<'a> {
    runtime: &'a Runtime,
}

impl<'a> FlowBudget<'a> {
    pub fn new(runtime: &'a Runtime) -> FlowBudget<'a> {
        FlowBudget { runtime }
    }

    /// Records one launch: critique's own invocation line, and the gateway's
    /// record of the run, which carries the gateway's fields beside critique's.
    /// `spec` is the spec the run belongs to, where it has one. A gateway ledger
    /// that will not take the record is not the stage's failure — the critics
    /// have already run and spent money — so the message comes back for the
    /// stage's summary instead of failing.
    pub fn log_invocation(
        &self,
        manifest: &InvocationContext,
        run: &LedgerRun,
        client: &LedgerClient,
        spec: Option<&str>,
    ) -> Result<Option<String>, CritiqueError> {
        let at = to_iso(self.runtime.clock.now_ms());
        let line = InvocationLine {
            r#type: "invocation".to_string(),
            recorded_at: at.clone(),
            repository: manifest.repository.clone(),
            flow: manifest.flow.clone(),
            run_id: manifest.run_id.clone(),
            stage: manifest.stage.clone(),
            round: manifest.round,
            critic_id: run.critic_id.clone(),
            client: run.client.clone(),
            kind: client.kind.clone(),
            family: run.family.clone(),
            model: run.model.clone(),
            status: run.status.clone(),
            charged_usd: run.charged_usd,
            vibe_cost_usd: run.vibe_cost_usd,
        };
        let path = crate::locations::ledger_path(&self.runtime.env)?;
        self.runtime.files.mkdir_all(dirname(&path)).map_err(CritiqueError::from_fs)?;
        self.runtime.files.append(&path, &format!("{}\n", compact(&line.to_json()))).map_err(CritiqueError::from_fs)?;
        let context = CritiqueLedgerContext {
            flow: manifest.flow.clone(),
            repository: manifest.repository.clone(),
            round: manifest.round,
            run_id: manifest.run_id.clone(),
            spec: spec.map(str::to_string),
            stage: manifest.stage.clone(),
        };
        let record = critique_ledger_record(&context, run, client, &at);
        match (LedgerStore { runtime: &self.runtime.gateway }).append_run(&record.to_json()) {
            Ok(()) => Ok(None),
            Err(error) => Ok(Some(error.message)),
        }
    }

    /// What the Vibe critics were charged under each of a feature's flow keys.
    pub fn vibe_spend_by_flow(&self, repository: &str, flows: &[String]) -> Result<Vec<FlowSpend>, CritiqueError> {
        Ok(spend_by_flow(&self.lines()?, repository, flows))
    }

    /// agy launches on this machine today.
    pub fn agy_invocations_today(&self) -> Result<usize, CritiqueError> {
        Ok(agy_invocations_on(&self.lines()?, self.runtime.clock.now_ms()))
    }

    /// Every launch both ledgers hold, which is what the spend and the agy count are read from.
    fn lines(&self) -> Result<Vec<LoggedInvocation>, CritiqueError> {
        Ok(LedgerSource::new(self.runtime).read()?.invocations)
    }
}

/// The run a launch belongs to, as critique's own line names it.
#[derive(Clone, Debug, PartialEq)]
pub struct InvocationContext {
    pub flow: String,
    pub repository: String,
    pub round: f64,
    pub run_id: String,
    pub stage: String,
}

/// What a critic run needs to become the gateway's record: the run it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct CritiqueLedgerContext {
    pub flow: String,
    pub repository: String,
    pub round: f64,
    pub run_id: String,
    pub spec: Option<String>,
    pub stage: String,
}

/// The critic client a run went through, as the gateway's record names it.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerClient {
    pub kind: String,
    /// The gateway client model the critic client is pinned to; without it the client runs its own model.
    pub gateway: Option<GatewayClient>,
}

/// The gateway client model a critic client is pinned to.
#[derive(Clone, Debug, PartialEq)]
pub struct GatewayClient {
    pub client: String,
    pub model: String,
}

/// What one critic run says about itself, as the gateway's record carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerRun {
    pub charged_usd: f64,
    pub client: String,
    pub cost_usd: Option<f64>,
    pub critic_id: String,
    pub exit_code: Option<f64>,
    pub family: String,
    pub model: String,
    pub status: String,
    pub duration_ms: Option<f64>,
    pub failure: Option<String>,
    pub finding_count: Option<f64>,
    pub lens_hashes: Option<Vec<(String, String)>>,
    pub lenses: Option<Vec<String>>,
    pub prompt_hashes: Option<PromptHashes>,
    pub retry_of: Option<String>,
    pub role: Option<String>,
    pub tokens: Option<CriticTokens>,
    /// `undefined` when the client reported nothing at all, `null` when it reported no cost.
    pub vibe_cost_usd: Option<Option<f64>>,
}

impl LedgerRun {
    /// A critic's run as the gateway's record carries it.
    pub fn of(run: &crate::critic::CriticRun) -> LedgerRun {
        LedgerRun {
            charged_usd: run.charged_usd,
            client: run.client.clone(),
            cost_usd: run.cost_usd,
            critic_id: run.critic_id.clone(),
            exit_code: run.exit_code,
            family: run.family.clone(),
            model: run.model.clone(),
            status: run.status.clone(),
            duration_ms: Some(run.duration_ms),
            failure: run.failure.clone(),
            finding_count: Some(run.finding_count),
            lens_hashes: Some(run.lens_hashes.clone()),
            lenses: Some(run.lenses.clone()),
            prompt_hashes: Some(run.prompt_hashes.clone()),
            retry_of: run.retry_of.clone(),
            role: Some(run.role.clone()),
            tokens: run.tokens.clone(),
            vibe_cost_usd: run.vibe_cost_usd,
        }
    }
}

impl LedgerClient {
    /// The critic client a run went through.
    pub fn of(config: &crate::routing::CriticClient) -> LedgerClient {
        LedgerClient {
            kind: config.kind().to_string(),
            gateway: config.gateway().map(|gateway| GatewayClient { client: gateway.client.clone(), model: gateway.model.clone() }),
        }
    }
}

pub use crate::clients::CriticTokens;

/// The prompt files one critic was given, by the hash of each.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptHashes {
    pub preamble: String,
    pub role: String,
    pub contract: String,
    pub schema: String,
}

/// One critic launch, appended to the ledger as soon as it ends, so the spend
/// and the agy count stay true even for a run that is never recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct InvocationLine {
    pub r#type: String,
    pub recorded_at: String,
    pub repository: String,
    pub flow: String,
    pub run_id: String,
    pub stage: String,
    pub round: f64,
    pub critic_id: String,
    pub client: String,
    pub kind: String,
    pub family: String,
    pub model: String,
    pub status: String,
    /// What the launch counts against the flow's Vibe cap; always 0 for agy.
    pub charged_usd: f64,
    /// What Vibe itself reported, beside the cost the runner computed; absent for agy.
    pub vibe_cost_usd: Option<Option<f64>>,
}

impl InvocationLine {
    /// The line as the ledger holds it: `JSON.stringify` order, `vibe_cost_usd` left out when it is undefined.
    pub fn to_json(&self) -> Value {
        let mut object = Object::new();
        object.set("charged_usd", Value::Num(self.charged_usd));
        object.set("client", Value::str(&self.client));
        object.set("critic_id", Value::str(&self.critic_id));
        object.set("family", Value::str(&self.family));
        object.set("flow", Value::str(&self.flow));
        object.set("kind", Value::str(&self.kind));
        object.set("model", Value::str(&self.model));
        object.set("recorded_at", Value::str(&self.recorded_at));
        object.set("repository", Value::str(&self.repository));
        object.set("round", Value::Num(self.round));
        object.set("run_id", Value::str(&self.run_id));
        object.set("stage", Value::str(&self.stage));
        object.set("status", Value::str(&self.status));
        object.set("type", Value::str(&self.r#type));
        set_optional(&mut object, "vibe_cost_usd", self.vibe_cost_usd.map(number_or_null));
        Value::Obj(object)
    }
}

/// One critic run as the gateway's ledger holds it: the gateway's own fields
/// plus everything critique's own line carries.
#[derive(Clone, Debug, PartialEq)]
pub struct CritiqueLedgerRecord {
    pub at: String,
    pub charged_usd: f64,
    pub client: String,
    pub cost_usd: f64,
    pub critic_client: String,
    pub critic_cost_usd: Option<f64>,
    pub critic_id: String,
    pub critic_model: String,
    pub duration_s: f64,
    pub exit_code: Option<f64>,
    pub family: String,
    pub failure: Option<String>,
    pub finding_count: Option<f64>,
    pub kind: String,
    pub lens_hashes: Option<Vec<(String, String)>>,
    pub lenses: Option<Vec<String>>,
    pub mode: String,
    pub model: String,
    pub outcome: String,
    pub prompt_hashes: Option<PromptHashes>,
    pub reason: Option<String>,
    pub recorded_at: String,
    pub repository: String,
    pub retry_of: Option<String>,
    pub role: Option<String>,
    pub round: f64,
    pub run_id: String,
    pub spec: Option<String>,
    pub stage: String,
    pub status: String,
    pub tokens: Option<CriticTokens>,
    pub r#type: String,
    pub vibe_cost_usd: Option<Option<f64>>,
    pub workflow: String,
}

impl CritiqueLedgerRecord {
    /// The record as the gateway's ledger line: `JSON.stringify` order, a field the run did not report left out.
    pub fn to_json(&self) -> Value {
        let mut object = Object::new();
        object.set("at", Value::str(&self.at));
        object.set("charged_usd", Value::Num(self.charged_usd));
        object.set("client", Value::str(&self.client));
        object.set("cost_usd", Value::Num(self.cost_usd));
        object.set("critic_client", Value::str(&self.critic_client));
        object.set("critic_cost_usd", number_or_null(self.critic_cost_usd));
        object.set("critic_id", Value::str(&self.critic_id));
        object.set("critic_model", Value::str(&self.critic_model));
        object.set("duration_s", Value::Num(self.duration_s));
        object.set("exit_code", number_or_null(self.exit_code));
        object.set("family", Value::str(&self.family));
        set_optional(&mut object, "failure", self.failure.as_deref().map(Value::str));
        set_optional(&mut object, "finding_count", self.finding_count.map(Value::Num));
        object.set("kind", Value::str(&self.kind));
        set_optional(&mut object, "lens_hashes", self.lens_hashes.as_deref().map(string_map));
        set_optional(&mut object, "lenses", self.lenses.as_deref().map(string_array));
        object.set("mode", Value::str(&self.mode));
        object.set("model", Value::str(&self.model));
        object.set("outcome", Value::str(&self.outcome));
        set_optional(&mut object, "prompt_hashes", self.prompt_hashes.as_ref().map(PromptHashes::to_json));
        set_optional(&mut object, "reason", self.reason.as_deref().map(Value::str));
        object.set("recorded_at", Value::str(&self.recorded_at));
        object.set("repository", Value::str(&self.repository));
        set_optional(&mut object, "retry_of", self.retry_of.as_deref().map(Value::str));
        set_optional(&mut object, "role", self.role.as_deref().map(Value::str));
        object.set("round", Value::Num(self.round));
        object.set("run_id", Value::str(&self.run_id));
        set_optional(&mut object, "spec", self.spec.as_deref().map(Value::str));
        object.set("stage", Value::str(&self.stage));
        object.set("status", Value::str(&self.status));
        set_optional(&mut object, "tokens", self.tokens.as_ref().map(CriticTokens::to_json));
        object.set("type", Value::str(&self.r#type));
        set_optional(&mut object, "vibe_cost_usd", self.vibe_cost_usd.map(number_or_null));
        object.set("workflow", Value::str(&self.workflow));
        Value::Obj(object)
    }
}

impl CriticTokens {
    /// The tokens as the gateway's record carries them.
    pub fn to_json(&self) -> Value {
        let mut object = Object::new();
        object.set("cached", Value::Num(self.cached));
        object.set("input", Value::Num(self.input));
        object.set("output", Value::Num(self.output));
        Value::Obj(object)
    }
}

impl PromptHashes {
    /// The hashes as the gateway's record carries them.
    pub fn to_json(&self) -> Value {
        let mut object = Object::new();
        object.set("contract", Value::str(&self.contract));
        object.set("preamble", Value::str(&self.preamble));
        object.set("role", Value::str(&self.role));
        object.set("schema", Value::str(&self.schema));
        Value::Obj(object)
    }
}

/// One critic run as the gateway's ledger line. The gateway's `reason` is its
/// own vocabulary, so only a failure it names is carried over; every other
/// failure stays in critique's `failure`.
pub fn critique_ledger_record(context: &CritiqueLedgerContext, run: &LedgerRun, client: &LedgerClient, at: &str) -> CritiqueLedgerRecord {
    let gateway = client.gateway.as_ref();
    CritiqueLedgerRecord {
        at: at.to_string(),
        charged_usd: run.charged_usd,
        client: gateway.map_or_else(|| client.kind.clone(), |gateway| gateway.client.clone()),
        cost_usd: run.charged_usd,
        critic_client: run.client.clone(),
        critic_cost_usd: run.cost_usd,
        critic_id: run.critic_id.clone(),
        critic_model: run.model.clone(),
        duration_s: run.duration_ms.unwrap_or(0.0) / 1000.0,
        exit_code: run.exit_code,
        family: run.family.clone(),
        failure: run.failure.clone(),
        finding_count: run.finding_count,
        kind: client.kind.clone(),
        lens_hashes: run.lens_hashes.clone(),
        lenses: run.lenses.clone(),
        mode: "read-only".to_string(),
        model: gateway.map_or_else(|| run.model.clone(), |gateway| gateway.model.clone()),
        outcome: if run.status == "ok" { "ok" } else { "failed" }.to_string(),
        prompt_hashes: run.prompt_hashes.clone(),
        reason: gateway_reason(run),
        recorded_at: at.to_string(),
        repository: context.repository.clone(),
        retry_of: run.retry_of.clone(),
        role: run.role.clone(),
        round: context.round,
        run_id: context.run_id.clone(),
        spec: context.spec.clone(),
        stage: context.stage.clone(),
        status: run.status.clone(),
        tokens: run.tokens.clone(),
        r#type: "invocation".to_string(),
        vibe_cost_usd: run.vibe_cost_usd,
        workflow: context.flow.clone(),
    }
}

/// The gateway's own reason for a failed run, where critique's failure is one the gateway names.
fn gateway_reason(run: &LedgerRun) -> Option<String> {
    if run.status == "ok" {
        return None;
    }
    let failure = run.failure.as_deref()?;
    match failure {
        "timeout" => Some("timeout".to_string()),
        "unparseable-output" | "empty-output" | "missing-structured-output" => Some("parse-error".to_string()),
        _ if is_exit_code(failure) => Some(failure.to_string()),
        _ => None,
    }
}

/// `/^exit-\d+$/`: `exit-` and one or more ASCII digits, to the end of the string.
fn is_exit_code(failure: &str) -> bool {
    match failure.strip_prefix("exit-") {
        Some(digits) => !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()),
        None => false,
    }
}

/// A JSON number, or `null` where the run reported none.
fn number_or_null(number: Option<f64>) -> Value {
    number.map_or(Value::Null, Value::Num)
}

/// A field the record leaves out when the run did not report it.
fn set_optional(object: &mut Object, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        object.set(key, value);
    }
}

/// A `Record<string, string>` as a JSON object, in the order it was built.
fn string_map(entries: &[(String, String)]) -> Value {
    let mut object = Object::new();
    for (key, value) in entries {
        object.set(key, Value::str(value));
    }
    Value::Obj(object)
}

/// A list of strings as a JSON array.
fn string_array(values: &[String]) -> Value {
    Value::Arr(values.iter().map(|value| Value::str(value)).collect())
}
