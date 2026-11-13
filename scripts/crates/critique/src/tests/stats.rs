//! The stats figures and the stats command: every branch of the tally, the calibration, the removal candidates, the
//! suggestions and the report, and the input the command reads.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::git::{Git, LastChange};
use crate::ledger::{parse_ledger, Ledger};
use crate::routing::StatsThresholds;
use crate::stats::{critique_stats, render_stats, CritiqueStats, RemovalCandidate, StatsFilter, StatsScope};
use crate::stats_command::run as stats_run;
use crate::Runtime;
use model_gateway::adapters::fs::SystemFiles;
use model_gateway::json::{self, Value};
use model_gateway::ports::GitRemote;
use std::collections::HashMap;

use super::scenarios::Sandbox;

fn thresholds(min: f64, features: f64) -> StatsThresholds {
    StatsThresholds { min_adjudicated_findings: min, removal_features: features }
}

fn filter(repository: Option<&str>, since: Option<&str>, until: Option<&str>) -> StatsFilter {
    StatsFilter { repository: repository.map(str::to_string), since: since.map(str::to_string), until: until.map(str::to_string) }
}

fn none_routed(_stage: &str) -> Vec<String> {
    Vec::new()
}

fn keys(value: &Value) -> Vec<String> {
    match value.as_object() {
        Some(object) => object.entries().iter().map(|(key, _)| (*key).to_string()).collect(),
        None => Vec::new(),
    }
}

fn at(value: &Value, key: &str) -> Value {
    value.as_object().and_then(|object| object.get(key)).cloned().unwrap_or(Value::Null)
}

/// One run line, with the fields a test does not care about filled in.
#[derive(Clone)]
struct Run {
    run_id: String,
    critic: String,
    client: String,
    stage: String,
    at: String,
    repository: String,
    spec: Option<String>,
    cost: Option<f64>,
    status: String,
    failure: Option<String>,
    findings: f64,
    lens_hashes: Vec<(String, String)>,
    preamble: String,
}

impl Run {
    fn new(run_id: &str, critic: &str, client: &str, stage: &str, at: &str) -> Run {
        Run {
            run_id: run_id.to_string(),
            critic: critic.to_string(),
            client: client.to_string(),
            stage: stage.to_string(),
            at: at.to_string(),
            repository: "github.com/Owner/Repo".to_string(),
            spec: Some("spec-1".to_string()),
            cost: Some(0.5),
            status: "ok".to_string(),
            failure: None,
            findings: 1.0,
            lens_hashes: vec![("lens".to_string(), "h".to_string())],
            preamble: "p".to_string(),
        }
    }

    fn spec(mut self, spec: Option<&str>) -> Run {
        self.spec = spec.map(str::to_string);
        self
    }

    fn cost(mut self, cost: Option<f64>) -> Run {
        self.cost = cost;
        self
    }

    fn status(mut self, status: &str) -> Run {
        self.status = status.to_string();
        self
    }

    fn failure(mut self, failure: Option<&str>) -> Run {
        self.failure = failure.map(str::to_string);
        self
    }

    fn findings(mut self, findings: f64) -> Run {
        self.findings = findings;
        self
    }

    fn repository(mut self, repository: &str) -> Run {
        self.repository = repository.to_string();
        self
    }

    fn lens_hashes(mut self, lens_hashes: &[(&str, &str)]) -> Run {
        self.lens_hashes = lens_hashes.iter().map(|(name, hash)| (name.to_string(), hash.to_string())).collect();
        self
    }

    fn preamble(mut self, preamble: &str) -> Run {
        self.preamble = preamble.to_string();
        self
    }

    fn text(&self) -> String {
        let spec = match &self.spec {
            Some(spec) => format!("\"spec\":{},", json::compact(&Value::str(spec))),
            None => String::new(),
        };
        let cost = match self.cost {
            Some(cost) => json::number_text(cost),
            None => "null".to_string(),
        };
        let failure = match &self.failure {
            Some(failure) => json::compact(&Value::str(failure)),
            None => "null".to_string(),
        };
        let lenses: Vec<String> = self
            .lens_hashes
            .iter()
            .map(|(name, hash)| format!("{}:{}", json::compact(&Value::str(name)), json::compact(&Value::str(hash))))
            .collect();
        format!(
            "{{\"type\":\"run\",\"recorded_at\":\"{}\",\"repository\":\"{}\",{spec}\"stage\":\"{}\",\"run_id\":\"{}\",\"critic_id\":\"{}\",\"client\":\"{}\",\"model\":\"model-{}\",\"role\":\"role-{}\",\"lenses\":[\"lens\"],\"prompt_hashes\":{{\"contract\":\"c\",\"preamble\":\"{}\",\"role\":\"r\",\"schema\":\"s\"}},\"lens_hashes\":{{{}}},\"cost_usd\":{cost},\"status\":\"{}\",\"failure\":{failure},\"finding_count\":{}}}",
            self.at,
            self.repository,
            self.stage,
            self.run_id,
            self.critic,
            self.client,
            self.client,
            self.client,
            self.preamble,
            lenses.join(","),
            self.status,
            json::number_text(self.findings)
        )
    }
}

/// One finding line, with the fields a test does not care about filled in.
#[derive(Clone)]
struct Finding {
    run_id: String,
    finding_id: String,
    stage: String,
    at: String,
    repository: String,
    spec: Option<String>,
    severity: String,
    raised_by: Vec<String>,
    verdict: String,
    pre_verdict: Option<String>,
}

impl Finding {
    fn new(run_id: &str, finding_id: &str, stage: &str, at: &str, raised_by: &[&str], verdict: &str) -> Finding {
        Finding {
            run_id: run_id.to_string(),
            finding_id: finding_id.to_string(),
            stage: stage.to_string(),
            at: at.to_string(),
            repository: "github.com/Owner/Repo".to_string(),
            spec: Some("spec-1".to_string()),
            severity: "major".to_string(),
            raised_by: raised_by.iter().map(|id| (*id).to_string()).collect(),
            verdict: verdict.to_string(),
            pre_verdict: None,
        }
    }

    fn severity(mut self, severity: &str) -> Finding {
        self.severity = severity.to_string();
        self
    }

    fn pre_verdict(mut self, pre_verdict: Option<&str>) -> Finding {
        self.pre_verdict = pre_verdict.map(str::to_string);
        self
    }

    fn spec(mut self, spec: Option<&str>) -> Finding {
        self.spec = spec.map(str::to_string);
        self
    }

    fn repository(mut self, repository: &str) -> Finding {
        self.repository = repository.to_string();
        self
    }

    fn text(&self) -> String {
        let spec = match &self.spec {
            Some(spec) => format!("\"spec\":{},", json::compact(&Value::str(spec))),
            None => String::new(),
        };
        let pre_verdict = match &self.pre_verdict {
            Some(pre_verdict) => json::compact(&Value::str(pre_verdict)),
            None => "null".to_string(),
        };
        let raised: Vec<String> = self.raised_by.iter().map(|id| json::compact(&Value::str(id))).collect();
        format!(
            "{{\"type\":\"finding\",\"recorded_at\":\"{}\",\"repository\":\"{}\",{spec}\"stage\":\"{}\",\"run_id\":\"{}\",\"finding_id\":\"{}\",\"severity\":\"{}\",\"raised_by\":[{}],\"verdict\":\"{}\",\"pre_verdict\":{pre_verdict}}}",
            self.at,
            self.repository,
            self.stage,
            self.run_id,
            self.finding_id,
            self.severity,
            raised.join(","),
            self.verdict
        )
    }
}

fn ledger_of(runs: &[Run], findings: &[Finding]) -> Ledger {
    let mut lines: Vec<String> = runs.iter().map(Run::text).collect();
    lines.extend(findings.iter().map(Finding::text));
    parse_ledger(&lines.join("\n")).expect("a ledger")
}

fn stats_of(runs: &[Run], findings: &[Finding], min: f64, features: f64) -> CritiqueStats {
    critique_stats(&ledger_of(runs, findings), &filter(None, None, None), &thresholds(min, features), &none_routed)
        .expect("the figures")
}

#[test]
fn a_cell_counts_its_runs_findings_and_costs() {
    let runs = [Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z")];
    let findings = [
        Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted"),
        Finding::new("r1", "f2", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "rejected").pre_verdict(Some("fabricated-evidence")),
        Finding::new("r1", "f3", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "hint"),
        Finding::new("r1", "f4", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "needs-human"),
    ];
    let stats = stats_of(&runs, &findings, 1.0, 3.0);
    let cell = &stats.cells[0];
    assert_eq!((cell.client.as_str(), cell.model.as_str(), cell.role.as_str(), cell.lenses.clone()), ("a", "model-a", "role-a", vec!["lens".to_string()]));
    assert_eq!(
        (cell.runs, cell.findings, cell.accepted, cell.rejected, cell.hints, cell.needs_human, cell.adjudicated, cell.unique_catches, cell.fabricated),
        (1.0, 4.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0)
    );
    assert_eq!((cell.precision, cell.fabrication_rate, cell.failure_rate, cell.timeout_rate), (Some(0.5), Some(0.25), 0.0, 0.0));
    assert_eq!(
        (cell.cost_usd, cell.unpriced_runs, cell.cost_per_accepted_usd, cell.stopped_runs, cell.wasted_usd, cell.below_threshold),
        (0.5, 0.0, Some(0.5), 0.0, 0.0, false)
    );
    assert_eq!((cell.first_recorded_at.as_str(), cell.last_recorded_at.as_str()), ("2026-01-01T00:00:00.000Z", "2026-01-01T00:00:00.000Z"));
    assert_eq!(cell.prompt_hash.len(), 12);
}

#[test]
fn a_cell_with_no_findings_has_no_rates() {
    let runs = [Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z").cost(None)];
    let stats = stats_of(&runs, &[], 1.0, 3.0);
    let cell = &stats.cells[0];
    assert_eq!((cell.findings, cell.adjudicated, cell.precision, cell.fabrication_rate), (0.0, 0.0, None, None));
    assert_eq!((cell.unpriced_runs, cell.cost_usd, cell.cost_per_accepted_usd), (1.0, 0.0, None));
    assert!(cell.below_threshold);
    assert_eq!(cell.calibration, None);
}

#[test]
fn calibration_splits_the_findings_in_two_halves_and_by_severity() {
    let runs = [Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z")];
    let findings = [
        Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted").severity("critical"),
        Finding::new("r1", "f2", "spec", "2026-01-02T00:00:00.000Z", &["c1"], "rejected").severity("critical"),
        Finding::new("r1", "f3", "spec", "2026-01-03T00:00:00.000Z", &["c1"], "accepted").severity("major"),
        Finding::new("r1", "f4", "spec", "2026-01-04T00:00:00.000Z", &["c1"], "rejected").severity("minor"),
    ];
    let stats = stats_of(&runs, &findings, 1.0, 3.0);
    let calibration = stats.cells[0].calibration.clone().expect("a calibration");
    assert_eq!((calibration.earlier.accepted, calibration.earlier.adjudicated, calibration.earlier.precision), (1.0, 2.0, Some(0.5)));
    assert_eq!((calibration.recent.accepted, calibration.recent.adjudicated, calibration.recent.precision), (1.0, 2.0, Some(0.5)));
    assert_eq!(calibration.drift, Some(0.0));
    let [critical, major, minor] = &calibration.by_severity;
    assert_eq!((critical.accepted, critical.adjudicated, critical.precision), (1.0, 2.0, Some(0.5)));
    assert_eq!((major.accepted, major.adjudicated, major.precision), (1.0, 1.0, Some(1.0)));
    assert_eq!((minor.accepted, minor.adjudicated, minor.precision), (0.0, 1.0, Some(0.0)));
}

#[test]
fn calibration_of_one_finding_has_no_drift_and_a_falling_critic_drifts_down() {
    let runs = [Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z")];
    let one = [Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted")];
    let calibration = stats_of(&runs, &one, 1.0, 3.0).cells[0].calibration.clone().expect("a calibration");
    assert_eq!((calibration.earlier.precision, calibration.recent.precision, calibration.drift), (None, Some(1.0), None));
    let falling = [
        Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted"),
        Finding::new("r1", "f2", "spec", "2026-01-02T00:00:00.000Z", &["c1"], "accepted"),
        Finding::new("r1", "f3", "spec", "2026-01-03T00:00:00.000Z", &["c1"], "rejected"),
        Finding::new("r1", "f4", "spec", "2026-01-04T00:00:00.000Z", &["c1"], "rejected"),
    ];
    let calibration = stats_of(&runs, &falling, 1.0, 3.0).cells[0].calibration.clone().expect("a calibration");
    assert_eq!(calibration.drift, Some(-1.0));
}

#[test]
fn a_finding_naming_a_critic_with_no_run_line_is_an_invalid_ledger() {
    let runs = [Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z")];
    let findings = [Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c9"], "accepted")];
    let error = critique_stats(&ledger_of(&runs, &findings), &filter(None, None, None), &thresholds(1.0, 3.0), &none_routed).err();
    assert_eq!(
        error,
        Some(crate::error::CritiqueError::new("invalid-ledger", "a finding of run r1 names critic c9, which has no run line"))
    );
}

#[test]
fn a_finding_counts_for_every_critic_that_raised_it_and_is_unique_only_alone() {
    let runs = [
        Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z"),
        Run::new("r1", "c2", "b", "spec", "2026-01-01T00:00:00.000Z"),
    ];
    let findings = [
        Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1", "c2"], "accepted"),
        Finding::new("r1", "f2", "spec", "2026-01-01T00:00:00.000Z", &["c1", "c1"], "accepted"),
    ];
    let stats = stats_of(&runs, &findings, 1.0, 3.0);
    assert_eq!((stats.cells[0].findings, stats.cells[0].unique_catches), (2.0, 1.0));
    assert_eq!((stats.cells[1].findings, stats.cells[1].unique_catches), (1.0, 0.0));
}

#[test]
fn the_filter_keeps_lines_by_repository_and_dates() {
    let runs = [
        Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z"),
        Run::new("r2", "c2", "a", "spec", "2026-02-01T00:00:00.000Z").repository("github.com/other/repo"),
        Run::new("r3", "c3", "a", "spec", "2026-03-01T00:00:00.000Z"),
    ];
    let ledger = ledger_of(&runs, &[]);
    let kept = |filter: &StatsFilter| critique_stats(&ledger, filter, &thresholds(1.0, 3.0), &none_routed).expect("the figures").cells[0].runs;
    assert_eq!(kept(&filter(None, None, None)), 3.0);
    assert_eq!(kept(&filter(Some("GITHUB.COM/OWNER/REPO"), None, None)), 2.0);
    assert_eq!(kept(&filter(None, Some("2026-02-01"), None)), 2.0);
    assert_eq!(kept(&filter(None, None, Some("2026-02-01"))), 2.0);
    assert_eq!(kept(&filter(None, Some("2026-02-01"), Some("2026-02-01"))), 1.0);
}

#[test]
fn removal_candidates_judge_a_review_stage_by_its_latest_runs() {
    let runs = [
        Run::new("r1", "c1", "a", "adr-review", "2026-01-01T00:00:00.000Z"),
        Run::new("r2", "c2", "a", "adr-review", "2026-01-02T00:00:00.000Z"),
        Run::new("r3", "c3", "a", "adr-review", "2026-01-03T00:00:00.000Z"),
    ];
    let stats = stats_of(&runs, &[], 1.0, 3.0);
    assert_eq!(
        stats.removal_candidates,
        vec![RemovalCandidate::Runs { stage: "adr-review".to_string(), runs: vec!["r3".to_string(), "r2".to_string(), "r1".to_string()] }]
    );
    let caught = [Finding::new("r3", "f1", "adr-review", "2026-01-03T00:00:00.000Z", &["c3"], "accepted")];
    assert!(stats_of(&runs, &caught, 1.0, 3.0).removal_candidates.is_empty());
}

#[test]
fn suggestions_name_a_challenger_with_higher_precision() {
    let runs = [
        Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z"),
        Run::new("r2", "c2", "b", "spec", "2026-01-01T00:00:00.000Z"),
    ];
    let findings = [
        Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted"),
        Finding::new("r1", "f2", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "rejected"),
        Finding::new("r2", "f3", "spec", "2026-01-01T00:00:00.000Z", &["c2"], "accepted"),
        Finding::new("r2", "f4", "spec", "2026-01-01T00:00:00.000Z", &["c2"], "accepted"),
    ];
    let ledger = ledger_of(&runs, &findings);
    let routed = |stage: &str| if stage == "spec" { vec!["a".to_string()] } else { Vec::new() };
    let stats = critique_stats(&ledger, &filter(None, None, None), &thresholds(1.0, 3.0), &routed).expect("the figures");
    assert_eq!(
        stats.suggestions,
        vec!["b has higher precision than a on the spec stage over 4 findings (1.00 against 0.50); consider swapping.".to_string()]
    );
    let both = |stage: &str| if stage == "spec" { vec!["a".to_string(), "b".to_string()] } else { Vec::new() };
    assert!(critique_stats(&ledger, &filter(None, None, None), &thresholds(1.0, 3.0), &both).expect("the figures").suggestions.is_empty());
    let absent = |stage: &str| if stage == "spec" { vec!["c".to_string()] } else { Vec::new() };
    assert!(critique_stats(&ledger, &filter(None, None, None), &thresholds(1.0, 3.0), &absent).expect("the figures").suggestions.is_empty());
    let high = |stage: &str| if stage == "spec" { vec!["a".to_string()] } else { Vec::new() };
    assert!(critique_stats(&ledger, &filter(None, None, None), &thresholds(3.0, 3.0), &high).expect("the figures").suggestions.is_empty());
}

#[test]
fn the_report_lists_every_stage_the_calibration_the_candidates_and_the_suggestions() {
    let runs = [
        Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z").spec(Some("s1")),
        Run::new("r2", "c2", "a", "spec", "2026-01-02T00:00:00.000Z").spec(Some("s2")),
        Run::new("r3", "c3", "a", "spec", "2026-01-03T00:00:00.000Z").spec(Some("s3")),
        Run::new("r4", "c4", "a", "adr-review", "2026-01-01T00:00:00.000Z"),
        Run::new("r5", "c5", "a", "adr-review", "2026-01-02T00:00:00.000Z"),
        Run::new("r6", "c6", "a", "adr-review", "2026-01-03T00:00:00.000Z"),
    ];
    let findings = [
        Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted"),
        Finding::new("r1", "f2", "spec", "2026-01-02T00:00:00.000Z", &["c1"], "rejected"),
    ];
    let stats = stats_of(&runs, &findings, 1.0, 3.0);
    let scope = StatsScope { ledger_path: "/l.jsonl".to_string(), filter: filter(None, None, None), thresholds: thresholds(1.0, 3.0) };
    let report = render_stats(&stats, &scope);
    assert!(report.starts_with("# Critique stats\n\nLedger: /l.jsonl. Repository: all. Dates: all.\n"), "{report}");
    assert!(report.contains("One row per critic client and prompt. A row with fewer than 1 adjudicated findings gets no suggestion."));
    assert!(report.contains("\n## spec\n\n| client | prompt | runs | adjudicated | precision | unique | fabricated | cost per accepted | failed | timed out | wasted |\n|---|---|---|---|---|---|---|---|---|---|---|\n| a | "));
    assert!(report.contains("\n## adr-review\n"));
    assert!(report.contains("\n## Calibration\n\nPrecision over time and by severity, for cells with 1 or more adjudicated findings."));
    assert!(report.contains("\n## Removal candidates\n\n- spec: no unique accepted catch across its last 3 features (s3, s2, s1)\n- adr-review: no unique accepted catch across its last 3 runs (r6, r5, r4)\n"));
    assert!(report.contains("\n## Suggestions\n\nnone\n"));
    assert!(report.ends_with("\n"));
}

#[test]
fn the_report_says_none_when_no_run_matches() {
    let stats = CritiqueStats { cells: Vec::new(), removal_candidates: Vec::new(), suggestions: Vec::new() };
    let scope = StatsScope {
        ledger_path: "/l.jsonl".to_string(),
        filter: filter(Some("github.com/owner/repo"), Some("2026-01-01"), None),
        thresholds: thresholds(20.0, 3.0),
    };
    let report = render_stats(&stats, &scope);
    assert!(report.contains("Ledger: /l.jsonl. Repository: github.com/owner/repo. Dates: 2026-01-01 to today."), "{report}");
    assert!(report.contains("\n\nNo critic runs in the ledger match.\n"));
    assert!(!report.contains("## Calibration"));
    assert!(report.contains("\n## Removal candidates\n\nnone\n"));
    assert!(report.contains("\n## Suggestions\n\nnone\n"));
    let scope = StatsScope { ledger_path: "/l.jsonl".to_string(), filter: filter(None, None, Some("2026-02-01")), thresholds: thresholds(20.0, 3.0) };
    assert!(render_stats(&stats, &scope).contains("Dates: start to 2026-02-01."));
}

#[test]
fn the_report_shows_a_rising_drift_with_a_plus() {
    let runs = [Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z")];
    let findings = [
        Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "rejected"),
        Finding::new("r1", "f2", "spec", "2026-01-02T00:00:00.000Z", &["c1"], "rejected"),
        Finding::new("r1", "f3", "spec", "2026-01-03T00:00:00.000Z", &["c1"], "accepted"),
        Finding::new("r1", "f4", "spec", "2026-01-04T00:00:00.000Z", &["c1"], "accepted"),
    ];
    let stats = stats_of(&runs, &findings, 1.0, 3.0);
    let scope = StatsScope { ledger_path: "/l.jsonl".to_string(), filter: filter(None, None, None), thresholds: thresholds(1.0, 3.0) };
    assert!(render_stats(&stats, &scope).contains("| 0.00 (2) | 1.00 (2) | +1.00 |"));
}

#[test]
fn the_report_shows_a_dash_for_a_slice_with_nothing_adjudicated() {
    let runs = [Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z")];
    let findings = [Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted").severity("critical")];
    let stats = stats_of(&runs, &findings, 1.0, 3.0);
    let scope = StatsScope { ledger_path: "/l.jsonl".to_string(), filter: filter(None, None, None), thresholds: thresholds(1.0, 3.0) };
    let report = render_stats(&stats, &scope);
    assert!(report.contains("| - | 1.00 (1) | - | 1.00 (1) | - | - |"), "{report}");
}

#[test]
fn the_report_shows_a_dash_for_a_cell_with_no_precision_or_cost() {
    let runs = [Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z").cost(None)];
    let stats = stats_of(&runs, &[], 1.0, 3.0);
    let scope = StatsScope { ledger_path: "/l.jsonl".to_string(), filter: filter(None, None, None), thresholds: thresholds(1.0, 3.0) };
    let report = render_stats(&stats, &scope);
    assert!(report.contains("| a | "), "{report}");
    assert!(report.contains("| 1 | 0 (too few) | - | 0 | - | - | 0% | 0% | - |"), "{report}");
}

#[test]
fn the_figures_print_in_the_key_order_the_node_runner_builds() {
    let runs = [Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z")];
    let findings = [Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted")];
    let stats = stats_of(&runs, &findings, 1.0, 3.0);
    assert_eq!(
        keys(&stats.to_json()),
        vec!["cells", "removal_candidates", "suggestions"]
    );
    assert_eq!(
        keys(&stats.cells[0].to_json()),
        vec![
            "accepted",
            "adjudicated",
            "below_threshold",
            "calibration",
            "client",
            "cost_per_accepted_usd",
            "cost_usd",
            "fabricated",
            "fabrication_rate",
            "failed_runs",
            "failure_rate",
            "findings",
            "first_recorded_at",
            "hints",
            "last_recorded_at",
            "lenses",
            "model",
            "needs_human",
            "precision",
            "prompt_hash",
            "rejected",
            "role",
            "runs",
            "stage",
            "stopped_runs",
            "timed_out_runs",
            "timeout_rate",
            "unique_catches",
            "unpriced_runs",
            "wasted_usd",
        ]
    );
    let calibration = at(&stats.cells[0].to_json(), "calibration");
    assert_eq!(keys(&calibration), vec!["by_severity", "drift", "earlier", "recent"]);
    assert_eq!(keys(&at(&calibration, "by_severity")), vec!["critical", "major", "minor"]);
    assert_eq!(keys(&at(&calibration, "earlier")), vec!["accepted", "adjudicated", "precision"]);
    assert_eq!(at(&at(&calibration, "earlier"), "precision"), Value::Null);
    assert_eq!(at(&stats.cells[0].to_json(), "calibration"), calibration);
    let candidates = [
        RemovalCandidate::Features { stage: "spec".to_string(), features: vec!["s1".to_string()] },
        RemovalCandidate::Runs { stage: "adr-review".to_string(), runs: vec!["r1".to_string()] },
    ];
    assert_eq!(keys(&candidates[0].to_json()), vec!["features", "stage"]);
    assert_eq!(keys(&candidates[1].to_json()), vec!["runs", "stage"]);
    assert_eq!(json::compact(&candidates[0].to_json()), r#"{"features":["s1"],"stage":"spec"}"#);
}

#[test]
fn optional_matching_reads_a_value_the_accept_recognises() {
    let value = json::parse(r#"{"since":"2026-01-01","until":"nope","empty":"","number":7}"#).expect("json");
    let fields = Fields::of(&value, "").expect("fields");
    assert_eq!(fields.optional_matching("since", |text| text.starts_with("2026"), "a date"), Ok(Some("2026-01-01".to_string())));
    assert_eq!(fields.optional_matching("missing", |_| true, "a date"), Ok(None));
    assert_eq!(
        fields.optional_matching("until", |text| text.starts_with("2026"), "a date"),
        Err(ShapeError("until must be a date".to_string()))
    );
    assert_eq!(fields.optional_matching("empty", |_| true, "a date"), Err(ShapeError("empty must be a non-empty string".to_string())));
    assert_eq!(fields.optional_matching("number", |_| true, "a date"), Err(ShapeError("number must be a non-empty string".to_string())));
}

/// A git that answers with what a test scripts.
struct FakeGit {
    remotes: Result<Vec<GitRemote>, String>,
    common: Result<String, String>,
    root: Result<String, String>,
}

impl FakeGit {
    fn none() -> FakeGit {
        FakeGit { remotes: Ok(Vec::new()), common: Ok(String::new()), root: Ok(String::new()) }
    }

    fn origin() -> FakeGit {
        FakeGit {
            remotes: Ok(vec![GitRemote { name: "origin".to_string(), url: "git@github.com:Owner/Repo.git".to_string() }]),
            common: Ok("/r/.git".to_string()),
            root: Ok("/r".to_string()),
        }
    }
}

impl Git for FakeGit {
    fn remotes(&self, _cwd: &str) -> Result<Vec<GitRemote>, String> {
        self.remotes.clone()
    }

    fn common_dir(&self, _cwd: &str) -> Result<String, String> {
        self.common.clone()
    }

    fn root(&self, _cwd: &str) -> Result<String, String> {
        self.root.clone()
    }

    fn resolve_commit(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn last_change(&self, _cwd: &str, _path: &str) -> Result<Option<LastChange>, String> {
        Err("unused".to_string())
    }

    fn file_commits(&self, _cwd: &str, _revision: &str, _path: &str) -> Result<Vec<crate::git::LastChange>, String> {
        Err("unused".to_string())
    }

    fn file_at(&self, _cwd: &str, _commit: &str, _path: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn merge_base(&self, _cwd: &str, _a: &str, _b: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn diff_commits(&self, _cwd: &str, _base: &str, _head: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn changed_paths(&self, _cwd: &str, _base: &str, _head: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn staged_entries(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn tree_of(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn write_index_tree(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn commit_tree(&self, _cwd: &str, _tree: &str, _parent: &str, _message: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn tree_files(&self, _cwd: &str, _commit: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn untracked_paths(&self, _cwd: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn commits_between(&self, _cwd: &str, _base: &str, _head: &str) -> Result<Vec<crate::git::CommitLink>, String> {
        Err("unused".to_string())
    }

    fn tree_entries(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn merge_tree(&self, _cwd: &str, _ours: &str, _theirs: &str) -> Result<crate::git::MergeTree, String> {
        Err("unused".to_string())
    }

    fn diff_against_head(&self, _cwd: &str, _staged: bool) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn paths_against_head(&self, _cwd: &str, _staged: bool) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn current_branch(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
}

fn write(sandbox: &Sandbox, path: &str, text: &str) {
    let target = sandbox.path.join(path);
    std::fs::create_dir_all(target.parent().unwrap_or(&sandbox.path)).expect("a directory");
    std::fs::write(target, text).expect("a file");
}

fn runtime(sandbox: &Sandbox, git: FakeGit) -> Runtime {
    let root = sandbox.text();
    let env = HashMap::from([
        ("HOME".to_string(), format!("{root}/home")),
        ("XDG_STATE_HOME".to_string(), format!("{root}/state")),
    ]);
    let mut runtime = super::cli::runtime("");
    runtime.env = env.clone();
    runtime.files = Box::new(SystemFiles { cwd: root.clone() });
    runtime.gateway.env = env;
    runtime.gateway.files = Box::new(SystemFiles { cwd: root });
    runtime.git = Box::new(git);
    runtime
}

const LEDGER: &str = "home/.local/share/agent-kit/critique/ledger.jsonl";

fn input(text: &str) -> Value {
    json::parse(text).expect("input")
}

#[test]
fn the_command_reports_the_figures() {
    let sandbox = Sandbox::new();
    let run = Run::new("r1", "c1", "vibe-mistral-large", "spec", "2026-01-01T00:00:00.000Z");
    let finding = Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted");
    write(&sandbox, LEDGER, &format!("{}\n{}\n", run.text(), finding.text()));
    let runtime = runtime(&sandbox, FakeGit::none());
    let result = stats_run(&input(r#"{"since":"2026-01-01"}"#), &runtime).expect("a result");
    assert_eq!(
        keys(&result),
        vec!["cells", "removal_candidates", "suggestions", "filter", "ledger_path", "report", "status", "thresholds"]
    );
    assert_eq!(json::compact(&at(&result, "filter")), r#"{"since":"2026-01-01"}"#);
    assert_eq!(at(&result, "status"), Value::str("reported"));
    assert_eq!(at(&result, "ledger_path"), Value::str(&format!("{}/home/.local/share/agent-kit/critique/ledger.jsonl", sandbox.text())));
    assert_eq!(json::compact(&at(&result, "thresholds")), r#"{"min_adjudicated_findings":20,"removal_features":3}"#);
    let report = at(&result, "report");
    let report = report.as_str().unwrap_or_default();
    assert!(report.starts_with("# Critique stats\n\nLedger: "), "{report}");
    assert!(report.contains("Repository: all. Dates: 2026-01-01 to today."), "{report}");
    assert!(report.contains("\n## spec\n"), "{report}");
    assert!(report.contains("| vibe-mistral-large | "), "{report}");
}

#[test]
fn the_command_refuses_a_repository_that_may_not_run() {
    let sandbox = Sandbox::new();
    let runtime = runtime(&sandbox, FakeGit::none());
    let result = stats_run(&input(r#"{"repo":"/r"}"#), &runtime).expect("a refusal");
    assert_eq!(json::compact(&result), r#"{"message":"the repository has no remote, so it counts as not opted in","reason":"no-remote","status":"refused"}"#);
}

#[test]
fn the_command_reads_its_input() {
    let sandbox = Sandbox::new();
    let runtime = runtime(&sandbox, FakeGit::none());
    let error = |text: &str| stats_run(&input(text), &runtime).err();
    assert_eq!(
        error(r#"{"repo":"/r","repository":"github.com/owner/repo"}"#),
        Some(crate::error::CritiqueError::new("invalid-input", "stats input: name the repository by repo or by repository, not both"))
    );
    assert_eq!(
        error(r#"{"since":"yesterday"}"#),
        Some(crate::error::CritiqueError::new("invalid-input", "stats input: since must be a date, YYYY-MM-DD"))
    );
    assert_eq!(
        error(r#"{"since":"2026-02-01","until":"2026-01-01"}"#),
        Some(crate::error::CritiqueError::new("invalid-input", "stats input: since 2026-02-01 is after until 2026-01-01"))
    );
    assert_eq!(
        error(r#"["stats"]"#),
        Some(crate::error::CritiqueError::new("invalid-input", "stats input: the value must be a JSON object"))
    );
    assert_eq!(
        error(r#"{"repository":""}"#),
        Some(crate::error::CritiqueError::new("invalid-input", "stats input: repository must be a non-empty string"))
    );
}

#[test]
fn a_day_must_be_a_real_calendar_day() {
    let sandbox = Sandbox::new();
    let runtime = runtime(&sandbox, FakeGit::none());
    let accepted = |day: &str| stats_run(&input(&format!(r#"{{"since":"{day}"}}"#)), &runtime).is_ok();
    for day in ["2026-01-01", "2024-02-29", "2000-02-29", "0000-01-01", "9999-12-31", "2026-04-30"] {
        assert!(accepted(day), "{day}");
    }
    for day in ["2026-02-30", "1900-02-29", "2026-13-01", "2026-00-10", "2026-01-00", "2026-04-31", "2026-1-01", "2026-01-01 ", "2026/01/01", "2026-01-0a", "20260101"] {
        assert!(!accepted(day), "{day}");
    }
}

#[test]
fn the_command_needs_stats_thresholds() {
    let sandbox = Sandbox::new();
    write(&sandbox, "home/.config/agent-kit/critique-routing.json", r#"{"stats":null}"#);
    let runtime = runtime(&sandbox, FakeGit::none());
    assert_eq!(
        stats_run(&input("{}"), &runtime).err(),
        Some(crate::error::CritiqueError::new("invalid-routing", "routing has no stats thresholds"))
    );
}

#[test]
fn the_command_reports_a_ledger_it_cannot_read() {
    let sandbox = Sandbox::new();
    write(&sandbox, LEDGER, "not json\n");
    let runtime = runtime(&sandbox, FakeGit::none());
    let error = stats_run(&input("{}"), &runtime).err().expect("an error");
    assert_eq!(error.code, "invalid-ledger");
    assert!(error.message.ends_with("ledger.jsonl: line 1 is not JSON"), "{}", error.message);
}

#[test]
fn the_command_reports_a_finding_with_no_run_line() {
    let sandbox = Sandbox::new();
    let finding = Finding::new("r1", "f1", "spec", "2026-01-01T00:00:00.000Z", &["c1"], "accepted");
    write(&sandbox, LEDGER, &format!("{}\n", finding.text()));
    let runtime = runtime(&sandbox, FakeGit::none());
    assert_eq!(
        stats_run(&input("{}"), &runtime).err(),
        Some(crate::error::CritiqueError::new("invalid-ledger", "a finding of run r1 names critic c1, which has no run line"))
    );
}

#[test]
fn the_command_reports_a_repository_it_cannot_read() {
    let sandbox = Sandbox::new();
    let runtime = runtime(&sandbox, FakeGit { remotes: Err("no git here".to_string()), common: Ok(String::new()), root: Ok(String::new()) });
    assert_eq!(stats_run(&input(r#"{"repo":"/r"}"#), &runtime).err(), Some(crate::error::CritiqueError::unexpected("no git here")));
}

#[test]
fn a_resolved_repository_is_the_one_the_figures_are_limited_to() {
    let sandbox = Sandbox::new();
    let run = Run::new("r1", "c1", "a", "spec", "2026-01-01T00:00:00.000Z");
    let other = Run::new("r2", "c2", "a", "spec", "2026-01-01T00:00:00.000Z").repository("github.com/other/repo");
    write(&sandbox, LEDGER, &format!("{}\n{}\n", run.text(), other.text()));
    let runtime = runtime(&sandbox, FakeGit::origin());
    let result = stats_run(&input(r#"{"repo":"/r"}"#), &runtime).expect("a result");
    let cells = at(&result, "cells");
    let cells = match cells {
        Value::Arr(cells) => cells,
        _ => Vec::new(),
    };
    assert_eq!(cells.len(), 1);
    assert_eq!(at(&cells[0], "runs"), Value::Num(1.0));
    assert_eq!(json::compact(&at(&result, "filter")), r#"{"repository":"github.com/owner/repo"}"#);
}

/// A stage the routing has no roster for routes to no client (node: `routing.stages[stage]?.base ?? []`). The ledger
/// only holds known stages, so the command never asks; the shipped routing answers for one it does hold.
#[test]
fn a_stage_routing_lacks_routes_to_no_client() {
    let shipped = json::parse(model_gateway::SHIPPED_ROUTING).expect("shipped routing");
    let routing = crate::routing::parse_routing(&shipped).expect("routing");
    assert!(crate::stats_command::routed_clients(&routing, "no-such-stage").is_empty());
    assert!(!crate::stats_command::routed_clients(&routing, "spec").is_empty());
}
