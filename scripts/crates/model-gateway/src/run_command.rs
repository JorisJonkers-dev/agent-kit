//! The `run` command: one task handed to one client model, and what came of it. Everything that can refuse
//! the run happens before anything is launched: the nesting guard, the data policy for the repository, and the
//! read-only proof of the client model. A run whose client was launched leaves a run directory and a ledger
//! line, whether the client succeeded or not; a failure of the gateway's own file writes after the launch
//! propagates, as in the node runner.
//!
//! A writing run works in a worktree of its own, on a branch of its own, entered in the kept list; it watches
//! the repository's other checkouts, stages what its delegate left, and runs the caller's check in the
//! worktree. What that adds to the result is in `write_run`.

use crate::cap_guard::{run_cost_class, CapGuard, CapSource, Reservation, ReserveArgs, MIN_GRANT_TIME_LEFT_S};
use crate::caps::Estimate;
use crate::client_run::{ClientRun, ClientRunError, ClientRunRequest};
use crate::config::{ClientConfig, CostClass, GatewayConfig, ModelEntry};
use crate::config_parse::load_config;
use crate::error::GatewayError;
use crate::escalation_run::{AttemptError, AttemptOptions, AttemptedRun, Delegation, EscalatedRun};
use crate::extras::RunExtras;
use crate::grant_guard::GrantGuard;
use crate::json::{self, Object, Value};
use crate::launchers::{LaunchPaths, LaunchRequest, DELEGATION_MARK};
use crate::ledger_file::LedgerStore;
use crate::locations;
use crate::models::family_of_model_id;
use crate::outputs::ClientOutput;
use crate::policy::{decide_policy, resolve_scope, DataScope, ModelPermission, PolicySubject};
use crate::ports::{ProcessResult, Runtime};
use crate::proof::read_proof_files;
use crate::run::{
    input_json, named_reset, price_run, read_only_proof, read_run_input, require_allowed_provider, run_failure_reason, run_id, summary_of, wrapped_prompt,
    ClientReport, LaunchExit, ReadOnlyProof, RunClient, RunInput, RunMode, RunTokens, FORBIDDEN_PROVIDERS, NO_TOKENS, PROVIDER_CLIENTS,
};
use crate::slots::SlotGuard;
use crate::state_file::Workflows;
use crate::text::join;
use crate::timestamps::to_iso;
use crate::usage::{plan_windows, warnings_of};
use crate::workflows;
use crate::write_run::{writing, FinishedWrite, WatchedWorktree, WriteReport};
use std::collections::BTreeMap;

/// What one delegation came to: a short result, with the full output left in its run directory.
#[derive(Clone, Debug, PartialEq)]
pub struct RunResult {
    pub ok: bool,
    pub run_id: String,
    pub workflow: String,
    pub client: String,
    pub model: String,
    pub family: String,
    pub mode: RunMode,
    pub tokens: RunTokens,
    pub cost_usd: f64,
    /// The file the delegate's full output was written to.
    pub output_path: String,
    /// The delegate's own summary, cut at `summary_max_chars`.
    pub summary: String,
    pub reason: Option<String>,
    /// When the client model's own answer said it is usable again.
    pub unavailable_until: Option<String>,
    /// The worktree a writing run worked in, the branch it is on, its staged files and its check.
    pub write: Option<WriteReport>,
    /// This run's place in its delegation when it is a retry or a climb (2 and up).
    pub attempt: Option<usize>,
    /// The run id of the delegation's first attempt, on every later one.
    pub attempt_of: Option<String>,
    /// Set when the run's throwaway worktree could not be removed.
    pub cleanup_warning: Option<String>,
    /// One line per plan window of this client at or above the warning ratio.
    pub warnings: Vec<String>,
}

impl RunResult {
    /// The result as the node runner prints it: its members in the order it spreads them.
    pub fn to_json(&self) -> Value {
        Value::Obj(self.to_object())
    }

    fn to_object(&self) -> Object {
        let mut members = vec![
            ("client", Value::str(&self.client)),
            ("cost_usd", Value::Num(self.cost_usd)),
            ("family", Value::str(&self.family)),
            ("mode", Value::str(self.mode.as_str())),
            ("model", Value::str(&self.model)),
            ("output_path", Value::str(&self.output_path)),
            ("run_id", Value::str(&self.run_id)),
            ("status", Value::str(if self.ok { "ok" } else { "failed" })),
            ("summary", Value::str(&self.summary)),
            ("tokens", tokens_json(self.tokens)),
            ("workflow", Value::str(&self.workflow)),
        ];
        let optional = [
            ("reason", self.reason.as_deref().map(Value::str)),
            ("unavailable_until", self.unavailable_until.as_deref().map(Value::str)),
        ];
        members.extend(optional.into_iter().filter_map(|(key, value)| value.map(|value| (key, value))));
        // A writing run's report sits where the node runner spreads it: after the reason, before the escalation's own members.
        if let Some(write) = &self.write {
            members.extend(write.members());
        }
        let optional = [
            ("attempt", self.attempt.map(|attempt| Value::Num(attempt as f64))),
            ("attempt_of", self.attempt_of.as_deref().map(Value::str)),
            ("cleanup_warning", self.cleanup_warning.as_deref().map(Value::str)),
            ("warnings", (!self.warnings.is_empty()).then(|| Value::Arr(self.warnings.iter().map(|line| Value::str(line)).collect()))),
        ];
        members.extend(optional.into_iter().filter_map(|(key, value)| value.map(|value| (key, value))));
        let mut object = Object::new();
        for (key, value) in members {
            object.set(key, value);
        }
        object
    }
}

impl AttemptedRun for RunResult {
    fn is_ok(&self) -> bool {
        self.ok
    }
    fn run_id(&self) -> &str {
        &self.run_id
    }
    fn client(&self) -> &str {
        &self.client
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn family(&self) -> Option<&str> {
        Some(&self.family)
    }
    fn summary(&self) -> Option<&str> {
        Some(&self.summary)
    }
    fn set_summary(&mut self, summary: String) {
        self.summary = summary;
    }
    fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
    fn output_path(&self) -> &str {
        &self.output_path
    }
    fn check_output_path(&self) -> Option<&str> {
        self.write.as_ref().and_then(|write| write.check.as_ref()).map(|check| check.output_path.as_str())
    }
}

/// The tokens as the result and the ledger write them: `cached`, `input`, `output`.
fn tokens_json(tokens: RunTokens) -> Value {
    json::object(vec![("cached", Value::Num(tokens.cached)), ("input", Value::Num(tokens.input)), ("output", Value::Num(tokens.output))])
}

/// Where a client model stands for one delegation: the facts a run settles before it launches anything.
struct Placed {
    repository: Option<String>,
    client: ClientConfig,
    config: GatewayConfig,
    entry: Option<ModelEntry>,
    family: String,
    root: Option<String>,
}

/// What a delegation needs before it can run, as a batch plan checks it: where it stands, what it costs and what
/// holds that cost.
pub struct Checked {
    pub repository: Option<String>,
    pub client: ClientConfig,
    pub config: GatewayConfig,
    pub entry: Option<ModelEntry>,
    pub family: String,
    pub root: Option<String>,
    pub cap: CapSource,
    pub cost_class: CostClass,
    pub estimate: Option<Estimate>,
}

/// What a batch writes on the ledger line of each attempt of one of its delegations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchTag {
    pub batch_id: String,
    pub delegation: String,
}

/// What one attempt is launched with: the escalation's own options, and the reservation a batch already took for it.
pub struct RunOptions<'a> {
    /// Absent on the first attempt, which nothing links to.
    pub link: Option<crate::escalation::AttemptLink>,
    /// Called once everything that can refuse the run has passed and before anything is launched.
    pub start_check: Option<crate::escalation_run::StartCheck<'a>>,
    /// A reservation taken up front for this attempt, which it uses and settles instead of reserving its own.
    pub reservation: Option<Reservation>,
    /// The batch plan and delegation this attempt belongs to, written on its ledger line.
    pub tag: Option<BatchTag>,
}

impl RunOptions<'_> {
    /// The options of a run that is not part of a batch: nothing taken up front, and nothing tagged.
    fn plain<'a, 'b: 'a>(options: &'a AttemptOptions<'b>) -> RunOptions<'a> {
        RunOptions { link: options.link.clone(), start_check: options.start_check, reservation: None, tag: None }
    }
}

/// Takes a slot of the client an attempt is about to run on; the attempt gives it back when it ends.
pub type SlotTaker<'a, 'b> = &'a dyn Fn(&str) -> SlotGuard<'b>;

/// What a batch hands one delegation: its pre-taken reservation, its tag, and the slot it must hold on the client
/// each attempt runs on.
#[derive(Default)]
pub struct DelegationOptions<'a, 'b> {
    pub reservation: Option<Reservation>,
    pub tag: Option<BatchTag>,
    pub slot: Option<SlotTaker<'a, 'b>>,
}

/// Where a run works, and how it leaves: in the repository itself when the client model's read-only proof
/// passed, else in a throwaway directory that `release` deletes. A writing run works in a worktree of its own,
/// which it never releases: the kept list is what removes it.
pub(crate) struct Workplace {
    pub(crate) dir: String,
    /// The repository a throwaway worktree was added to; `None` for a plain directory.
    pub(crate) worktree_of: Option<String>,
    pub(crate) throwaway: bool,
    /// The branch a writing run's worktree is on.
    pub(crate) branch: Option<String>,
}

/// Everything one launched run needs beyond its input, settled before anything starts.
pub(crate) struct Delegated<'a> {
    pub(crate) started_at: i64,
    pub(crate) started_ms: f64,
    pub(crate) binary: &'a str,
    pub(crate) config: &'a GatewayConfig,
    pub(crate) entry: Option<&'a ModelEntry>,
    pub(crate) family: &'a str,
    pub(crate) id: &'a str,
    pub(crate) input: &'a RunInput,
    pub(crate) run_dir: &'a str,
    pub(crate) work_dir: &'a str,
    /// The repository root the run works in, when it names a repository.
    pub(crate) repo: Option<&'a str>,
    /// The branch a writing run's worktree is on.
    pub(crate) branch: Option<&'a str>,
    /// The worktrees of the repository a writing run watches, with what they held before the delegate ran.
    pub(crate) watched: Vec<WatchedWorktree>,
    /// When the run's grant or workflow lapses; the check may not outlive it.
    pub(crate) ends_at_ms: Option<i64>,
}

pub struct RunCommand<'a> {
    pub runtime: &'a Runtime,
}

impl RunCommand<'_> {
    /// Reads the input and runs the delegation: a run, retried, climbed and swapped as the escalation decides.
    pub fn run(&self, raw: &Value) -> Result<Value, GatewayError> {
        let input = read_run_input(raw)?;
        let delegation = (EscalatedRun { runtime: self.runtime }).run(&input, &mut |attempt, options| self.run_once(attempt, &RunOptions::plain(options)))?;
        Ok(match delegation {
            Delegation::Run(escalated) => {
                let mut result = escalated.run.to_object();
                for (key, value) in escalated.report.members() {
                    result.set(key, value);
                }
                Value::Obj(result)
            }
            Delegation::Unavailable(unavailable) => unavailable.to_json(),
        })
    }

    /// One delegation whose input is already read, with the reservation a batch took for it. Only its first
    /// attempt holds that reservation: a retry, a climb or a swap reserves its own, as any attempt does. Each
    /// attempt holds a slot of the client it really runs on, which after a fall-through is not the client its
    /// delegation was planned for.
    pub fn delegation(&self, input: &RunInput, given: &DelegationOptions) -> Result<Delegation<RunResult>, GatewayError> {
        let mut held = given.reservation.clone();
        (EscalatedRun { runtime: self.runtime }).run(input, &mut |attempt, options| {
            let taken = held.take();
            let give = given.slot.map(|slot| slot(&attempt.client));
            let result = self.run_once(
                attempt,
                &RunOptions { link: options.link.clone(), start_check: options.start_check, reservation: taken, tag: given.tag.clone() },
            );
            drop(give);
            result
        })
    }

    /// What a delegation needs before it can run, as a batch plan checks it without launching or reserving
    /// anything: its client model, the data policy, its workflow, its estimate, and the grant that covers it. The
    /// first thing wrong is the refusal it returns.
    pub fn check(&self, input: &RunInput) -> Result<Checked, GatewayError> {
        let placed = self.place(input)?;
        let cost_class = run_cost_class(&placed.client, input);
        let estimate = (CapGuard { runtime: self.runtime }).estimate(input, placed.entry.as_ref(), &placed.config)?;
        self.require_workflow_open(&input.workflow)?;
        let cap = self.cap_source(cost_class, input)?;
        Ok(Checked {
            repository: placed.repository,
            client: placed.client,
            config: placed.config,
            entry: placed.entry,
            family: placed.family,
            root: placed.root,
            cap,
            cost_class,
            estimate,
        })
    }

    /// A fresh run id, and the run directory that run keeps its prompt, output and result in.
    pub fn allot(&self, root: Option<&str>, at_ms: i64) -> Result<(String, String), GatewayError> {
        let id = run_id(at_ms, &crate::commands::random_suffix());
        let run_dir = self.run_dir(root, &id)?;
        Ok((run_dir, id))
    }

    /// One attempt: a single run of one client model, which the escalation wraps.
    fn run_once(&self, given: &RunInput, options: &RunOptions) -> Result<RunResult, AttemptError> {
        self.refuse_nested()?;
        let placed = self.place(given)?;
        let Placed { repository, client, config, entry, family, root } = placed;
        let held = options.reservation.as_ref();
        let cost_class = run_cost_class(&client, given);
        // The other checkouts of the repository, watched from before anything is taken: the listing itself
        // refuses the run when it cannot be read, so a refusal leaves no reservation to settle, no workflow
        // joined, and no worktree kept behind it.
        let watched = match writing(given) {
            Some(write) => self.watch_others(root.as_deref().unwrap_or(write.repo))?,
            None => Vec::new(),
        };
        // A metered-employer delegation that starts later than its plan was checked (a batch's dependent) needs
        // its grant to still stand.
        let within = match held {
            Some(_) if cost_class == CostClass::MeteredEmployer => Some(self.within_grant(given)?),
            _ => None,
        };
        let input = within.as_ref().unwrap_or(given);
        if let Some(start_check) = options.start_check {
            if let Some(blocked) = start_check(input)? {
                (CapGuard { runtime: self.runtime }).release(held, self.runtime.clock.now_ms())?;
                return Err(AttemptError::CannotStart(blocked));
            }
        }
        // A writing run never works in a throwaway directory: its worktree is the caller's to take.
        let throwaway = match writing(input) {
            Some(_) => false,
            None => self.needs_throwaway(input, entry.as_ref())?,
        };
        let caps = CapGuard { runtime: self.runtime };
        let estimate = match held {
            Some(reservation) => Some(reservation.estimate.clone()),
            None => caps.estimate(input, entry.as_ref(), &config)?,
        };
        self.require_workflow_open(&input.workflow)?;
        let cap = match held {
            Some(_) => CapSource::MeteredPersonal,
            None => self.cap_source(cost_class, input)?,
        };
        let started_at = self.runtime.clock.now_ms();
        let started_ms = self.runtime.clock.monotonic_ms();
        let (run_dir, id) = match held {
            Some(reservation) => (reservation.record.run_dir.clone(), reservation.record.run_id.clone()),
            None => self.allot(root.as_deref(), started_at)?,
        };
        let reservation = match held {
            Some(reservation) => Some(reservation.clone()),
            None => caps.reserve(&ReserveArgs {
                cap: &cap,
                config: &config,
                cost_class,
                estimate: estimate.as_ref(),
                input,
                now_ms: self.runtime.clock.now_ms(),
                run_dir: &run_dir,
                run_id: &id,
            })?,
        };
        self.join_workflow(&input.workflow, reservation.as_ref())?;
        let workplace = match writing(input) {
            Some(write) => self.write_workplace(&write, root.as_deref().unwrap_or(write.repo), &id, started_at)?,
            None => self.workplace(root.as_deref(), throwaway)?,
        };
        let input = held_to(input, reservation.as_ref());
        let delegated = self.delegate(&Delegated {
            started_at,
            started_ms,
            binary: &client.binary,
            config: &config,
            entry: entry.as_ref(),
            family: &family,
            id: &id,
            input: &input,
            run_dir: &run_dir,
            work_dir: &workplace.dir,
            repo: root.as_deref(),
            branch: workplace.branch.as_deref(),
            watched,
            ends_at_ms: reservation.as_ref().and_then(|reservation| reservation.ends_at_ms),
        });
        let warning = self.release(&workplace);
        let mut result = delegated?;
        if let Some(link) = &options.link {
            result.attempt = Some(link.attempt);
            result.attempt_of = link.attempt_of.clone();
        }
        result.cleanup_warning = warning;
        (LedgerStore { runtime: self.runtime }).append_run(&self.ledger_record(&result, repository.as_deref(), started_ms, cost_class, options.tag.as_ref()))?;
        caps.settle(reservation.as_ref(), result.cost_usd, self.runtime.clock.now_ms())?;
        let ledger = (LedgerStore { runtime: self.runtime }).read()?;
        let windows = plan_windows(
            &input.client,
            client.plan_limits.as_deref().unwrap_or(&[]),
            &crate::caps::run_records(&ledger),
            self.runtime.clock.now_ms(),
            config.settings.plan_warning_ratio,
        );
        result.warnings = warnings_of(&input.client, &windows);
        let text = format!("{}\n", json::pretty(&result.to_json()));
        self.runtime.files.write_text(&join(&[&run_dir, "result.json"]), &text).map_err(GatewayError::from_fs)?;
        Ok(result)
    }

    /// A metered-employer delegation that starts later than its plan was checked (a batch's dependent) needs its
    /// grant to still stand: it is refused `no-grant` when every grant lapsed, and `grant-expiring` when under a
    /// minute is left on it, and a run with more time than that is held to what is left.
    pub(crate) fn within_grant(&self, input: &RunInput) -> Result<RunInput, GatewayError> {
        let now = self.runtime.clock.now_ms();
        let grant = (GrantGuard { runtime: self.runtime }).require(&input.workflow, &input.client, now)?;
        let expires = crate::timestamps::parse(&grant.expires_at).unwrap_or_default();
        let left_s = (expires - now).div_euclid(1000);
        if left_s < MIN_GRANT_TIME_LEFT_S {
            return Err(GatewayError::new(
                "grant-expiring",
                format!(
                    "grant {} for workflow {} has {} seconds left, under the {} a run needs",
                    grant.id,
                    input.workflow,
                    left_s.max(0),
                    MIN_GRANT_TIME_LEFT_S
                ),
            ));
        }
        if left_s as f64 >= input.time_limit_s {
            return Ok(input.clone());
        }
        let mut held = input.clone();
        held.time_limit_s = left_s as f64;
        Ok(held)
    }

    /// A delegate may not delegate on: the mark is in its environment, so this call came from inside one.
    pub(crate) fn refuse_nested(&self) -> Result<(), GatewayError> {
        if self.runtime.env.contains_key(DELEGATION_MARK) {
            return Err(GatewayError::new("nested-delegation", format!("a delegate may not delegate on: {DELEGATION_MARK} is set in this environment")));
        }
        Ok(())
    }

    /// Where a client model stands for one delegation, or the refusal that says why it cannot run: the client
    /// and its provider, the model the catalogue places, the repository it works in and the data policy for it.
    fn place(&self, input: &RunInput) -> Result<Placed, GatewayError> {
        let config = load_config(self.runtime)?;
        let Some(client) = config.clients.get(&input.client).cloned() else {
            return Err(GatewayError::new("unknown-model", format!("the gateway knows no client {}", input.client)));
        };
        if input.client == "opencode" {
            require_allowed_provider(&input.model, client.providers.as_deref())?;
        }
        require_provider(input, &client)?;
        let entry = catalogue_entry(&config, &input.client, &input.model)?;
        let Some(family) = entry.as_ref().map(|entry| entry.family.clone()).or_else(|| family_of_model_id(&input.model).map(str::to_string)) else {
            return Err(GatewayError::new(
                "unknown-model",
                format!(
                    "{}:{} is not a model the gateway can place: add it to the catalogue in defaults.json or the machine override",
                    input.client, input.model
                ),
            ));
        };
        let root = match &input.repo {
            Some(repo) => Some(self.repository_root(repo)?),
            None => None,
        };
        let repository = self.permission(input, &config, &family)?;
        Ok(Placed { repository, client, config, entry, family, root })
    }

    /// The data policy's permission for this client model, or a refusal naming the rule. A run with a repository
    /// reads it, so it carries code whatever the caller marks; an inline run carries only what the caller declares.
    /// The result is the repository the ledger keys the run by.
    fn permission(&self, input: &RunInput, config: &GatewayConfig, family: &str) -> Result<Option<String>, GatewayError> {
        let scope = resolve_scope(self.runtime, input.repo.as_deref())?;
        let subject = PolicySubject { client: input.client.clone(), model: input.model.clone(), family: family.to_string() };
        let decision = decide_policy(config, &scope, std::slice::from_ref(&subject));
        let permission: Option<&ModelPermission> = decision.permissions.first().filter(|permission| permission.permitted);
        let Some(permission) = permission else {
            return Err(GatewayError::new("policy-refused", format!("{}:{} is not permitted: {}", input.client, input.model, decision.reason)));
        };
        let carries = input.carries.map_or((false, false), |carries| (carries.code, carries.history));
        let carries_code = if input.repo.is_none() { carries.0 } else { true };
        if carries_code && !permission.may_send.code {
            return Err(GatewayError::new("policy-refused", format!("{}:{} may not carry code: {}", input.client, input.model, decision.reason)));
        }
        if carries.1 && !permission.may_send.history {
            return Err(GatewayError::new("policy-refused", format!("{}:{} may not carry history: {}", input.client, input.model, decision.reason)));
        }
        Ok(match scope {
            DataScope::Repository { key, .. } => Some(key),
            _ => None,
        })
    }

    /// Whether the run must work in a throwaway directory: a client model goes in place only when its read-only
    /// proof passed on this machine, whatever the client.
    fn needs_throwaway(&self, input: &RunInput, entry: Option<&ModelEntry>) -> Result<bool, GatewayError> {
        let files = read_proof_files(self.runtime)?;
        let names: Vec<&str> = entry.and_then(|entry| entry.names.as_ref()).map_or(Vec::new(), |names| names.iter().map(String::as_str).collect());
        Ok(!matches!(read_only_proof(&files, &input.model, &names, &input.client), ReadOnlyProof::Proven { .. }))
    }

    /// The repository's top level, which is where the delegate works. A path git cannot read, or one in no
    /// repository at all, is an error: a run that names a repository never falls back to an inline one.
    fn repository_root(&self, repo: &str) -> Result<String, GatewayError> {
        let root = self.runtime.git.root(repo).and_then(|root| root.ok_or_else(|| "not a git repository".to_string()));
        root.map_err(|message| GatewayError::new("repository-unreadable", format!("{repo}: {message}")))
    }

    /// Where the run keeps its prompt, its output and its result: in the repository's common directory when it
    /// has one, so a worktree's run is found with the repository's other runs, and in this machine's state
    /// directory otherwise.
    fn run_dir(&self, root: Option<&str>, id: &str) -> Result<String, GatewayError> {
        let Some(root) = root else {
            return Ok(join(&[&locations::runs_dir(&self.runtime.env)?, id]));
        };
        Ok(join(&[&self.common_dir(root)?, "model-gateway", "runs", id]))
    }

    /// The repository's common directory, which every gateway directory of the repository sits in.
    pub(crate) fn common_dir(&self, root: &str) -> Result<String, GatewayError> {
        self.runtime.git.common_dir(root).map_err(|message| GatewayError::new("repository-unreadable", format!("{root}: {message}")))
    }

    /// What the run's estimate is held against: the grant that covers a metered-employer run, which it is
    /// refused without, or the metered-personal caps.
    fn cap_source(&self, cost_class: CostClass, input: &RunInput) -> Result<CapSource, GatewayError> {
        if cost_class != CostClass::MeteredEmployer {
            return Ok(CapSource::MeteredPersonal);
        }
        Ok(CapSource::Grant((GrantGuard { runtime: self.runtime }).require(&input.workflow, &input.client, self.runtime.clock.now_ms())?))
    }

    /// Whether the run may join its workflow: one that ended is refused, and nothing is started.
    fn require_workflow_open(&self, name: &str) -> Result<(), GatewayError> {
        workflows::require_open(&(Workflows { runtime: self.runtime }).read()?, name, self.runtime.clock.now_ms()).map(|_| ())
    }

    /// A run names its workflow, and the gateway keeps that workflow's life: one named for the first time starts
    /// now. It runs once the estimate is reserved, so a run that cannot start its workflow releases the
    /// reservation rather than holding it against the caps.
    fn join_workflow(&self, name: &str, reservation: Option<&Reservation>) -> Result<(), GatewayError> {
        let now = self.runtime.clock.now_ms();
        let joined = (Workflows { runtime: self.runtime }).update(|records| workflows::start(records, name, now));
        if let Err(error) = joined {
            (CapGuard { runtime: self.runtime }).release(reservation, now)?;
            return Err(error);
        }
        Ok(())
    }

    /// The directory the client works in. A run with a passing proof works in the repository itself, or in a
    /// fresh empty directory when it has none (which stays). A throwaway run works in a worktree of the
    /// repository's HEAD, or in a fresh empty directory when it has none, and `release` deletes it.
    fn workplace(&self, root: Option<&str>, throwaway: bool) -> Result<Workplace, GatewayError> {
        let temp = temp_root(&self.runtime.env);
        if !throwaway {
            let dir = match root {
                Some(root) => root.to_string(),
                None => self.runtime.files.temp_dir(&temp, "model-gateway-run-").map_err(GatewayError::from_fs)?,
            };
            return Ok(Workplace { dir, worktree_of: None, throwaway: false, branch: None });
        }
        let dir = self.runtime.files.temp_dir(&temp, "model-gateway-worktree-").map_err(GatewayError::from_fs)?;
        let Some(root) = root else {
            return Ok(Workplace { dir, worktree_of: None, throwaway: true, branch: None });
        };
        if let Err(message) = self.runtime.git.add_worktree(root, &dir) {
            self.runtime.files.remove(&dir).map_err(GatewayError::from_fs)?;
            return Err(GatewayError::new("worktree-failed", format!("{root}: cannot add a throwaway worktree: {message}")));
        }
        Ok(Workplace { dir, worktree_of: Some(root.to_string()), throwaway: true, branch: None })
    }

    /// Deletes the run's throwaway directory. A cleanup that fails never fails the run: it is reported as a
    /// warning the result carries, and a run that failed keeps its own error.
    fn release(&self, workplace: &Workplace) -> Option<String> {
        let released = match (&workplace.worktree_of, workplace.throwaway) {
            (Some(root), _) => self.runtime.git.remove_worktree(root, &workplace.dir),
            (None, true) => self.runtime.files.remove(&workplace.dir).map_err(|error| error.message),
            (None, false) => Ok(()),
        };
        released.err().map(|message| format!("cannot remove the throwaway worktree {}: {message}", workplace.dir))
    }

    /// One launched run, up to the point where its result is known: its prompt on disk, the launch and the
    /// client's full output.
    fn delegate(&self, run: &Delegated) -> Result<RunResult, GatewayError> {
        let input = run.input;
        let files = self.runtime.files.as_ref();
        let codex = self.codex_worker(run);
        let paths = LaunchPaths {
            export_dir: join(&[run.run_dir, "vibe"]),
            prompt_path: join(&[run.run_dir, "prompt.md"]),
            run_dir: Some(run.run_dir.to_string()),
            work_dir: run.work_dir.to_string(),
            git_writable: codex.git_writable,
        };
        let prompt = wrapped_prompt(&input.prompt, run.config.settings.summary_max_chars, input.mode, codex.worker.as_ref());
        files.write_text(&paths.prompt_path, &prompt).map_err(GatewayError::from_fs)?;
        let input_text = format!("{}\n", json::pretty(&input_json(input)));
        files.write_text(&join(&[run.run_dir, "input.json"]), &input_text).map_err(GatewayError::from_fs)?;
        let (result, output, failure) = self.launch(run.binary, input, &paths, &prompt);
        let exit = LaunchExit { exit_code: result.exit_code, timed_out: result.timed_out, stderr: result.stderr.clone() };
        let report = ClientReport { failure: output.failure.clone(), error: output.error.clone() };
        let delegate_reason = failure.or_else(|| run_failure_reason(&exit, output.answer.as_deref(), Some(&report)));
        let finished: Option<FinishedWrite> = match writing(input) {
            Some(write) => Some(self.finish_write(run, &write, delegate_reason.as_deref())),
            None => None,
        };
        let reason = match &finished {
            Some(finished) => finished.reason.clone(),
            None => delegate_reason.clone(),
        };
        let refused = matches!(delegate_reason.as_deref(), Some("rate-limited" | "model-unavailable"));
        let unavailable_until = if refused {
            let elapsed = self.runtime.clock.monotonic_ms() - run.started_ms;
            named_reset(&exit, output.error.as_ref(), run.started_at + elapsed as i64).map(to_iso)
        } else {
            None
        };
        let cost = price_run(output.charged, output.tokens, run.entry.and_then(|entry| entry.prices.as_ref()));
        let output_path = join(&[run.run_dir, "output.json"]);
        files.write_text(&output_path, &result.stdout).map_err(GatewayError::from_fs)?;
        if !result.stderr.is_empty() {
            files.write_text(&join(&[run.run_dir, "stderr.txt"]), &result.stderr).map_err(GatewayError::from_fs)?;
        }
        let summary = output.answer.as_deref().map_or_else(String::new, |answer| summary_of(answer, run.config.settings.summary_max_chars));
        Ok(RunResult {
            ok: reason.is_none(),
            run_id: run.id.to_string(),
            workflow: input.workflow.clone(),
            client: input.client.clone(),
            model: input.model.clone(),
            family: run.family.to_string(),
            mode: input.mode,
            tokens: output.tokens,
            cost_usd: cost,
            output_path,
            summary,
            reason,
            unavailable_until,
            write: finished.map(|finished| finished.report),
            attempt: None,
            attempt_of: None,
            cleanup_warning: None,
            warnings: Vec::new(),
        })
    }

    /// The client, non-interactively, through the gateway's launch path, with the mark that stops a delegate
    /// delegating on. A launch that cannot start at all is a failed run, not an error: the caller still gets a
    /// result and a ledger line. The failure is the reason the run carries.
    pub(crate) fn launch(&self, binary: &str, input: &RunInput, paths: &LaunchPaths, prompt: &str) -> (ProcessResult, ClientOutput, Option<String>) {
        let launched = RunClient::parse(&input.client).ok_or_else(|| ClientRunError::Spawn(format!("{} is not a client a run can launch", input.client))).and_then(|client| {
            (ClientRun { process: self.runtime.process.as_ref(), files: self.runtime.files.as_ref(), env: &self.runtime.env }).launch(&ClientRunRequest {
                launch: LaunchRequest {
                    model: input.model.clone(),
                    time_limit_s: input.time_limit_s,
                    max_price_usd: input.max_price_usd,
                    mode: Some(input.mode),
                    provider: input.provider.clone(),
                },
                binary: binary.to_string(),
                client,
                prompt: prompt.to_string(),
                paths: paths.clone(),
                extras: RunExtras { env: Some(BTreeMap::from([(DELEGATION_MARK.to_string(), "1".to_string())])), ..RunExtras::default() },
            })
        });
        match launched {
            Ok(outcome) => (outcome.result, outcome.output, None),
            Err(error) => {
                let (ClientRunError::Gateway(GatewayError { message, .. }) | ClientRunError::Spawn(message)) = error;
                let output = ClientOutput { answer: None, tokens: NO_TOKENS, charged: None, failure: None, error: None };
                (ProcessResult { stdout: String::new(), stderr: message, exit_code: 1, timed_out: false }, output, Some("launch-failed".to_string()))
            }
        }
    }

    /// The run's one ledger line: what it was, what it cost and how it ended.
    fn ledger_record(&self, result: &RunResult, repository: Option<&str>, started_ms: f64, cost_class: CostClass, tag: Option<&BatchTag>) -> Value {
        let mut members = vec![
            ("at", Value::Str(to_iso(self.runtime.clock.now_ms()))),
            ("client", Value::str(&result.client)),
            ("cost_class", Value::str(cost_class.as_str())),
            ("cost_usd", Value::Num(result.cost_usd)),
            ("duration_s", Value::Num((self.runtime.clock.monotonic_ms() - started_ms + 0.5).floor() / 1000.0)),
            ("family", Value::str(&result.family)),
            ("mode", Value::str(result.mode.as_str())),
            ("model", Value::str(&result.model)),
            ("outcome", Value::str(if result.ok { "ok" } else { "failed" })),
            ("repository", repository.map_or(Value::Null, Value::str)),
            ("run_id", Value::str(&result.run_id)),
            ("tokens", tokens_json(result.tokens)),
            ("workflow", Value::str(&result.workflow)),
        ];
        let optional = [
            ("reason", result.reason.as_deref().map(Value::str)),
            ("unavailable_until", result.unavailable_until.as_deref().map(Value::str)),
            ("attempt", result.attempt.map(|attempt| Value::Num(attempt as f64))),
            ("attempt_of", result.attempt_of.as_deref().map(Value::str)),
        ];
        members.extend(optional.into_iter().filter_map(|(key, value)| value.map(|value| (key, value))));
        // A batch's tag goes last, as the node runner spreads it.
        if let Some(tag) = tag {
            members.push(("batch_id", Value::str(&tag.batch_id)));
            members.push(("delegation", Value::str(&tag.delegation)));
        }
        json::object(members)
    }
}

/// The input a reserved run is launched with: its own, held to the estimate it reserved and to the time its
/// grant leaves. A run that reserved nothing runs as it was sent.
fn held_to(input: &RunInput, reservation: Option<&Reservation>) -> RunInput {
    let Some(reservation) = reservation else {
        return input.clone();
    };
    let mut held = input.clone();
    // A price the run did not send is written after the members it did.
    held.tail.extend(input.max_price_usd.is_none().then_some("max_price_usd"));
    held.max_price_usd = Some(reservation.estimate.usd);
    held.time_limit_s = input.time_limit_s.min(reservation.time_limit_s);
    held
}

/// The directory temporary directories go in: `TMPDIR`, else `TMP`, else `TEMP`, else `/tmp`, as Node's `os.tmpdir()`.
fn temp_root(env: &std::collections::HashMap<String, String>) -> String {
    for name in ["TMPDIR", "TMP", "TEMP"] {
        if let Some(dir) = env.get(name).filter(|dir| !dir.is_empty()) {
            let trimmed = dir.trim_end_matches('/');
            return if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() };
        }
    }
    "/tmp".to_string()
}

/// The providers a machine lets a client run on, as an error names them.
fn allowed(client: &ClientConfig) -> String {
    match client.providers.as_deref() {
        None | Some([]) => "no provider".to_string(),
        Some(providers) => providers.join(", "),
    }
}

/// The provider a run may use. A client that runs on a provider the caller names must name one this machine
/// lists for it, and no configuration can turn on a provider the owner forbade.
fn require_provider(input: &RunInput, client: &ClientConfig) -> Result<(), GatewayError> {
    let named = PROVIDER_CLIENTS.contains(&input.client.as_str());
    let Some(provider) = &input.provider else {
        if !named {
            return Ok(());
        }
        return Err(GatewayError::new("provider-not-allowed", format!("a {} run must name its provider: this machine allows {}", input.client, allowed(client))));
    };
    let forbidden = FORBIDDEN_PROVIDERS.iter().find(|forbidden| *forbidden == provider || input.model.starts_with(&format!("{forbidden}/")));
    if forbidden.is_some() {
        return Err(GatewayError::new("provider-not-allowed", format!("{} is not allowed: the owner forbade OpenRouter", forbidden.unwrap_or(&""))));
    }
    if !client.providers.as_deref().unwrap_or_default().contains(provider) {
        return Err(GatewayError::new(
            "provider-not-allowed",
            format!("{} may not run on {provider}: this machine allows {}", input.client, allowed(client)),
        ));
    }
    Ok(())
}

/// The catalogue entry for a client model, or `None` when the catalogue does not know it; a disabled model is refused.
fn catalogue_entry(config: &GatewayConfig, client: &str, model: &str) -> Result<Option<ModelEntry>, GatewayError> {
    if config.disabled.contains(&format!("{client}:{model}")) {
        return Err(GatewayError::new("unknown-model", format!("{client}:{model} is disabled in this machine's configuration")));
    }
    Ok(config.models.iter().find(|candidate| candidate.client == client && candidate.model == model).cloned())
}
