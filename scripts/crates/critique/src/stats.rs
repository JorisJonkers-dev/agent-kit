//! The figures `/critique-stats` reports: one cell per critic client on one stage with one prompt, the stages that are
//! candidates for removal, and the routing suggestions. The node runner's `domain/stats.ts`.

use crate::collate::locale_compare;
use crate::error::CritiqueError;
use crate::ledger::{Ledger, LedgerCriticRun, LedgerFinding, LedgerRunContext, SEVERITIES};
use crate::routing::StatsThresholds;
use crate::stage::CRITIQUE_STAGES;
use crate::stage_table::{removal_rule, RemovalRule};
use model_gateway::caps::to_fixed;
use model_gateway::json::{self, number_text, object, Object, Value};
use model_gateway::sha256::sha256_hex;
use model_gateway::text::js_slice;
use std::cmp::Ordering;

/// Which ledger lines the figures cover. Dates are `YYYY-MM-DD`, both ends included, read from `recorded_at` in UTC.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatsFilter {
    /// The allowlist key, `host/owner/repo`; compared ignoring case.
    pub repository: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

/// Accepted over adjudicated for one slice of a cell's findings.
#[derive(Clone, Debug, PartialEq)]
pub struct PrecisionSlice {
    /// Accepted plus rejected findings in the slice.
    pub adjudicated: f64,
    pub accepted: f64,
    /// Null with nothing adjudicated in the slice.
    pub precision: Option<f64>,
}

/// Whether a critic is right over time and whether its severities mean what they say. Only a cell with the routing's
/// minimum of adjudicated findings has one.
#[derive(Clone, Debug, PartialEq)]
pub struct StatsCalibration {
    /// The older half of the cell's adjudicated findings, ordered by when their run was recorded.
    pub earlier: PrecisionSlice,
    /// The newer half; it holds the odd finding.
    pub recent: PrecisionSlice,
    /// Recent precision minus earlier precision; null when either half has none.
    pub drift: Option<f64>,
    /// Precision per severity the finding was recorded with, in the order of `SEVERITIES`.
    pub by_severity: [PrecisionSlice; 3],
}

/// One cell: one critic client on one stage with one prompt. A changed role, lens, preamble, contract or schema file
/// changes the prompt hash, so a revised prompt is compared on its own findings, never mixed with the old.
#[derive(Clone, Debug, PartialEq)]
pub struct StatsCell {
    pub stage: String,
    pub client: String,
    pub model: String,
    pub role: String,
    pub lenses: Vec<String>,
    /// The first twelve hex digits of a hash over every prompt file hash the critic's runs carry.
    pub prompt_hash: String,
    pub first_recorded_at: String,
    pub last_recorded_at: String,
    pub runs: f64,
    /// Runs that gave no usable review: failed or violated read-only.
    pub failed_runs: f64,
    /// Failed runs stopped by a time limit. Vibe's exit 3 is a deadline or a price-limit stop, and both count.
    pub timed_out_runs: f64,
    /// Every finding a critic of this cell raised, alone or with others.
    pub findings: f64,
    pub accepted: f64,
    pub rejected: f64,
    pub hints: f64,
    pub needs_human: f64,
    /// Accepted plus rejected: the findings precision is measured on.
    pub adjudicated: f64,
    /// True when the cell has too few adjudicated findings for a suggestion.
    pub below_threshold: bool,
    /// Accepted findings no other critic client raised.
    pub unique_catches: f64,
    /// Findings fan-out rejected because a quote is in no packet file.
    pub fabricated: f64,
    /// Accepted over adjudicated; hints and needs-human are left out. Null with nothing adjudicated.
    pub precision: Option<f64>,
    /// Fabricated over findings; null with no findings.
    pub fabrication_rate: Option<f64>,
    pub failure_rate: f64,
    pub timeout_rate: f64,
    /// The summed cost of the runs whose cost is known.
    pub cost_usd: f64,
    /// Runs with no known cost (Antigravity is quota, not money; a deadline stop reports none).
    pub unpriced_runs: f64,
    /// Cost over accepted findings; null with no accepted finding or no priced run.
    pub cost_per_accepted_usd: Option<f64>,
    /// Runs a price or time limit stopped before they found anything.
    pub stopped_runs: f64,
    /// The summed known cost of the stopped runs: spend that bought no finding.
    pub wasted_usd: f64,
    /// Null while the cell is below the adjudicated-findings threshold.
    pub calibration: Option<StatsCalibration>,
}

/// A stage whose latest features, or at a review stage whose latest runs, brought no accepted finding that only one
/// critic client raised.
#[derive(Clone, Debug, PartialEq)]
pub enum RemovalCandidate {
    /// The features (spec references) looked at, latest first.
    Features { stage: String, features: Vec<String> },
    /// The review runs looked at, latest first.
    Runs { stage: String, runs: Vec<String> },
}

/// The figures per cell, the stages that are candidates for removal, and the routing suggestions.
#[derive(Clone, Debug, PartialEq)]
pub struct CritiqueStats {
    pub cells: Vec<StatsCell>,
    pub removal_candidates: Vec<RemovalCandidate>,
    /// Plain sentences for the owner. Nothing applies them.
    pub suggestions: Vec<String>,
}

/// A limit stopped the run before it found anything: a time or price limit, or the outer timer that killed it.
const TIMEOUT_FAILURES: [&str; 3] = ["print-timeout", "stopped-early", "exit-124"];

/// A limit stopped the run: a time or price limit, or the outer timer that killed it.
const STOPPED_FAILURES: [&str; 4] = ["print-timeout", "stopped-early", "exit-124", "timeout"];

/// One adjudicated finding of a cell, as calibration reads it.
struct JudgedFinding {
    accepted: bool,
    at: String,
    severity: String,
}

fn number_or_null(value: Option<f64>) -> Value {
    match value {
        Some(number) => Value::Num(number),
        None => Value::Null,
    }
}

fn strings(values: &[String]) -> Value {
    Value::Arr(values.iter().map(|value| Value::str(value)).collect())
}

impl PrecisionSlice {
    pub fn to_json(&self) -> Value {
        object(vec![
            ("accepted", Value::Num(self.accepted)),
            ("adjudicated", Value::Num(self.adjudicated)),
            ("precision", number_or_null(self.precision)),
        ])
    }
}

impl StatsCalibration {
    pub fn to_json(&self) -> Value {
        let mut severities = Object::new();
        for (name, slice) in SEVERITIES.iter().zip(&self.by_severity) {
            severities.set(name, slice.to_json());
        }
        object(vec![
            ("by_severity", Value::Obj(severities)),
            ("drift", number_or_null(self.drift)),
            ("earlier", self.earlier.to_json()),
            ("recent", self.recent.to_json()),
        ])
    }
}

impl StatsCell {
    pub fn to_json(&self) -> Value {
        object(vec![
            ("accepted", Value::Num(self.accepted)),
            ("adjudicated", Value::Num(self.adjudicated)),
            ("below_threshold", Value::Bool(self.below_threshold)),
            ("calibration", match &self.calibration {
                Some(calibration) => calibration.to_json(),
                None => Value::Null,
            }),
            ("client", Value::str(&self.client)),
            ("cost_per_accepted_usd", number_or_null(self.cost_per_accepted_usd)),
            ("cost_usd", Value::Num(self.cost_usd)),
            ("fabricated", Value::Num(self.fabricated)),
            ("fabrication_rate", number_or_null(self.fabrication_rate)),
            ("failed_runs", Value::Num(self.failed_runs)),
            ("failure_rate", Value::Num(self.failure_rate)),
            ("findings", Value::Num(self.findings)),
            ("first_recorded_at", Value::str(&self.first_recorded_at)),
            ("hints", Value::Num(self.hints)),
            ("last_recorded_at", Value::str(&self.last_recorded_at)),
            ("lenses", strings(&self.lenses)),
            ("model", Value::str(&self.model)),
            ("needs_human", Value::Num(self.needs_human)),
            ("precision", number_or_null(self.precision)),
            ("prompt_hash", Value::str(&self.prompt_hash)),
            ("rejected", Value::Num(self.rejected)),
            ("role", Value::str(&self.role)),
            ("runs", Value::Num(self.runs)),
            ("stage", Value::str(&self.stage)),
            ("stopped_runs", Value::Num(self.stopped_runs)),
            ("timed_out_runs", Value::Num(self.timed_out_runs)),
            ("timeout_rate", Value::Num(self.timeout_rate)),
            ("unique_catches", Value::Num(self.unique_catches)),
            ("unpriced_runs", Value::Num(self.unpriced_runs)),
            ("wasted_usd", Value::Num(self.wasted_usd)),
        ])
    }
}

impl RemovalCandidate {
    pub fn to_json(&self) -> Value {
        match self {
            RemovalCandidate::Features { stage, features } => {
                object(vec![("features", strings(features)), ("stage", Value::str(stage))])
            }
            RemovalCandidate::Runs { stage, runs } => object(vec![("runs", strings(runs)), ("stage", Value::str(stage))]),
        }
    }
}

impl CritiqueStats {
    pub fn to_json(&self) -> Value {
        Value::Obj(self.to_object())
    }

    /// The figures as the object the command spreads into its result, in the order the node runner builds it.
    pub fn to_object(&self) -> Object {
        let mut result = Object::new();
        result.set("cells", Value::Arr(self.cells.iter().map(StatsCell::to_json).collect()));
        result.set("removal_candidates", Value::Arr(self.removal_candidates.iter().map(RemovalCandidate::to_json).collect()));
        result.set("suggestions", strings(&self.suggestions));
        result
    }
}

/// A JavaScript `Map`: insertion order, and a key set twice keeps its first place.
struct OrderedMap {
    entries: Vec<(String, usize)>,
}

impl OrderedMap {
    fn new() -> OrderedMap {
        OrderedMap { entries: Vec::new() }
    }

    fn get(&self, key: &str) -> Option<usize> {
        self.entries.iter().find(|(name, _)| name == key).map(|(_, value)| *value)
    }

    fn set(&mut self, key: String, value: usize) {
        match self.entries.iter_mut().find(|(name, _)| *name == key) {
            Some((_, slot)) => *slot = value,
            None => self.entries.push((key, value)),
        }
    }
}

fn slice(findings: &[&JudgedFinding]) -> PrecisionSlice {
    let accepted = findings.iter().filter(|finding| finding.accepted).count() as f64;
    let adjudicated = findings.len() as f64;
    PrecisionSlice { accepted, adjudicated, precision: if findings.is_empty() { None } else { Some(accepted / adjudicated) } }
}

/// The cell's findings in the order their runs were recorded, split in two halves, and by severity.
fn calibration_of(judged: &[JudgedFinding]) -> StatsCalibration {
    let mut ordered: Vec<&JudgedFinding> = judged.iter().collect();
    ordered.sort_by(|a, b| a.at.as_str().cmp(b.at.as_str()));
    let half = ordered.len() / 2;
    let earlier = slice(&ordered[..half]);
    let recent = slice(&ordered[half..]);
    let by_severity = std::array::from_fn(|at| {
        let of: Vec<&JudgedFinding> = ordered.iter().copied().filter(|finding| finding.severity == SEVERITIES[at]).collect();
        slice(&of)
    });
    StatsCalibration {
        drift: match (earlier.precision, recent.precision) {
            (Some(earlier), Some(recent)) => Some(recent - earlier),
            _ => None,
        },
        earlier,
        recent,
        by_severity,
    }
}

/// One cell while the ledger is read.
struct Tally {
    stage: String,
    client: String,
    model: String,
    role: String,
    lenses: Vec<String>,
    prompt_hash: String,
    first: String,
    last: String,
    runs: f64,
    failed: f64,
    timed_out: f64,
    findings: f64,
    accepted: f64,
    rejected: f64,
    hints: f64,
    needs_human: f64,
    unique: f64,
    fabricated: f64,
    cost: f64,
    unpriced: f64,
    stopped: f64,
    wasted: f64,
    /// Every accepted or rejected finding of the cell, for calibration.
    judged: Vec<JudgedFinding>,
}

impl Tally {
    fn new(run: &LedgerCriticRun, prompt_hash: String) -> Tally {
        Tally {
            stage: run.context.stage.clone(),
            client: run.client.clone(),
            model: run.model.clone(),
            role: run.role.clone(),
            lenses: run.lenses.clone(),
            prompt_hash,
            first: String::new(),
            last: String::new(),
            runs: 0.0,
            failed: 0.0,
            timed_out: 0.0,
            findings: 0.0,
            accepted: 0.0,
            rejected: 0.0,
            hints: 0.0,
            needs_human: 0.0,
            unique: 0.0,
            fabricated: 0.0,
            cost: 0.0,
            unpriced: 0.0,
            stopped: 0.0,
            wasted: 0.0,
            judged: Vec::new(),
        }
    }

    fn add_run(&mut self, run: &LedgerCriticRun) {
        self.runs += 1.0;
        if self.first.is_empty() || run.context.recorded_at < self.first {
            self.first = run.context.recorded_at.clone();
        }
        if run.context.recorded_at > self.last {
            self.last = run.context.recorded_at.clone();
        }
        if run.status != "ok" {
            self.failed += 1.0;
        }
        if let Some(failure) = &run.failure {
            if TIMEOUT_FAILURES.contains(&failure.as_str()) {
                self.timed_out += 1.0;
            }
        }
        match run.cost_usd {
            Some(cost) => self.cost += cost,
            None => self.unpriced += 1.0,
        }
        if let Some(failure) = &run.failure {
            if STOPPED_FAILURES.contains(&failure.as_str()) && run.finding_count == 0.0 {
                self.stopped += 1.0;
                self.wasted += run.cost_usd.unwrap_or(0.0);
            }
        }
    }

    fn add_finding(&mut self, finding: &LedgerFinding, unique: bool) {
        self.findings += 1.0;
        if finding.verdict == "accepted" {
            self.accepted += 1.0;
        }
        if finding.verdict == "rejected" {
            self.rejected += 1.0;
        }
        if finding.verdict == "hint" {
            self.hints += 1.0;
        }
        if finding.verdict == "needs-human" {
            self.needs_human += 1.0;
        }
        if finding.verdict == "accepted" && unique {
            self.unique += 1.0;
        }
        if finding.pre_verdict.as_deref() == Some("fabricated-evidence") {
            self.fabricated += 1.0;
        }
        if finding.verdict == "accepted" || finding.verdict == "rejected" {
            self.judged.push(JudgedFinding {
                accepted: finding.verdict == "accepted",
                at: finding.context.recorded_at.clone(),
                severity: finding.severity.clone(),
            });
        }
    }

    fn cell(&self, thresholds: &StatsThresholds) -> StatsCell {
        let adjudicated = self.accepted + self.rejected;
        let priced = self.runs - self.unpriced;
        let below_threshold = adjudicated < thresholds.min_adjudicated_findings;
        StatsCell {
            accepted: self.accepted,
            adjudicated,
            below_threshold,
            calibration: if below_threshold { None } else { Some(calibration_of(&self.judged)) },
            client: self.client.clone(),
            cost_per_accepted_usd: if self.accepted == 0.0 || priced == 0.0 { None } else { Some(self.cost / self.accepted) },
            cost_usd: self.cost,
            fabricated: self.fabricated,
            fabrication_rate: if self.findings == 0.0 { None } else { Some(self.fabricated / self.findings) },
            failed_runs: self.failed,
            failure_rate: self.failed / self.runs,
            findings: self.findings,
            first_recorded_at: self.first.clone(),
            hints: self.hints,
            last_recorded_at: self.last.clone(),
            lenses: self.lenses.clone(),
            model: self.model.clone(),
            needs_human: self.needs_human,
            precision: if adjudicated == 0.0 { None } else { Some(self.accepted / adjudicated) },
            prompt_hash: self.prompt_hash.clone(),
            rejected: self.rejected,
            role: self.role.clone(),
            runs: self.runs,
            stage: self.stage.clone(),
            stopped_runs: self.stopped,
            timed_out_runs: self.timed_out,
            timeout_rate: self.timed_out / self.runs,
            unique_catches: self.unique,
            unpriced_runs: self.unpriced,
            wasted_usd: self.wasted,
        }
    }
}

/// The figures per cell, the stages that are candidates for removal, and the routing suggestions, from the ledger
/// lines the filter keeps. `routed` names the clients a stage's roster routes to now. A finding counts for every cell
/// whose critic raised it.
pub fn critique_stats(
    ledger: &Ledger,
    filter: &StatsFilter,
    thresholds: &StatsThresholds,
    routed: &dyn Fn(&str) -> Vec<String>,
) -> Result<CritiqueStats, CritiqueError> {
    let runs: Vec<&LedgerCriticRun> = ledger.runs.iter().filter(|run| kept(&run.context, filter)).collect();
    let mut tallies: Vec<Tally> = Vec::new();
    let mut by_key = OrderedMap::new();
    let mut by_critic = OrderedMap::new();
    for run in &runs {
        let hash = prompt_hash(run);
        let key = format!("{} {} {}", run.context.stage, run.client, hash);
        let index = match by_key.get(&key) {
            Some(index) => index,
            None => {
                tallies.push(Tally::new(run, hash));
                by_key.set(key, tallies.len() - 1);
                tallies.len() - 1
            }
        };
        tallies[index].add_run(run);
        by_critic.set(critic_key(&run.context.run_id, &run.critic_id), index);
    }
    let mut unique_catches: Vec<&LedgerFinding> = Vec::new();
    for finding in ledger.findings.iter().filter(|finding| kept(&finding.context, filter)) {
        let mut raisers: Vec<usize> = Vec::new();
        for critic in &finding.raised_by {
            let index = raiser(&by_critic, finding, critic)?;
            if !raisers.contains(&index) {
                raisers.push(index);
            }
        }
        let mut clients: Vec<&str> = Vec::new();
        for index in &raisers {
            let client = tallies[*index].client.as_str();
            if !clients.contains(&client) {
                clients.push(client);
            }
        }
        let unique = clients.len() == 1;
        if unique && finding.verdict == "accepted" {
            unique_catches.push(finding);
        }
        for index in &raisers {
            tallies[*index].add_finding(finding, unique);
        }
    }
    let mut cells: Vec<StatsCell> = tallies.iter().map(|tally| tally.cell(thresholds)).collect();
    cells.sort_by(|a, b| {
        stage_index(&a.stage)
            .cmp(&stage_index(&b.stage))
            .then_with(|| locale_compare(&a.client, &b.client, false))
            .then_with(|| locale_compare(&a.first_recorded_at, &b.first_recorded_at, false))
    });
    let removal_candidates = removal_candidates(&runs, &unique_catches, thresholds.removal_features);
    let suggestions = suggestions(&cells, routed);
    Ok(CritiqueStats { cells, removal_candidates, suggestions })
}

/// A stage's place in `CRITIQUE_STAGES`, which is the order the report lists stages in.
fn stage_index(stage: &str) -> i64 {
    CRITIQUE_STAGES.iter().position(|name| *name == stage).map_or(-1, |index| index as i64)
}

fn kept(line: &LedgerRunContext, filter: &StatsFilter) -> bool {
    let day = js_slice(&line.recorded_at, 0.0, Some(10.0));
    let repository = match &filter.repository {
        Some(repository) => line.repository.to_lowercase() == repository.to_lowercase(),
        None => true,
    };
    let since = match &filter.since {
        Some(since) => day.as_str() >= since.as_str(),
        None => true,
    };
    let until = match &filter.until {
        Some(until) => day.as_str() <= until.as_str(),
        None => true,
    };
    repository && since && until
}

fn prompt_hash(run: &LedgerCriticRun) -> String {
    let mut lenses: Vec<&(String, String)> = run.lens_hashes.iter().collect();
    lenses.sort_by(|a, b| locale_compare(&a.0, &b.0, false));
    let pairs: Vec<Value> = lenses.iter().map(|(name, hash)| Value::Arr(vec![Value::str(name), Value::str(hash)])).collect();
    let hashes = &run.prompt_hashes;
    let value = Value::Arr(vec![
        Value::str(&hashes.preamble),
        Value::str(&hashes.role),
        Value::str(&hashes.contract),
        Value::str(&hashes.schema),
        Value::Arr(pairs),
    ]);
    js_slice(&sha256_hex(json::compact(&value).as_bytes()), 0.0, Some(12.0))
}

fn critic_key(run_id: &str, critic_id: &str) -> String {
    format!("{run_id} {critic_id}")
}

fn raiser(by_critic: &OrderedMap, finding: &LedgerFinding, critic_id: &str) -> Result<usize, CritiqueError> {
    match by_critic.get(&critic_key(&finding.context.run_id, critic_id)) {
        Some(index) => Ok(index),
        None => Err(CritiqueError::new(
            "invalid-ledger",
            format!("a finding of run {} names critic {critic_id}, which has no run line", finding.context.run_id),
        )),
    }
}

/// What a removal rule counts a ledger line under: for `features`, the spec it belongs to, and a line recorded before
/// the ledger named the spec counts under none; for `runs`, the run itself.
fn removal_unit(rule: RemovalRule, line: &LedgerRunContext) -> Option<String> {
    match rule {
        RemovalRule::Features => line.spec.clone(),
        RemovalRule::Runs => Some(line.run_id.clone()),
    }
}

/// A stage is a candidate when its latest `features` features (distinct specs, ordered by their latest run of the
/// stage) brought no unique accepted catch; a review stage, when its latest `features` runs did. A stage that has run
/// on fewer is not judged yet.
fn removal_candidates(runs: &[&LedgerCriticRun], unique_catches: &[&LedgerFinding], features: f64) -> Vec<RemovalCandidate> {
    let mut candidates = Vec::new();
    for stage in CRITIQUE_STAGES {
        let rule = removal_rule(stage);
        let mut latest: Vec<(String, String)> = Vec::new();
        for run in runs {
            if run.context.stage != stage {
                continue;
            }
            let Some(unit) = removal_unit(rule, &run.context) else { continue };
            match latest.iter_mut().find(|(name, _)| *name == unit) {
                Some((_, at)) => {
                    if *at < run.context.recorded_at {
                        *at = run.context.recorded_at.clone();
                    }
                }
                None => latest.push((unit, run.context.recorded_at.clone())),
            }
        }
        if (latest.len() as f64) < features {
            continue;
        }
        latest.sort_by(|a, b| locale_compare(&b.1, &a.1, false));
        let looked: Vec<String> = latest.into_iter().take(features as usize).map(|(unit, _)| unit).collect();
        let caught = unique_catches.iter().any(|finding| {
            if finding.context.stage != stage {
                return false;
            }
            match removal_unit(rule, &finding.context) {
                Some(unit) => looked.contains(&unit),
                None => false,
            }
        });
        if !caught {
            candidates.push(match rule {
                RemovalRule::Runs => RemovalCandidate::Runs { stage: stage.to_string(), runs: looked },
                RemovalRule::Features => RemovalCandidate::Features { stage: stage.to_string(), features: looked },
            });
        }
    }
    candidates
}

/// For each routed client of a stage, the client off the roster whose current prompt has the higher precision on that
/// stage, when both current cells have enough adjudicated findings. A client's current cell is its latest prompt hash
/// on the stage, since that is the prompt that runs now.
fn suggestions(cells: &[StatsCell], routed: &dyn Fn(&str) -> Vec<String>) -> Vec<String> {
    let mut sentences = Vec::new();
    for stage in CRITIQUE_STAGES {
        let mut current: Vec<usize> = Vec::new();
        for (index, cell) in cells.iter().enumerate() {
            if cell.stage != stage {
                continue;
            }
            match current.iter().position(|seen| cells[*seen].client == cell.client) {
                Some(at) => {
                    if cells[current[at]].last_recorded_at < cell.last_recorded_at {
                        current[at] = index;
                    }
                }
                None => current.push(index),
            }
        }
        let eligible: Vec<usize> = current.into_iter().filter(|index| !cells[*index].below_threshold).collect();
        let roster = routed(stage);
        for client in &roster {
            let Some(incumbent) = eligible.iter().find(|index| cells[**index].client == *client) else { continue };
            let mut challengers: Vec<usize> = eligible
                .iter()
                .copied()
                .filter(|index| !roster.contains(&cells[*index].client) && precision_of(&cells[*index]) > precision_of(&cells[*incumbent]))
                .collect();
            challengers.sort_by(|a, b| precision_of(&cells[*b]).partial_cmp(&precision_of(&cells[*a])).unwrap_or(Ordering::Equal));
            let Some(challenger) = challengers.first() else { continue };
            let challenger = &cells[*challenger];
            let incumbent = &cells[*incumbent];
            sentences.push(format!(
                "{} has higher precision than {} on the {stage} stage over {} findings ({} against {}); consider swapping.",
                challenger.client,
                client,
                number_text(challenger.adjudicated + incumbent.adjudicated),
                to_fixed(precision_of(challenger), 2),
                to_fixed(precision_of(incumbent), 2),
            ));
        }
    }
    sentences
}

fn precision_of(cell: &StatsCell) -> f64 {
    cell.precision.unwrap_or(0.0)
}

/// What the report says the figures cover.
pub struct StatsScope {
    pub ledger_path: String,
    pub filter: StatsFilter,
    pub thresholds: StatsThresholds,
}

/// The report the skill shows the owner: one table per stage, the removal candidates, then the suggestions.
pub fn render_stats(stats: &CritiqueStats, scope: &StatsScope) -> String {
    let filter = &scope.filter;
    let dates = if filter.since.is_none() && filter.until.is_none() {
        "all".to_string()
    } else {
        format!("{} to {}", filter.since.as_deref().unwrap_or("start"), filter.until.as_deref().unwrap_or("today"))
    };
    let mut lines: Vec<String> = vec![
        "# Critique stats".to_string(),
        String::new(),
        format!(
            "Ledger: {}. Repository: {}. Dates: {dates}.",
            scope.ledger_path,
            filter.repository.as_deref().unwrap_or("all")
        ),
        format!(
            "One row per critic client and prompt. A row with fewer than {} adjudicated findings gets no suggestion.",
            number_text(scope.thresholds.min_adjudicated_findings)
        ),
    ];
    if let Some(wasted) = wasted_line(&stats.cells) {
        lines.push(wasted);
    }
    if stats.cells.is_empty() {
        lines.push(String::new());
        lines.push("No critic runs in the ledger match.".to_string());
    }
    for stage in CRITIQUE_STAGES {
        let rows: Vec<&StatsCell> = stats.cells.iter().filter(|cell| cell.stage == stage).collect();
        if rows.is_empty() {
            continue;
        }
        lines.push(String::new());
        lines.push(format!("## {stage}"));
        lines.push(String::new());
        lines.push("| client | prompt | runs | adjudicated | precision | unique | fabricated | cost per accepted | failed | timed out | wasted |".to_string());
        lines.push("|---|---|---|---|---|---|---|---|---|---|---|".to_string());
        for cell in rows {
            let adjudicated = format!("{}{}", number_text(cell.adjudicated), if cell.below_threshold { " (too few)" } else { "" });
            let precision = match cell.precision {
                Some(precision) => to_fixed(precision, 2),
                None => "-".to_string(),
            };
            let fabricated = match cell.fabrication_rate {
                Some(rate) => percent(rate),
                None => "-".to_string(),
            };
            let cost = match cell.cost_per_accepted_usd {
                Some(cost) => format!("${}", to_fixed(cost, 4)),
                None => "-".to_string(),
            };
            let wasted = if cell.stopped_runs == 0.0 {
                "-".to_string()
            } else {
                format!("${} ({})", to_fixed(cell.wasted_usd, 4), number_text(cell.stopped_runs))
            };
            let row = [
                cell.client.clone(),
                cell.prompt_hash.clone(),
                number_text(cell.runs),
                adjudicated,
                precision,
                number_text(cell.unique_catches),
                fabricated,
                cost,
                percent(cell.failure_rate),
                percent(cell.timeout_rate),
                wasted,
            ]
            .join(" | ");
            lines.push(format!("| {row} |"));
        }
    }
    if !stats.cells.is_empty() {
        lines.extend(calibration_lines(&stats.cells, scope.thresholds.min_adjudicated_findings));
    }
    lines.push(String::new());
    lines.push("## Removal candidates".to_string());
    lines.push(String::new());
    if stats.removal_candidates.is_empty() {
        lines.push("none".to_string());
    }
    for candidate in &stats.removal_candidates {
        let (stage, looked, unit) = match candidate {
            RemovalCandidate::Features { stage, features } => (stage, features, "features"),
            RemovalCandidate::Runs { stage, runs } => (stage, runs, "runs"),
        };
        lines.push(format!(
            "- {stage}: no unique accepted catch across its last {} {unit} ({})",
            number_text(looked.len() as f64),
            looked.join(", ")
        ));
    }
    lines.push(String::new());
    lines.push("## Suggestions".to_string());
    lines.push(String::new());
    if stats.suggestions.is_empty() {
        lines.push("none".to_string());
    }
    for suggestion in &stats.suggestions {
        lines.push(format!("- {suggestion}"));
    }
    format!("{}\n", lines.join("\n"))
}

/// The calibration table: one row per cell with enough adjudicated findings, or the sentence saying none has.
fn calibration_lines(cells: &[StatsCell], minimum: f64) -> Vec<String> {
    let rows: Vec<(&StatsCell, &StatsCalibration)> =
        cells.iter().filter_map(|cell| cell.calibration.as_ref().map(|calibration| (cell, calibration))).collect();
    if rows.is_empty() {
        return vec![
            String::new(),
            "## Calibration".to_string(),
            String::new(),
            format!("No critic has {} adjudicated findings yet.", number_text(minimum)),
        ];
    }
    let mut lines = vec![
        String::new(),
        "## Calibration".to_string(),
        String::new(),
        format!(
            "Precision over time and by severity, for cells with {} or more adjudicated findings. Earlier and recent are the older and newer half of the cell's findings; the count is in brackets.",
            number_text(minimum)
        ),
        String::new(),
        "| stage | client | prompt | adjudicated | earlier | recent | drift | critical | major | minor |".to_string(),
        "|---|---|---|---|---|---|---|---|---|---|".to_string(),
    ];
    for (cell, calibration) in rows {
        let [critical, major, minor] = &calibration.by_severity;
        let drift = match calibration.drift {
            Some(drift) => format!("{}{}", if drift >= 0.0 { "+" } else { "" }, to_fixed(drift, 2)),
            None => "-".to_string(),
        };
        let row = [
            cell.stage.clone(),
            cell.client.clone(),
            cell.prompt_hash.clone(),
            number_text(cell.adjudicated),
            share(&calibration.earlier),
            share(&calibration.recent),
            drift,
            share(critical),
            share(major),
            share(minor),
        ]
        .join(" | ");
        lines.push(format!("| {row} |"));
    }
    lines
}

fn share(slice: &PrecisionSlice) -> String {
    match slice.precision {
        Some(precision) => format!("{} ({})", to_fixed(precision, 2), number_text(slice.adjudicated)),
        None => "-".to_string(),
    }
}

/// What every row's stopped runs cost together; none when no run was stopped.
fn wasted_line(cells: &[StatsCell]) -> Option<String> {
    let runs: f64 = cells.iter().map(|cell| cell.stopped_runs).sum();
    if runs == 0.0 {
        return None;
    }
    let usd: f64 = cells.iter().map(|cell| cell.wasted_usd).sum();
    let runs_text = if runs == 1.0 {
        "1 run a price or time limit stopped before it".to_string()
    } else {
        format!("{} runs a price or time limit stopped before they", number_text(runs))
    };
    Some(format!("Wasted: ${} on {runs_text} found anything.", to_fixed(usd, 4)))
}

fn percent(rate: f64) -> String {
    format!("{}%", number_text((rate * 100.0).round()))
}
