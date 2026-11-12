//! Tests for the grant a run is held to, whether a client can start now and a
//! client's plan limit. Ported from the node runner's tests of the same modules.

use super::machine::{grant, now_ms, reservation_line, Machine, DEFAULTS};
use crate::availability::client_availability;
use crate::config::{CostClass, PlanWindow};
use crate::config_parse::load_config;
use crate::grant_guard::GrantGuard;
use crate::grants::{no_grant_message, Grant};
use crate::plan::{read_plan_limit, PlanLimitReading};
use crate::ports::ProcessResult;
use crate::timestamps;
use crate::usage::{PlanState, PlanWindowUsage};

fn other(client: &str, id: &str, workflow: &str, expires_in_s: f64) -> Grant {
    Grant { client: client.to_string(), id: id.to_string(), workflow: workflow.to_string(), ..grant(5.0, expires_in_s) }
}

#[test]
fn the_live_grant_covers_a_workflow_and_client_until_it_lapses() {
    let machine = Machine::new(DEFAULTS);
    machine.write_workflows(&[("w", "2026-11-04T08:00:00.000Z")]);
    machine.write_grants(&[grant(5.0, 7200.0), other("codex", "lapsed", "w2", -10.0)]);
    let guard = GrantGuard { runtime: &machine.runtime };
    assert_eq!(guard.live("w", "codex", now_ms()).expect("read").map(|live| live.id), Some("grant-1".to_string()));
    assert_eq!(guard.live("w", "opencode", now_ms()), Ok(None));
    assert_eq!(guard.live("w2", "codex", now_ms()), Ok(None));
    assert_eq!(guard.require("w", "codex", now_ms()).map(|live| live.id), Ok("grant-1".to_string()));
}

#[test]
fn a_run_without_a_live_grant_is_refused_naming_the_call_that_would_add_one() {
    let machine = Machine::new(DEFAULTS);
    let refused = (GrantGuard { runtime: &machine.runtime }).require("w", "codex", now_ms()).expect_err("no grant");
    assert_eq!((refused.code.as_str(), refused.message), ("no-grant", no_grant_message("w", "codex")));
}

#[test]
fn the_clients_a_live_grant_covers_are_those_of_the_workflow_that_have_not_lapsed() {
    let machine = Machine::new(DEFAULTS);
    machine.write_workflows(&[("w", "2026-11-04T08:00:00.000Z")]);
    machine.write_grants(&[
        grant(5.0, 7200.0),
        other("opencode", "lapsed", "w", -10.0),
        other("hermes", "elsewhere", "w2", 7200.0),
    ]);
    let clients = (GrantGuard { runtime: &machine.runtime }).live_clients("w", now_ms()).expect("read");
    assert_eq!(clients.into_iter().collect::<Vec<_>>(), ["codex"]);
}

#[test]
fn asking_for_a_grant_settles_the_reservations_whose_runs_died_first() {
    let machine = Machine::new(DEFAULTS);
    // The dead run's reservation of $4.50 would make a $5 grant look nearly spent; its expiry has passed.
    machine.write_grants(&[grant(5.0, 7200.0)]);
    machine.write_ledger(&[reservation_line("dead", "/nowhere", 4.5, "2026-11-04T09:45:00.000Z", CostClass::MeteredEmployer)]);
    let guard = GrantGuard { runtime: &machine.runtime };
    assert!(guard.live("w", "codex", now_ms()).expect("read").is_some());
    assert_eq!(machine.ledger_text().lines().count(), 2);
    assert_eq!(guard.live_clients("w", now_ms()).expect("read").len(), 1);
    assert_eq!(machine.ledger_text().lines().count(), 2);
}

#[test]
fn a_client_that_is_not_installed_cannot_start() {
    let machine = Machine::new(DEFAULTS);
    let config = load_config(&machine.runtime).expect("config");
    let availability = client_availability(&machine.runtime, &config, "claude").expect("read").expect("a client");
    assert_eq!((availability.status, availability.installed), ("unavailable", false));
    assert_eq!(availability.reason, "not installed: claude is not on PATH");
}

#[test]
fn an_installed_client_with_no_status_command_can_start_and_a_signed_out_one_cannot() {
    let machine = Machine::new(DEFAULTS);
    machine.process.install("claude");
    machine.process.install("codex");
    machine.process.answer(
        "codex",
        Ok(ProcessResult { stdout: String::new(), stderr: "Not logged in\n".to_string(), exit_code: 1, timed_out: false }),
    );
    let config = load_config(&machine.runtime).expect("config");
    let claude = client_availability(&machine.runtime, &config, "claude").expect("read").expect("a client");
    assert_eq!(claude.status, "available");
    assert_eq!(claude.reason, "claude has no status command, so the gateway cannot tell whether it is signed in");
    let codex = client_availability(&machine.runtime, &config, "codex").expect("read").expect("a client");
    assert_eq!((codex.status, codex.reason.as_str()), ("unavailable", "codex login status: Not logged in"));
}

#[test]
fn a_client_the_gateway_does_not_know_has_no_availability() {
    let machine = Machine::new(DEFAULTS);
    let config = load_config(&machine.runtime).expect("config");
    assert_eq!(client_availability(&machine.runtime, &config, "nobody"), Ok(None));
}

fn plan_machine() -> Machine {
    Machine::at(DEFAULTS, timestamps::parse("2026-11-04T10:52:15.000Z").expect("a timestamp"))
}

/// One ledger line of a Vibe run that cost `cost` at `at`.
fn vibe_run(at: &str, cost: f64) -> String {
    format!(
        r#"{{"at":"{at}","client":"vibe","cost_usd":{cost},"duration_s":1,"family":"mistral","mode":"read-only","model":"mistral-large-4-0-3","outcome":"ok","repository":null,"run_id":"run-{at}","tokens":{{"cached":0,"input":0,"output":0}},"workflow":"spec-293"}}"#
    )
}

fn plan_of(machine: &Machine, client: &str) -> PlanLimitReading {
    read_plan_limit(&machine.runtime, client).expect("a reading")
}

#[test]
fn a_client_the_machine_file_gives_no_plan_limit_is_unwatched() {
    let machine = plan_machine();
    machine.write_ledger(&[vibe_run("2026-11-04T09:00:00.000Z", 40.0)]);
    assert_eq!(plan_of(&machine, "vibe"), PlanLimitReading { client: "vibe".to_string(), windows: vec![], warnings: vec![], unable_to_start: None });
    assert_eq!(plan_of(&machine, "nobody").windows, vec![]);
}

#[test]
fn each_window_of_the_plan_is_reported_with_what_the_ledger_says_is_spent_in_it() {
    let machine = plan_machine();
    machine.write_ledger(&[vibe_run("2026-11-04T09:00:00.000Z", 3.0), vibe_run("2026-10-20T09:00:00.000Z", 20.0)]);
    machine.write_override(r#"{"clients":{"vibe":{"plan_limits":[{"limit_usd":12,"window":"5h"},{"limit_usd":60,"window":"month"}]}}}"#);
    let reading = plan_of(&machine, "vibe");
    assert_eq!(
        reading.windows,
        [
            PlanWindowUsage::Usd { window: PlanWindow::FiveHours, limit_usd: 12.0, spent_usd: 3.0, ratio: 0.25, state: PlanState::Ok },
            PlanWindowUsage::Usd { window: PlanWindow::Month, limit_usd: 60.0, spent_usd: 23.0, ratio: 0.3833, state: PlanState::Ok },
        ]
    );
    assert_eq!((reading.warnings, reading.unable_to_start), (vec![], None));
}

#[test]
fn a_window_at_the_warning_ratio_warns_in_the_gateways_own_words() {
    let machine = plan_machine();
    machine.write_ledger(&[vibe_run("2026-11-04T09:00:00.000Z", 10.0)]);
    machine.write_override(r#"{"clients":{"vibe":{"plan_limits":[{"limit_usd":12,"window":"5h"}]}}}"#);
    let reading = plan_of(&machine, "vibe");
    assert_eq!(reading.warnings, ["vibe has used 83% of its 5h plan limit ($10.00 of $12.00)"]);
    assert_eq!(reading.unable_to_start, None);
    assert_eq!(reading.windows[0].state(), PlanState::Warning);
}

#[test]
fn a_client_at_100_percent_of_a_window_cannot_start() {
    let machine = plan_machine();
    machine.write_ledger(&[vibe_run("2026-11-04T09:00:00.000Z", 12.0)]);
    machine.write_override(r#"{"clients":{"vibe":{"plan_limits":[{"limit_usd":12,"window":"5h"}]}}}"#);
    let reading = plan_of(&machine, "vibe");
    assert_eq!(reading.unable_to_start.as_deref(), Some("at 100% of its 5h plan limit ($12.00 of $12.00)"));
    assert_eq!(reading.warnings, ["vibe has used 100% of its 5h plan limit ($12.00 of $12.00)"]);
}

#[test]
fn a_windows_runs_are_counted_when_the_plan_limit_names_runs_instead_of_dollars() {
    let machine = plan_machine();
    machine.write_ledger(&[vibe_run("2026-11-04T09:00:00.000Z", 0.0), vibe_run("2026-11-04T08:00:00.000Z", 0.0)]);
    machine.write_override(r#"{"clients":{"vibe":{"plan_limits":[{"limit_runs":2,"window":"day"}]}}}"#);
    let reading = plan_of(&machine, "vibe");
    assert_eq!(reading.windows, [PlanWindowUsage::Runs { window: PlanWindow::Day, limit_runs: 2.0, runs: 2.0, ratio: 1.0, state: PlanState::Exhausted }]);
    assert_eq!(reading.unable_to_start.as_deref(), Some("at 100% of its day plan limit (2 of 2 runs)"));
}
