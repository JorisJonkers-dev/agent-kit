//! The review runs `/flow` lists: each a flow of its own, with its stage, verdict counts and spend. The node runner's
//! `domain/review-runs.ts`.

use crate::budget::LoggedInvocation;
use crate::collate::locale_compare;
use crate::ledger::{LedgerCriticRun, LedgerFinding, VERDICTS};
use crate::stage_table::is_review_stage;
use model_gateway::json::{self, Object, Value};

/// A run as its directory or the ledger shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewRunSource {
    pub run_id: String,
    pub stage: String,
    pub round: f64,
    pub recorded: bool,
}

/// One review run as `/flow` lists it: a flow of its own, so its spend is its own.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewRun {
    pub stage: String,
    pub run_id: String,
    pub round: f64,
    pub recorded: bool,
    /// The recorded verdicts of its findings, in the order of `VERDICTS`; all zero until the run is recorded.
    pub verdicts: Vec<(&'static str, f64)>,
    /// What its Vibe critics were charged.
    pub spent_usd: f64,
}

impl ReviewRun {
    pub fn to_json(&self) -> Value {
        let mut verdicts = Object::new();
        for (verdict, count) in &self.verdicts {
            verdicts.set(verdict, Value::Num(*count));
        }
        json::object(vec![
            ("recorded", Value::Bool(self.recorded)),
            ("round", Value::Num(self.round)),
            ("run_id", Value::str(&self.run_id)),
            ("spent_usd", Value::Num(self.spent_usd)),
            ("stage", Value::str(&self.stage)),
            ("verdicts", Value::Obj(verdicts)),
        ])
    }
}

/// Everything the list is computed from, already narrowed to one repository.
pub struct ReviewRunSources<'a> {
    /// The runs the run directories hold.
    pub directories: Vec<ReviewRunSource>,
    /// The ledger's run lines, which still hold runs whose directory is gone.
    pub ledger_runs: &'a [LedgerCriticRun],
    pub findings: Vec<&'a LedgerFinding>,
    pub invocations: Vec<&'a LoggedInvocation>,
}

/// The review runs, newest first, at most `limit` of them. A run of a critique stage belongs to a feature's flow and
/// is left out. Nothing here says a review is due: a review runs only when someone asks for one.
pub fn review_runs(sources: &ReviewRunSources<'_>, limit: f64) -> Vec<ReviewRun> {
    // A Map: a run keeps the place it first took, and the directory's own stage and round win over the ledger's.
    let mut runs: Vec<ReviewRunSource> = Vec::new();
    for run in &sources.directories {
        match runs.iter_mut().find(|known| known.run_id == run.run_id) {
            Some(known) => *known = run.clone(),
            None => runs.push(run.clone()),
        }
    }
    for line in sources.ledger_runs {
        let context = &line.context;
        match runs.iter_mut().find(|known| known.run_id == context.run_id) {
            Some(known) => known.recorded = true,
            None => runs.push(ReviewRunSource { run_id: context.run_id.clone(), stage: context.stage.clone(), round: context.round, recorded: true }),
        }
    }
    let mut reviews: Vec<ReviewRunSource> = runs.into_iter().filter(|run| is_review_stage(&run.stage)).collect();
    reviews.sort_by(|a, b| locale_compare(&b.run_id, &a.run_id, true));
    reviews.truncate(limit.min(reviews.len() as f64) as usize);
    reviews.into_iter().map(|run| review_run(sources, run)).collect()
}

fn review_run(sources: &ReviewRunSources<'_>, run: ReviewRunSource) -> ReviewRun {
    let spent_usd = sources
        .invocations
        .iter()
        .filter(|line| line.kind == "vibe" && line.run_id.as_deref() == Some(run.run_id.as_str()))
        .fold(0.0, |total, line| total + line.charged_usd);
    let findings: Vec<&&LedgerFinding> = sources.findings.iter().filter(|finding| finding.context.run_id == run.run_id).collect();
    let verdicts = VERDICTS.iter().map(|verdict| (*verdict, findings.iter().filter(|finding| finding.verdict == *verdict).count() as f64)).collect();
    ReviewRun { recorded: run.recorded, round: run.round, run_id: run.run_id, spent_usd, stage: run.stage, verdicts }
}
