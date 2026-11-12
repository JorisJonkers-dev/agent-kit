//! Tests for the caps a metered run is held to: the estimate, the cap checks, the
//! reservation and its settlement, the grant a run is held to and the plan a batch
//! reserves at once. Ported from the node runner's tests of the same modules.

use super::machine::{grant, now_ms, reservation_line, Machine, DEFAULTS};
use crate::cap_guard::*;
use crate::caps::{
    cap_refusal_message, check_caps, estimate_run, live_reservations, metered_charges, prompt_tokens, reservation_record, settled_record,
    settled_record_json, CapName, CapRoom, Estimate, ReservationArgs, RESERVATION_GRACE_S,
};
use crate::config::{CostClass, GatewayConfig, MeteredPersonalCaps, ModelPrices, Tier};
use crate::config_parse::{load_config, parse_defaults};
use crate::error::GatewayError;
use crate::grants::grant_refusal;
use crate::json::{compact, parse};
use crate::ledger::{LedgerEntry, ReservationRecord};
use crate::run::{read_run_input, RunInput};
use crate::timestamps;
use crate::workflows::{workflow_ends_at, WorkflowRecord};

fn input(extra: &str) -> RunInput {
    let text = format!(r#"{{"client":"hermes","mode":"read-only","model":"hermes-hard","prompt":"hi","workflow":"w"{extra}}}"#);
    read_run_input(&parse(&text).expect("JSON")).expect("a run input")
}

fn config(machine: &Machine) -> GatewayConfig {
    load_config(&machine.runtime).expect("the config")
}

fn code<T: std::fmt::Debug>(result: Result<T, GatewayError>) -> (String, String) {
    let error = result.expect_err("the call must refuse");
    (error.code, error.message)
}

fn estimate(usd: f64) -> Estimate {
    Estimate { prompt_tokens: 1.0, output_tokens: 1.0, usd }
}

fn hold(machine: &Machine, cap: &CapSource, run: &RunInput, usd: f64) -> Result<Option<Reservation>, GatewayError> {
    let config = config(machine);
    let held = estimate(usd);
    let class = if run.client == "codex" { CostClass::MeteredEmployer } else { CostClass::MeteredPersonal };
    (CapGuard { runtime: &machine.runtime }).reserve(&ReserveArgs {
        cap,
        config: &config,
        cost_class: class,
        estimate: Some(&held),
        input: run,
        now_ms: now_ms(),
        run_dir: "/run",
        run_id: "run-1",
    })
}

fn settings(machine: &Machine) -> crate::config::Settings {
    config(machine).settings
}

#[test]
fn a_prompts_tokens_count_ascii_at_three_and_a_half_characters_rounded_up() {
    assert_eq!(prompt_tokens(&"x".repeat(3500)), 1000.0);
    assert_eq!(prompt_tokens(&"x".repeat(3501)), 1001.0);
    assert_eq!(prompt_tokens(""), 0.0);
}

#[test]
fn every_character_that_is_not_ascii_counts_as_a_token_of_its_own() {
    assert!(prompt_tokens(&"漢".repeat(1000)) >= 1000.0);
    assert_eq!(prompt_tokens(&format!("{}{}", "x".repeat(35), "漢".repeat(10))), 20.0);
    assert_eq!(prompt_tokens("😀"), 1.0);
}

#[test]
fn a_run_is_estimated_at_its_prompts_tokens_plus_its_tiers_output_allowance() {
    let machine = Machine::new(DEFAULTS);
    let settings = settings(&machine);
    let prices = ModelPrices { input: 3.0, cached_input: 0.3, output: 15.0 };
    let estimated = estimate_run("hi", Some(Tier::Hard), Some(&prices), &settings).expect("an estimate");
    assert_eq!((estimated.prompt_tokens, estimated.output_tokens), (1.0, 6000.0));
    assert!((estimated.usd - 0.090003).abs() < 1e-12, "{}", estimated.usd);
    assert_eq!(estimate_run("hi", None, Some(&prices), &settings), None);
    assert_eq!(estimate_run("hi", Some(Tier::Hard), None, &settings), None);
}

#[test]
fn an_estimate_rounds_up_but_not_for_float_noise() {
    let machine = Machine::new(DEFAULTS);
    let mut settings = settings(&machine);
    settings.output_allowance_tokens = [0.0; 4];
    // 1000 tokens at $1.4 per million is 0.0014 exactly, though 1.4 * 1000 lands a hair above 1400.
    let prices = ModelPrices { input: 1.4, cached_input: 0.0, output: 1.0 };
    assert_eq!(estimate_run(&"x".repeat(3500), Some(Tier::Trivial), Some(&prices), &settings).map(|estimated| estimated.usd), Some(0.0014));
    // A real fraction of a micro-dollar rounds up.
    let dear = ModelPrices { input: 1.5, cached_input: 0.0, output: 1.0 };
    assert_eq!(estimate_run(&"x".repeat(35), Some(Tier::Trivial), Some(&dear), &settings).map(|estimated| estimated.usd), Some(0.000015));
}

fn reserved_line(run_id: &str, workflow: &str, estimate_usd: f64, reserved_at: &str) -> LedgerEntry {
    LedgerEntry::Reservation(ReservationRecord {
        run_id: run_id.to_string(),
        workflow: workflow.to_string(),
        client: "hermes".to_string(),
        cost_class: CostClass::MeteredPersonal,
        model: "hermes-hard".to_string(),
        estimate_usd,
        reserved_at: reserved_at.to_string(),
        expires_at: "2026-11-04T11:00:00.000Z".to_string(),
        run_dir: "/run".to_string(),
    })
}

fn caps() -> MeteredPersonalCaps {
    MeteredPersonalCaps { per_workflow_usd: 2.0, per_day_usd: 5.0 }
}

#[test]
fn a_run_that_fits_both_caps_is_not_refused() {
    let entries = vec![reserved_line("r1", "w", 1.0, "2026-11-04T09:00:00.000Z")];
    assert_eq!(check_caps(&entries, "w", 1.0, &caps(), now_ms()), None);
}

#[test]
fn the_workflow_cap_is_checked_first_and_names_what_is_held() {
    let entries = vec![reserved_line("r1", "w", 1.5, "2026-11-04T09:00:00.000Z"), reserved_line("r2", "other", 1.5, "2026-11-04T09:00:00.000Z")];
    let refused = check_caps(&entries, "w", 1.0, &caps(), now_ms()).expect("refused");
    assert_eq!(refused, CapRoom { cap: CapName::PerWorkflowUsd, limit_usd: 2.0, spent_usd: 0.0, reserved_usd: 1.5 });
    assert_eq!(
        cap_refusal_message(&refused, 1.0, "w", now_ms()),
        "does not fit the per-workflow cap of $2.00: $0.00 spent and $1.50 reserved in workflow w, and its estimate is $1.00"
    );
}

#[test]
fn the_day_cap_counts_the_reservations_of_the_day_across_workflows() {
    let entries: Vec<LedgerEntry> = ["a", "b", "c"].iter().map(|workflow| reserved_line(workflow, workflow, 1.9, "2026-11-04T09:00:00.000Z")).collect();
    let refused = check_caps(&entries, "w", 1.5, &caps(), now_ms()).expect("refused");
    assert_eq!(refused.cap, CapName::PerDayUsd);
    assert_eq!((refused.spent_usd, refused.reserved_usd), (0.0, 5.7));
    assert_eq!(
        cap_refusal_message(&refused, 1.5, "w", now_ms()),
        "does not fit the per-day cap of $5.00: $0.00 spent and $5.70 reserved on 2026-11-04, and its estimate is $1.50"
    );
    // A reservation of another day holds nothing against today's cap.
    let yesterday: Vec<LedgerEntry> = ["a", "b", "c"].iter().map(|workflow| reserved_line(workflow, workflow, 1.9, "2026-11-03T09:00:00.000Z")).collect();
    assert_eq!(check_caps(&yesterday, "w", 1.5, &caps(), now_ms()), None);
}

#[test]
fn a_reservation_lapses_a_grace_period_after_its_time_limit() {
    let args = ReservationArgs {
        run_id: "r1",
        workflow: "w",
        client: "codex",
        cost_class: CostClass::MeteredEmployer,
        model: "gpt-5.5",
        estimate_usd: 1.0,
        reserved_at_ms: now_ms(),
        time_limit_s: 900.0,
        run_dir: "/nowhere",
    };
    let record = reservation_record(&args);
    assert_eq!(record.reserved_at, "2026-11-04T10:00:00.000Z");
    assert_eq!(record.expires_at, timestamps::to_iso(now_ms() + (900 + RESERVATION_GRACE_S as i64) * 1000));
    // A fractional time limit keeps whole milliseconds.
    let fractional = reservation_record(&ReservationArgs { time_limit_s: 0.0005, ..args });
    assert_eq!(fractional.expires_at, "2026-11-04T10:01:00.000Z");
}

#[test]
fn a_settled_reservation_counts_the_cost_of_a_run_that_left_a_result_and_nothing_for_one_that_never_ran() {
    let record = reservation_record(&ReservationArgs {
        run_id: "r1",
        workflow: "w",
        client: "codex",
        cost_class: CostClass::MeteredEmployer,
        model: "gpt-5.5",
        estimate_usd: 1.0,
        reserved_at_ms: now_ms(),
        time_limit_s: 900.0,
        run_dir: "/nowhere",
    });
    let settle = |cost: f64, reason: &str| {
        LedgerEntry::Settled(crate::ledger::SettledRecord { reason: Some(reason.to_string()), ..settled_record(&record, cost, now_ms()) })
    };
    let entry = LedgerEntry::Reservation(record.clone());
    let ran = vec![entry.clone(), settle(7.0, "expired")];
    let usd = |entries: &[LedgerEntry]| metered_charges(entries, CostClass::MeteredEmployer).iter().map(|charge| charge.usd).collect::<Vec<f64>>();
    assert_eq!(usd(&ran), [7.0]);
    assert!(usd(&[entry.clone(), settle(0.0, "not-run")]).is_empty());
    assert!(usd(&[entry, settle(0.0, "not-started")]).is_empty());
    assert!(live_reservations(&ran).is_empty());
}

#[test]
fn a_released_settlement_is_written_with_its_reason_last_as_the_node_runner_spreads_it() {
    let record = reservation_record(&ReservationArgs {
        run_id: "r1",
        workflow: "w",
        client: "codex",
        cost_class: CostClass::MeteredEmployer,
        model: "gpt-5.5",
        estimate_usd: 1.0,
        reserved_at_ms: now_ms(),
        time_limit_s: 900.0,
        run_dir: "/nowhere",
    });
    let released = crate::ledger::SettledRecord { reason: Some("not-started".to_string()), ..settled_record(&record, 0.0, now_ms()) };
    assert_eq!(
        compact(&settled_record_json(&released)),
        r#"{"client":"codex","cost_class":"metered-employer","cost_usd":0,"kind":"settled","reserved_at":"2026-11-04T10:00:00.000Z","run_id":"r1","settled_at":"2026-11-04T10:00:00.000Z","workflow":"w","reason":"not-started"}"#
    );
}

#[test]
fn a_workflow_stops_taking_runs_when_someone_ends_it_or_its_24_hours_run_out_whichever_comes_first() {
    let record = |ended_at: Option<&str>| WorkflowRecord {
        name: "w".to_string(),
        started_at: "2026-11-04T00:00:00.000Z".to_string(),
        ended_at: ended_at.map(str::to_string),
        ended_last: false,
    };
    let at = |text: &str| timestamps::parse(text).expect("a timestamp");
    assert_eq!(workflow_ends_at(&record(None)), at("2026-11-05T00:00:00.000Z"));
    assert_eq!(workflow_ends_at(&record(Some("2026-11-04T12:00:00.000Z"))), at("2026-11-04T12:00:00.000Z"));
    assert_eq!(workflow_ends_at(&record(Some("2026-11-06T12:00:00.000Z"))), at("2026-11-05T00:00:00.000Z"));
}

#[test]
fn a_run_that_does_not_fit_its_grant_is_told_what_is_spent_and_reserved() {
    let held = grant(5.0, 7200.0);
    let entries = vec![LedgerEntry::Reservation(ReservationRecord {
        run_id: "r1".to_string(),
        workflow: "w".to_string(),
        client: "codex".to_string(),
        cost_class: CostClass::MeteredEmployer,
        model: "gpt-5.5".to_string(),
        estimate_usd: 4.5,
        reserved_at: "2026-11-04T09:30:00.000Z".to_string(),
        expires_at: "2026-11-04T11:00:00.000Z".to_string(),
        run_dir: "/run".to_string(),
    })];
    assert_eq!(grant_refusal(&entries, &held, 0.5, "codex", "gpt-5.5"), None);
    assert_eq!(
        grant_refusal(&entries, &held, 1.0, "codex", "gpt-5.5").as_deref(),
        Some("codex:gpt-5.5 does not fit grant grant-1 of $5.00 for workflow w: $0.00 spent and $4.50 reserved, and its estimate is $1.00")
    );
}

#[test]
fn a_client_the_configuration_does_not_declare_is_refused_rather_than_held_to_nothing() {
    let machine = Machine::new(DEFAULTS);
    let config = config(&machine);
    let stranger = input(r#","client":"agy","model":"m""#);
    let (refusal, message) = code(held_cost_class(&stranger, &config));
    assert_eq!(refusal, "unknown-client");
    assert!(message.contains("agy"), "{message}");
    // The guard refuses it before it estimates the run.
    assert_eq!(code((CapGuard { runtime: &machine.runtime }).estimate(&stranger, None, &config)).0, "unknown-client");
}

#[test]
fn only_a_metered_run_is_estimated_and_a_run_that_cannot_be_priced_is_refused() {
    let machine = Machine::new(DEFAULTS);
    let config = config(&machine);
    let guard = CapGuard { runtime: &machine.runtime };
    let entry = |model: &str| config.models.iter().find(|entry| entry.model == model);
    // A subsidised run is held to nothing.
    assert_eq!(guard.estimate(&input(r#","client":"claude","model":"claude-sonnet-5-5""#), entry("claude-sonnet-5-5"), &config), Ok(None));
    // A provider the machine marks subsidised is subsidised too; a paid one is held.
    let kimi = input(r#","client":"opencode","model":"opencode-go/kimi-k2""#);
    assert_eq!(held_cost_class(&kimi, &config), Ok(None));
    assert_eq!(held_cost_class(&input(r#","client":"opencode","model":"other/x""#), &config), Ok(Some(CostClass::MeteredPersonal)));
    // A priced model is estimated; one the catalogue prices nowhere is refused.
    let estimated = guard.estimate(&input(""), entry("hermes-hard"), &config).expect("no refusal").expect("an estimate");
    assert_eq!((estimated.prompt_tokens, estimated.output_tokens), (1.0, 6000.0));
    let (refusal, message) = code(guard.estimate(&input(""), None, &config));
    assert_eq!(refusal, "no-estimate");
    assert_eq!(
        message,
        "hermes:hermes-hard cannot be estimated: the catalogue gives it no price or no tier, so a metered run on it cannot be held to the caps"
    );
    // A provider the caller names is the run's cost class.
    let named = input(r#","provider":"anthropic""#);
    assert_eq!(held_cost_class(&named, &config), Ok(Some(CostClass::MeteredPersonal)));
    assert_eq!(run_cost_class(&config.clients["hermes"], &named), CostClass::MeteredPersonal);
}

#[test]
fn a_metered_personal_reservation_is_written_with_the_settlements_that_were_due() {
    let machine = Machine::new(DEFAULTS);
    let lapsed = reservation_line("old", "/nowhere", 0.25, "2026-11-04T09:45:00.000Z", CostClass::MeteredPersonal);
    machine.write_ledger(&[lapsed]);
    let held = hold(&machine, &CapSource::MeteredPersonal, &input(""), 0.5).expect("held").expect("a reservation");
    assert_eq!((held.time_limit_s, held.ends_at_ms), (900.0, None));
    assert_eq!(held.record.run_id, "run-1");
    assert_eq!(held.record.expires_at, timestamps::to_iso(now_ms() + 960_000));
    let text = machine.ledger_text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(lines[1].starts_with(r#"{"client":"codex","cost_class":"metered-personal","cost_usd":0.25,"kind":"settled","reason":"expired""#), "{}", lines[1]);
    assert!(lines[2].contains(r#""kind":"reservation""#) && lines[2].contains(r#""run_id":"run-1""#), "{}", lines[2]);
}

#[test]
fn a_run_without_an_estimate_holds_nothing() {
    let machine = Machine::new(DEFAULTS);
    let config = config(&machine);
    let run = input("");
    let held = (CapGuard { runtime: &machine.runtime }).reserve(&ReserveArgs {
        cap: &CapSource::MeteredPersonal,
        config: &config,
        cost_class: CostClass::MeteredPersonal,
        estimate: None,
        input: &run,
        now_ms: now_ms(),
        run_dir: "/run",
        run_id: "run-1",
    });
    assert_eq!(held, Ok(None));
    assert_eq!(machine.ledger_text(), "");
}

#[test]
fn a_run_that_does_not_fit_the_caps_is_refused_naming_the_cap_and_holds_nothing() {
    let machine = Machine::new(DEFAULTS);
    let (refusal, message) = code(hold(&machine, &CapSource::MeteredPersonal, &input(""), 3.0));
    assert_eq!(refusal, "cap-exceeded");
    assert_eq!(
        message,
        "hermes:hermes-hard does not fit the per-workflow cap of $2.00: $0.00 spent and $0.00 reserved in workflow w, and its estimate is $3.00"
    );
    assert_eq!(machine.ledger_text(), "");
}

#[test]
fn a_refused_run_still_settles_the_reservations_that_were_due() {
    let machine = Machine::new(DEFAULTS);
    machine.write_ledger(&[reservation_line("old", "/nowhere", 0.25, "2026-11-04T09:45:00.000Z", CostClass::MeteredPersonal)]);
    assert_eq!(code(hold(&machine, &CapSource::MeteredPersonal, &input(""), 3.0)).0, "cap-exceeded");
    assert_eq!(machine.ledger_text().lines().count(), 2);
}

#[test]
fn a_run_under_a_grant_that_does_not_fit_it_is_refused() {
    let machine = Machine::new(DEFAULTS);
    let run = input(r#","client":"codex","model":"gpt-5.5""#);
    let (refusal, message) = code(hold(&machine, &CapSource::Grant(grant(5.0, 7200.0)), &run, 6.0));
    assert_eq!(refusal, "cap-exceeded");
    assert_eq!(message, "codex:gpt-5.5 does not fit grant grant-1 of $5.00 for workflow w: $0.00 spent and $0.00 reserved, and its estimate is $6.00");
}

fn workflow_ending_in(machine: &Machine, ends_in_s: i64) {
    let started = timestamps::to_iso(now_ms() + ends_in_s * 1000 - 24 * 3600 * 1000);
    machine.write_workflows(&[("w", &started)]);
}

fn under_grant(machine: &Machine, held: crate::grants::Grant, time_limit_s: &str) -> Result<Option<Reservation>, GatewayError> {
    let run = input(&format!(r#","client":"codex","model":"gpt-5.5","time_limit_s":{time_limit_s}"#));
    hold(machine, &CapSource::Grant(held), &run, 0.1)
}

#[test]
fn a_run_under_a_grant_keeps_its_own_time_limit_when_the_grant_and_the_workflow_outlast_it() {
    let machine = Machine::new(DEFAULTS);
    workflow_ending_in(&machine, 7200);
    let held = under_grant(&machine, grant(5.0, 7200.0), "900").expect("held").expect("a reservation");
    assert_eq!(held.time_limit_s, 900.0);
    assert_eq!(held.record.expires_at, timestamps::to_iso(now_ms() + (900 + 60) * 1000));
    assert_eq!(held.ends_at_ms, Some(now_ms() + 7_200_000));
}

#[test]
fn a_run_under_a_grant_is_clamped_to_the_time_left_on_the_grant_and_reserves_for_the_clamped_limit() {
    let machine = Machine::new(DEFAULTS);
    let held = under_grant(&machine, grant(5.0, 300.0), "900").expect("held").expect("a reservation");
    assert_eq!(held.time_limit_s, 300.0);
    assert_eq!(held.record.expires_at, timestamps::to_iso(now_ms() + (300 + 60) * 1000));
}

#[test]
fn a_run_under_a_grant_is_clamped_to_the_time_left_in_the_workflow_when_that_ends_first() {
    let machine = Machine::new(DEFAULTS);
    workflow_ending_in(&machine, 120);
    assert_eq!(under_grant(&machine, grant(5.0, 7200.0), "900").expect("held").expect("a reservation").time_limit_s, 120.0);
}

#[test]
fn the_time_left_is_rounded_down_to_whole_seconds_and_a_minute_is_still_enough() {
    let machine = Machine::new(DEFAULTS);
    assert_eq!(under_grant(&machine, grant(5.0, 100.9), "900").expect("held").expect("a reservation").time_limit_s, 100.0);
    assert_eq!(under_grant(&machine, grant(5.0, 60.0), "900").expect("held").expect("a reservation").time_limit_s, 60.0);
}

#[test]
fn a_run_launched_with_under_a_minute_left_on_the_grant_is_refused_naming_the_time_left_and_holds_nothing() {
    let machine = Machine::new(DEFAULTS);
    let (refusal, message) = code(under_grant(&machine, grant(5.0, 59.0), "900"));
    assert_eq!(refusal, "grant-expiring");
    assert_eq!(
        message,
        "the grant covering workflow w has 59 seconds left: wait for it to lapse, then ask the owner for a new grant, or start a new workflow"
    );
    assert_eq!(machine.ledger_text(), "");
    // A grant that already lapsed has none left.
    assert!(code(under_grant(&machine, grant(5.0, -30.0), "900")).1.contains("has 0 seconds left"));
}

#[test]
fn a_run_launched_with_under_a_minute_left_in_the_workflow_is_refused_naming_the_time_left() {
    let machine = Machine::new(DEFAULTS);
    workflow_ending_in(&machine, 30);
    let (refusal, message) = code(under_grant(&machine, grant(5.0, 7200.0), "900"));
    assert_eq!(refusal, "grant-expiring");
    assert!(message.contains("30 seconds"), "{message}");
}

#[test]
fn a_reservation_is_settled_with_what_the_run_cost_and_released_at_no_cost_when_it_never_started() {
    let machine = Machine::new(DEFAULTS);
    let guard = CapGuard { runtime: &machine.runtime };
    // Nothing is held, so nothing is written.
    assert_eq!(guard.settle(None, 1.0, now_ms()), Ok(()));
    assert_eq!(guard.release(None, now_ms()), Ok(()));
    assert_eq!(machine.ledger_text(), "");
    let held = hold(&machine, &CapSource::MeteredPersonal, &input(""), 0.5).expect("held");
    guard.settle(held.as_ref(), 0.05, now_ms()).expect("settled");
    let other = hold(&machine, &CapSource::MeteredPersonal, &input(r#","workflow":"w2""#), 0.5).expect("held");
    guard.release(other.as_ref(), now_ms()).expect("released");
    let text = machine.ledger_text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines[1].starts_with(r#"{"client":"hermes","cost_class":"metered-personal","cost_usd":0.05,"kind":"settled","reserved_at""#), "{}", lines[1]);
    assert!(lines[3].ends_with(r#""reason":"not-started"}"#) && lines[3].contains(r#""cost_usd":0,"#), "{}", lines[3]);
}

fn planned(id: &str, run_id: &str, run: RunInput, usd: f64, cap: CapSource, cost_class: CostClass) -> PlannedReservation {
    PlannedReservation { id: id.to_string(), cap, cost_class, estimate: estimate(usd), input: run, run_dir: format!("/runs/{run_id}"), run_id: run_id.to_string(), window_s: 1800.0 }
}

#[test]
fn a_plan_that_fits_holds_every_estimate_in_one_locked_update_with_the_settlements_that_were_due() {
    let machine = Machine::new(DEFAULTS);
    machine.write_ledger(&[reservation_line("old", "/nowhere", 0.25, "2026-11-04T09:45:00.000Z", CostClass::MeteredPersonal)]);
    let plan = vec![
        planned("a", "run-a", input(""), 0.5, CapSource::MeteredPersonal, CostClass::MeteredPersonal),
        planned("b", "run-b", input(r#","client":"codex","model":"gpt-5.5""#), 1.0, CapSource::Grant(grant(5.0, 7200.0)), CostClass::MeteredEmployer),
    ];
    let config = config(&machine);
    let reserved = (CapGuard { runtime: &machine.runtime }).reserve_plan(&plan, &config, now_ms()).expect("reserved");
    assert!(reserved.refused.is_empty());
    assert_eq!(reserved.held.keys().collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(reserved.held["a"].time_limit_s, 900.0);
    assert_eq!(reserved.held["a"].record.expires_at, timestamps::to_iso(now_ms() + (1800 + 60) * 1000));
    let held: Vec<String> = machine.reservations().iter().map(|record| record.run_id.clone()).collect();
    assert_eq!(held, ["old", "run-a", "run-b"]);
    assert_eq!(machine.ledger_text().lines().count(), 4);
}

#[test]
fn a_plan_is_checked_in_order_with_the_estimates_before_it_already_held_and_holds_nothing_when_one_is_refused() {
    let machine = Machine::new(DEFAULTS);
    let plan = vec![
        planned("a", "run-a", input(""), 1.5, CapSource::MeteredPersonal, CostClass::MeteredPersonal),
        planned("b", "run-b", input(""), 1.5, CapSource::MeteredPersonal, CostClass::MeteredPersonal),
        planned("c", "run-c", input(r#","client":"codex","model":"gpt-5.5""#), 6.0, CapSource::Grant(grant(5.0, 7200.0)), CostClass::MeteredEmployer),
    ];
    let config = config(&machine);
    let reserved = (CapGuard { runtime: &machine.runtime }).reserve_plan(&plan, &config, now_ms()).expect("reserved");
    let refused: Vec<(&str, &str)> = reserved.refused.iter().map(|refusal| (refusal.id.as_str(), refusal.message.as_str())).collect();
    assert_eq!(
        refused,
        [
            ("b", "hermes:hermes-hard does not fit the per-workflow cap of $2.00: $0.00 spent and $1.50 reserved in workflow w, and its estimate is $1.50"),
            ("c", "codex:gpt-5.5 does not fit grant grant-1 of $5.00 for workflow w: $0.00 spent and $0.00 reserved, and its estimate is $6.00"),
        ]
    );
    assert!(machine.reservations().is_empty());
}

#[test]
fn the_reservations_of_runs_that_were_lost_are_given_back_by_what_their_results_say() {
    let machine = Machine::new(DEFAULTS);
    let guard = CapGuard { runtime: &machine.runtime };
    let root = machine.sandbox.text();
    let held = |run_id: &str| {
        reservation_record(&ReservationArgs {
            run_id,
            workflow: "w",
            client: "codex",
            cost_class: CostClass::MeteredEmployer,
            model: "gpt-5.5",
            estimate_usd: 1.0,
            reserved_at_ms: now_ms(),
            time_limit_s: 900.0,
            run_dir: &format!("{root}/{run_id}"),
        })
    };
    let (finished, never_ran) = (held("r1"), held("r2"));
    machine.write_at(&format!("{root}/r1/result.json"), r#"{"cost_usd":7}"#);
    let lines: Vec<String> = [&finished, &never_ran]
        .iter()
        .map(|record| reservation_line(&record.run_id, &record.run_dir, 1.0, &record.expires_at, CostClass::MeteredEmployer))
        .collect();
    machine.write_ledger(&lines);
    let records = [finished.clone(), never_ran.clone()];
    guard.release_unrun(&records, now_ms()).expect("released");
    // A second call finds both settled and leaves them alone.
    guard.release_unrun(&records, now_ms()).expect("released again");
    let settled: Vec<(String, f64, Option<String>)> = machine
        .ledger()
        .into_iter()
        .filter_map(|entry| match entry {
            LedgerEntry::Settled(record) => Some((record.run_id, record.cost_usd, record.reason)),
            _ => None,
        })
        .collect();
    assert_eq!(
        settled,
        [("r1".to_string(), 7.0, Some("expired".to_string())), ("r2".to_string(), 0.0, Some("not-run".to_string()))]
    );
}

#[test]
fn the_config_the_tests_read_is_the_one_they_name() {
    let config: GatewayConfig = parse_defaults(&parse(DEFAULTS).expect("JSON")).expect("defaults");
    assert_eq!(config.settings.metered_personal_caps, caps());
}
