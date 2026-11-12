//! Tests for the caps: the lines the ledger holds, the settlements it needs,
//! the charges it counts, and the money the messages and the JSON print.

use crate::caps::{
    fits, live_reservations, metered_charges, round, run_records, settle_expired, settled_record_json, to_fixed, usd,
};
use crate::config::CostClass;
use crate::json::compact;
use crate::ledger::{LedgerEntry, LedgerRecord, ReservationRecord, SettledRecord};
use std::collections::BTreeMap;

fn record(run_id: &str, at: &str, cost_usd: f64, cost_class: Option<CostClass>) -> LedgerRecord {
    LedgerRecord {
        at: at.to_string(),
        client: "codex".to_string(),
        cost_class,
        cost_usd,
        model: "gpt-5.5".to_string(),
        ok: true,
        reason: None,
        unavailable_until: None,
        run_id: run_id.to_string(),
        tokens: None,
        workflow: "spec-293".to_string(),
    }
}

fn reservation(run_id: &str, estimate_usd: f64, cost_class: CostClass) -> ReservationRecord {
    ReservationRecord {
        run_id: run_id.to_string(),
        workflow: "spec-293".to_string(),
        client: "codex".to_string(),
        cost_class,
        model: "gpt-5.5".to_string(),
        estimate_usd,
        reserved_at: "2026-11-04T10:00:00.000Z".to_string(),
        expires_at: "2026-11-04T11:00:00.000Z".to_string(),
        run_dir: "/tmp/run".to_string(),
    }
}

fn settled(run_id: &str, cost_usd: f64, reason: Option<&str>, cost_class: CostClass) -> SettledRecord {
    SettledRecord {
        run_id: run_id.to_string(),
        workflow: "spec-293".to_string(),
        client: "codex".to_string(),
        cost_class,
        cost_usd,
        reserved_at: "2026-11-04T10:00:00.000Z".to_string(),
        settled_at: "2026-11-04T10:30:00.000Z".to_string(),
        reason: reason.map(str::to_string),
    }
}

#[test]
fn run_records_takes_every_line_that_is_not_a_reservation_or_a_settlement() {
    let entries = vec![
        LedgerEntry::Run(record("r1", "2026-11-04T10:00:00.000Z", 1.0, None)),
        LedgerEntry::Reservation(reservation("r2", 0.5, CostClass::MeteredEmployer)),
        LedgerEntry::Settled(settled("r2", 0.4, Some("expired"), CostClass::MeteredEmployer)),
        LedgerEntry::Run(record("r3", "2026-11-04T10:01:00.000Z", 2.0, None)),
    ];

    let runs = run_records(&entries);

    assert_eq!(runs.iter().map(|run| run.run_id.as_str()).collect::<Vec<_>>(), ["r1", "r3"]);
}

#[test]
fn live_reservations_holds_only_the_ones_no_settlement_released_and_no_run_line_counted() {
    let entries = vec![
        LedgerEntry::Reservation(reservation("live", 0.5, CostClass::MeteredEmployer)),
        LedgerEntry::Reservation(reservation("released", 0.5, CostClass::MeteredEmployer)),
        LedgerEntry::Settled(settled("released", 0.4, Some("expired"), CostClass::MeteredEmployer)),
        LedgerEntry::Reservation(reservation("ran", 0.5, CostClass::MeteredEmployer)),
        LedgerEntry::Run(record("ran", "2026-11-04T10:05:00.000Z", 0.4, Some(CostClass::MeteredEmployer))),
    ];

    let live = live_reservations(&entries);

    assert_eq!(live.iter().map(|held| held.run_id.as_str()).collect::<Vec<_>>(), ["live"]);
}

#[test]
fn settle_expired_settles_a_run_that_left_a_cost_and_one_whose_expiry_passed() {
    let entries = vec![
        LedgerEntry::Reservation(reservation("reported", 0.5, CostClass::MeteredEmployer)),
        LedgerEntry::Reservation(reservation("lapsed", 0.75, CostClass::MeteredEmployer)),
        LedgerEntry::Reservation(ReservationRecord {
            expires_at: "2026-11-04T13:00:00.000Z".to_string(),
            ..reservation("waiting", 0.25, CostClass::MeteredEmployer)
        }),
        LedgerEntry::Reservation(ReservationRecord { expires_at: "soon".to_string(), ..reservation("unreadable", 0.1, CostClass::MeteredEmployer) }),
    ];
    let costs = BTreeMap::from([("reported".to_string(), 0.4)]);

    let settlements = settle_expired(&entries, 1_793_793_600_000, &costs);

    assert_eq!(settlements.iter().map(|settle| settle.run_id.as_str()).collect::<Vec<_>>(), ["reported", "lapsed"]);
    assert_eq!(settlements[0].cost_usd, 0.4);
    assert_eq!(settlements[1].cost_usd, 0.75);
    assert_eq!(settlements[1].settled_at, "2026-11-04T12:00:00.000Z");
    assert_eq!(settlements[1].reason.as_deref(), Some("expired"));
    assert_eq!(settlements[1].reserved_at, "2026-11-04T10:00:00.000Z");
    assert_eq!(settlements[1].cost_class, CostClass::MeteredEmployer);
    assert_eq!(settlements[1].client, "codex");
    assert_eq!(settlements[1].workflow, "spec-293");
}

#[test]
fn metered_charges_counts_a_runs_own_line_at_the_moment_its_settlement_reserved() {
    let entries = vec![
        LedgerEntry::Run(record("r1", "2026-11-04T10:05:00.000Z", 1.5, Some(CostClass::MeteredEmployer))),
        LedgerEntry::Settled(settled("r1", 1.5, Some("expired"), CostClass::MeteredEmployer)),
        LedgerEntry::Run(record("r2", "2026-11-04T10:06:00.000Z", 2.0, Some(CostClass::MeteredEmployer))),
        LedgerEntry::Run(record("other-class", "2026-11-04T10:07:00.000Z", 9.0, Some(CostClass::Subsidised))),
        LedgerEntry::Run(record("no-class", "2026-11-04T10:08:00.000Z", 9.0, None)),
    ];

    let charges = metered_charges(&entries, CostClass::MeteredEmployer);

    assert_eq!(charges.len(), 2);
    assert_eq!(charges[0].at, "2026-11-04T10:00:00.000Z");
    assert_eq!(charges[0].usd, 1.5);
    assert_eq!(charges[0].client, "codex");
    assert_eq!(charges[0].workflow, "spec-293");
    assert_eq!(charges[1].at, "2026-11-04T10:06:00.000Z");
}

#[test]
fn metered_charges_counts_a_settlement_only_for_a_run_that_left_no_line() {
    let entries = vec![
        LedgerEntry::Settled(settled("died", 0.25, Some("expired"), CostClass::MeteredEmployer)),
        LedgerEntry::Settled(settled("never-ran", 0.0, Some("not-run"), CostClass::MeteredEmployer)),
        LedgerEntry::Settled(settled("never-started", 0.0, Some("not-started"), CostClass::MeteredEmployer)),
        LedgerEntry::Settled(settled("no-reason", 0.5, None, CostClass::MeteredEmployer)),
        LedgerEntry::Settled(settled("other-class", 0.5, Some("expired"), CostClass::MeteredPersonal)),
    ];

    let charges = metered_charges(&entries, CostClass::MeteredEmployer);

    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].usd, 0.25);
    assert_eq!(charges[0].at, "2026-11-04T10:00:00.000Z");
}

#[test]
fn metered_charges_reads_the_last_settlement_of_a_run() {
    let entries = vec![
        LedgerEntry::Run(record("r1", "2026-11-04T10:05:00.000Z", 1.5, Some(CostClass::MeteredEmployer))),
        LedgerEntry::Settled(settled("r1", 1.5, Some("expired"), CostClass::MeteredEmployer)),
        LedgerEntry::Settled(SettledRecord {
            reserved_at: "2026-11-04T09:00:00.000Z".to_string(),
            ..settled("r1", 1.5, Some("expired"), CostClass::MeteredEmployer)
        }),
    ];

    let charges = metered_charges(&entries, CostClass::MeteredEmployer);

    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].at, "2026-11-04T09:00:00.000Z");
}

#[test]
fn settled_record_json_writes_the_line_the_ledger_holds() {
    let with_reason = settled("r1", 0.4, Some("expired"), CostClass::MeteredEmployer);
    let without = settled("r2", 0.0, None, CostClass::MeteredPersonal);

    assert_eq!(
        compact(&settled_record_json(&with_reason)),
        r#"{"client":"codex","cost_class":"metered-employer","cost_usd":0.4,"kind":"settled","reason":"expired","reserved_at":"2026-11-04T10:00:00.000Z","run_id":"r1","settled_at":"2026-11-04T10:30:00.000Z","workflow":"spec-293"}"#
    );
    assert_eq!(
        compact(&settled_record_json(&without)),
        r#"{"client":"codex","cost_class":"metered-personal","cost_usd":0,"kind":"settled","reserved_at":"2026-11-04T10:00:00.000Z","run_id":"r2","settled_at":"2026-11-04T10:30:00.000Z","workflow":"spec-293"}"#
    );
}

#[test]
fn fits_holds_the_estimate_to_the_cent_the_ledger_counts_in() {
    assert!(fits(5.0, 1.0, 0.5, 3.5));
    assert!(!fits(5.0, 1.0, 0.5, 3.500001));
    assert!(fits(5.0, 0.1, 0.2, 0.3));
}

#[test]
fn round_rounds_a_half_towards_positive_infinity() {
    assert_eq!(round(0.5, 0), 1.0);
    assert_eq!(round(-0.5, 0), 0.0);
    assert_eq!(round(2.5, 0), 3.0);
    assert_eq!(round(-2.5, 0), -2.0);
    assert_eq!(round(0.1234565, 6), 0.123457);
    assert_eq!(round(0.93333, 4), 0.9333);
    assert_eq!(round(0.1 + 0.2, 6), 0.3);
}

#[test]
fn usd_names_dollars_as_javascript_prints_them() {
    assert_eq!(usd(0.0), "$0.00");
    assert_eq!(usd(1.0), "$1.00");
    assert_eq!(usd(0.125), "$0.13");
    assert_eq!(usd(-0.001), "$-0.00");
    assert_eq!(usd(1e21), "$1e+21");
    assert_eq!(usd(f64::NAN), "$NaN");
    assert_eq!(usd(f64::INFINITY), "$Infinity");
    assert_eq!(usd(f64::NEG_INFINITY), "$-Infinity");
}

#[test]
fn to_fixed_rounds_the_exact_binary_value_half_away_from_zero() {
    let cases = [
        (0.0, "0.00"),
        (0.1, "0.10"),
        (0.125, "0.13"),
        (0.13, "0.13"),
        (1.0, "1.00"),
        (1.005, "1.00"),
        (2.675, "2.67"),
        (0.005, "0.01"),
        (0.015, "0.01"),
        (123.456, "123.46"),
        (9.999, "10.00"),
        (0.999, "1.00"),
        (1e20, "100000000000000000000.00"),
        (1e-7, "0.00"),
        (-0.001, "-0.00"),
        (-0.125, "-0.13"),
        (-1.005, "-1.00"),
        (2.5, "2.50"),
        (0.375, "0.38"),
        (1e21, "1e+21"),
        (1.5e21, "1.5e+21"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
    ];
    for (value, expected) in cases {
        assert_eq!(to_fixed(value, 2), expected, "{value}");
    }
    assert_eq!(to_fixed(2.5, 0), "3");
    assert_eq!(to_fixed(3.5, 0), "4");
    assert_eq!(to_fixed(-2.5, 0), "-3");
    assert_eq!(to_fixed(9.9, 0), "10");
}
