//! Tests for the grants: the file they are read from and written to, the room
//! one holds, when it lapses, and the grant that covers a workflow now.

use crate::config::CostClass;
use crate::grants::{
    compact_timestamp, grant_file_text, grant_id, grant_json, grant_lapse, grant_room, grant_view_json, grant_views,
    lapsed_grant_json, live_grant, new_grant, parse_grants_file, Grant, GrantLapse, GrantView, NewGrant, GRANTS_VERSION,
    GRANT_TTL_MS,
};
use crate::json::{compact, pretty};
use crate::ledger::{LedgerEntry, LedgerRecord, ReservationRecord, SettledRecord};
use crate::timestamps;
use crate::workflows::WorkflowRecord;

fn now() -> i64 {
    timestamps::parse("2026-11-04T10:52:15.000Z").expect("a timestamp")
}

fn grant() -> Grant {
    Grant {
        answer: "yes, up to five dollars".to_string(),
        cap_usd: 5.0,
        max_starts: None,
        client: "codex".to_string(),
        expires_at: "2026-11-05T10:52:15.000Z".to_string(),
        id: "grant-20261104T105215Z-abc123".to_string(),
        question: "may I spend up to $5 of nedap money on spec-293?".to_string(),
        workflow: "spec-293".to_string(),
        written_at: "2026-11-04T10:52:15.000Z".to_string(),
        ended_at: None,
    }
}

fn run_record(at: &str, cost_usd: f64) -> LedgerRecord {
    LedgerRecord {
        at: at.to_string(),
        client: "codex".to_string(),
        cost_class: Some(CostClass::MeteredEmployer),
        cost_usd,
        model: "gpt-5.5".to_string(),
        ok: true,
        reason: None,
        unavailable_until: None,
        run_id: format!("run-{at}"),
        tokens: None,
        workflow: "spec-293".to_string(),
    }
}

fn run(at: &str, cost_usd: f64) -> LedgerEntry {
    LedgerEntry::Run(run_record(at, cost_usd))
}

fn reservation_record(run_id: &str, estimate_usd: f64) -> ReservationRecord {
    ReservationRecord {
        run_id: run_id.to_string(),
        workflow: "spec-293".to_string(),
        client: "codex".to_string(),
        cost_class: CostClass::MeteredEmployer,
        model: "gpt-5.5".to_string(),
        estimate_usd,
        reserved_at: "2026-11-04T10:52:15.000Z".to_string(),
        expires_at: "2026-11-04T11:08:15.000Z".to_string(),
        run_dir: "/tmp/run".to_string(),
    }
}

fn reservation(run_id: &str, estimate_usd: f64) -> LedgerEntry {
    LedgerEntry::Reservation(reservation_record(run_id, estimate_usd))
}

fn settled(run_id: &str, cost_usd: f64, reason: Option<&str>) -> LedgerEntry {
    LedgerEntry::Settled(SettledRecord {
        run_id: run_id.to_string(),
        workflow: "spec-293".to_string(),
        client: "codex".to_string(),
        cost_class: CostClass::MeteredEmployer,
        cost_usd,
        reserved_at: "2026-11-04T10:52:15.000Z".to_string(),
        settled_at: "2026-11-04T10:53:15.000Z".to_string(),
        reason: reason.map(str::to_string),
    })
}

fn running() -> WorkflowRecord {
    WorkflowRecord {
        name: "spec-293".to_string(),
        started_at: "2026-11-04T10:00:00.000Z".to_string(),
        ended_at: None,
        ended_last: false,
    }
}

fn grant_input(record: Option<&WorkflowRecord>) -> NewGrant<'_> {
    NewGrant {
        id: "grant-20261104T105215Z-abc123",
        workflow: "spec-293",
        client: "codex",
        cap_usd: 5.0,
        max_starts: None,
        question: "may I?",
        answer: "yes",
        written_at: now(),
        workflow_record: record,
    }
}

fn grants_file(grants: &str) -> String {
    format!(r#"{{"grants":[{grants}],"version":1}}"#)
}

/// One grant's JSON with the fields `overrides` names replaced or added.
fn grant_text(overrides: &[(&str, &str)]) -> String {
    let mut fields = vec![
        ("answer", "\"yes, up to five dollars\""),
        ("cap_usd", "5"),
        ("client", "\"codex\""),
        ("expires_at", "\"2026-11-05T10:52:15.000Z\""),
        ("id", "\"grant-20261104T105215Z-abc123\""),
        ("question", "\"may I spend up to $5 of nedap money on spec-293?\""),
        ("workflow", "\"spec-293\""),
        ("written_at", "\"2026-11-04T10:52:15.000Z\""),
    ];
    for (key, value) in overrides {
        match fields.iter_mut().find(|(held, _)| held == key) {
            Some(slot) => slot.1 = value,
            None => fields.push((key, value)),
        }
    }
    let body: Vec<String> = fields.iter().map(|(key, value)| format!("\"{key}\":{value}")).collect();
    grants_file(&format!("{{{}}}", body.join(",")))
}

#[test]
fn parse_grants_file_reads_the_grants_a_file_holds() {
    assert_eq!(parse_grants_file(&grant_text(&[])), Ok(vec![grant()]));
    assert_eq!(parse_grants_file(&grants_file("")), Ok(vec![]));
    let ended = parse_grants_file(&grant_text(&[("ended_at", "\"2026-11-04T10:53:00.000Z\"")])).expect("a grant");
    assert_eq!(ended[0].ended_at.as_deref(), Some("2026-11-04T10:53:00.000Z"));
}

#[test]
fn parse_grants_file_refuses_a_file_that_is_not_json_of_this_version_or_holds_no_list() {
    assert!(parse_grants_file("{ not json").expect_err("not JSON").starts_with("not JSON: "));
    assert_eq!(parse_grants_file(r#"{"grants":[],"version":2}"#).expect_err("version"), "version must be 1");
    assert_eq!(parse_grants_file(r#"{"grants":"none","version":1}"#).expect_err("list"), "grants must be a list");
    assert_eq!(parse_grants_file("[]").expect_err("object"), "the grants file must be a JSON object");
    assert_eq!(parse_grants_file(r#"{"version":1}"#).expect_err("list"), "grants must be a list");
}

#[test]
fn parse_grants_file_names_the_field_that_is_wrong() {
    let cases: [(&str, &[(&str, &str)], &str); 10] = [
        ("7", &[], "grants[0] must be an object"),
        ("{}", &[("cap_usd", "0")], "grants[0].cap_usd must be a number above 0"),
        ("{}", &[("cap_usd", "\"5\"")], "grants[0].cap_usd must be a number above 0"),
        ("{}", &[("answer", "\"\"")], "grants[0].answer must be a non-empty string"),
        ("{}", &[("client", "\"\"")], "grants[0].client must be a non-empty string"),
        ("{}", &[("expires_at", "\"soon\"")], "grants[0].expires_at must be an ISO-8601 UTC timestamp like 2026-11-04T10:52:15Z"),
        ("{}", &[("id", "\"\"")], "grants[0].id must be a non-empty string"),
        ("{}", &[("id", "7")], "grants[0].id must be a non-empty string"),
        ("{}", &[("question", "\"\"")], "grants[0].question must be a non-empty string"),
        ("{}", &[("workflow", "\"\"")], "grants[0].workflow must be a non-empty string"),
    ];
    for (body, overrides, expected) in cases {
        let text = if overrides.is_empty() { grants_file(body) } else { grant_text(overrides) };
        assert_eq!(parse_grants_file(&text).expect_err(body), expected, "{body}");
    }
    let written = grant_text(&[("written_at", "\"soon\"")]);
    assert_eq!(
        parse_grants_file(&written).expect_err("written_at"),
        "grants[0].written_at must be an ISO-8601 UTC timestamp like 2026-11-04T10:52:15Z"
    );
    let ended = grant_text(&[("ended_at", "\"soon\"")]);
    assert_eq!(
        parse_grants_file(&ended).expect_err("ended_at"),
        "grants[0].ended_at must be an ISO-8601 UTC timestamp like 2026-11-04T10:52:15Z"
    );
}

#[test]
fn parse_grants_file_refuses_a_timestamp_that_is_not_a_full_iso_utc_one() {
    let stamps = [
        "2026-11-04 10:52:15",
        "2026-11-04T10:52:15",
        "2026-11-04T10:52:15+02:00",
        "2026-11-04T10:52:15.000+00:00",
        "2026-11-04",
        "Nov 4 2026 10:52:15 UTC",
        "2026-11-04T10:52Z",
        "2026-02-30T10:52:15Z",
        "2026-11-04T24:00:00Z",
        "2026-13-04T10:52:15Z",
        " 2026-11-04T10:52:15Z",
        "0026-11-04T10:52:15Z",
        "2026-11-04T10:52:15.Z",
        "2026-11-04T10:52:15.1234Z",
        "2026-11-04T10:52:15Z ",
        "2026-11-04T10:52:1xZ",
        "2026-11-04T10:52:",
        "2026x11-04T10:52:15Z",
        "2026-11x04T10:52:15Z",
        "2026-11-04x10:52:15Z",
        "2026-11-04T10x52:15Z",
        "2026-11-04T10:52x15Z",
    ];
    for stamp in stamps {
        for field in ["expires_at", "written_at", "ended_at"] {
            let text = grant_text(&[(field, &format!("\"{stamp}\""))]);
            assert_eq!(
                parse_grants_file(&text).expect_err(stamp),
                format!("grants[0].{field} must be an ISO-8601 UTC timestamp like 2026-11-04T10:52:15Z"),
                "{stamp} in {field}"
            );
        }
    }
    let epoch = grant_text(&[("written_at", "1793000000000")]);
    assert_eq!(
        parse_grants_file(&epoch).expect_err("epoch"),
        "grants[0].written_at must be an ISO-8601 UTC timestamp like 2026-11-04T10:52:15Z"
    );
}

#[test]
fn parse_grants_file_reads_the_full_iso_utc_timestamps() {
    for stamp in ["2026-11-04T10:52:15Z", "2026-11-04T10:52:15.5Z", "2026-11-04T10:52:15.123Z", "2026-02-28T23:59:59.999Z"] {
        let text = grant_text(&[("written_at", &format!("\"{stamp}\""))]);
        assert_eq!(parse_grants_file(&text).expect(stamp)[0].written_at, stamp);
    }
}

#[test]
fn new_grant_ends_24_hours_after_it_was_written() {
    let written = new_grant(&grant_input(None)).expect("a grant");

    assert_eq!(written.expires_at, timestamps::to_iso(now() + GRANT_TTL_MS));
    assert_eq!(written.written_at, "2026-11-04T10:52:15.000Z");
    assert_eq!(written.ended_at, None);
    assert_eq!(written.id, "grant-20261104T105215Z-abc123");
    assert_eq!(written.workflow, "spec-293");
    assert_eq!(written.client, "codex");
    assert_eq!(written.cap_usd, 5.0);
    assert_eq!(written.question, "may I?");
    assert_eq!(written.answer, "yes");
}

#[test]
fn new_grant_ends_when_the_workflows_own_24_hours_run_out_first() {
    let late = WorkflowRecord { started_at: "2026-11-04T10:00:00.000Z".to_string(), ..running() };
    let early = WorkflowRecord { started_at: "2026-11-04T11:00:00.000Z".to_string(), ..running() };

    assert_eq!(new_grant(&grant_input(Some(&late))).expect("a grant").expires_at, "2026-11-05T10:00:00.000Z");
    assert_eq!(new_grant(&grant_input(Some(&early))).expect("a grant").expires_at, "2026-11-05T10:52:15.000Z");
}

#[test]
fn new_grant_refuses_a_workflow_that_already_ended_or_ran_out() {
    let ran_out = WorkflowRecord { started_at: "2026-11-03T10:00:00.000Z".to_string(), ..running() };
    let exactly = WorkflowRecord { started_at: "2026-11-03T10:52:15.000Z".to_string(), ..running() };
    let ended = WorkflowRecord { ended_at: Some("2026-11-04T10:30:00.000Z".to_string()), ..running() };

    let error = new_grant(&grant_input(Some(&ran_out))).expect_err("ran out");
    assert_eq!(error.code, "workflow-ended");
    assert_eq!(
        error.message,
        "workflow spec-293 ended at 2026-11-04T10:00:00.000Z: a grant for it would lapse on arrival, so start a new workflow"
    );
    assert_eq!(new_grant(&grant_input(Some(&exactly))).expect_err("exactly").code, "workflow-ended");
    let error = new_grant(&grant_input(Some(&ended))).expect_err("ended");
    assert_eq!(
        error.message,
        "workflow spec-293 ended at 2026-11-04T10:30:00.000Z: a grant for it would lapse on arrival, so start a new workflow"
    );
}

#[test]
fn grant_id_names_the_moment_it_was_written_and_a_suffix() {
    assert_eq!(grant_id(now(), "abc123"), "grant-20261104T105215Z-abc123");
    assert_eq!(compact_timestamp(now()), "20261104T105215Z");
    assert_eq!(compact_timestamp(0), "19700101T000000Z");
}

#[test]
fn grant_room_counts_what_the_workflow_spent_since_the_grant_and_what_live_reservations_hold() {
    let entries = vec![
        run("2026-11-04T10:00:00.000Z", 4.0),
        run("2026-11-04T10:55:00.000Z", 1.5),
        reservation("run-reserved", 0.5),
        LedgerEntry::Reservation(ReservationRecord {
            cost_class: CostClass::MeteredPersonal,
            run_id: "other-class".to_string(),
            ..reservation_record("x", 0.5)
        }),
        LedgerEntry::Reservation(ReservationRecord {
            workflow: "spec-294".to_string(),
            run_id: "other-workflow".to_string(),
            ..reservation_record("x", 0.5)
        }),
    ];

    let room = grant_room(&entries, &grant());

    assert_eq!(room.spent_usd, 1.5);
    assert_eq!(room.reserved_usd, 0.5);
    assert_eq!(room.remaining_usd, 3.0);
    assert_eq!(room.grant, grant());
}

#[test]
fn grant_room_counts_only_the_runs_and_reservations_of_the_client_the_grant_names() {
    let entries = vec![
        run("2026-11-04T10:55:00.000Z", 1.0),
        LedgerEntry::Run(LedgerRecord { client: "opencode".to_string(), ..run_record("2026-11-04T10:56:00.000Z", 2.0) }),
        reservation("run-reserved", 0.5),
        LedgerEntry::Reservation(ReservationRecord {
            client: "opencode".to_string(),
            run_id: "other-client".to_string(),
            ..reservation_record("x", 0.5)
        }),
    ];

    let room = grant_room(&entries, &grant());

    assert_eq!(room.spent_usd, 1.0);
    assert_eq!(room.reserved_usd, 0.5);
    assert_eq!(room.remaining_usd, 3.5);
}

#[test]
fn grant_room_counts_a_settled_run_that_left_no_line_of_its_own_and_never_twice() {
    let entries = vec![
        run("2026-11-04T10:55:00.000Z", 1.0),
        settled("run-settled", 1.0, None),
        settled("run-died", 0.25, Some("expired")),
    ];

    assert_eq!(grant_room(&entries, &grant()).spent_usd, 1.25);
}

#[test]
fn grant_room_holds_nothing_more_for_a_reservation_whose_run_wrote_its_own_line() {
    let entries = vec![run("2026-11-04T10:55:00.000Z", 1.0), reservation("run-2026-11-04T10:55:00.000Z", 0.5)];

    let room = grant_room(&entries, &grant());

    assert_eq!(room.reserved_usd, 0.0);
    assert_eq!(room.spent_usd, 1.0);
}

#[test]
fn grant_room_counts_nothing_when_the_grant_was_written_at_a_moment_that_does_not_parse() {
    let entries = vec![run("2026-11-04T10:55:00.000Z", 1.0), reservation("run-reserved", 0.5)];
    let unreadable = Grant { written_at: "soon".to_string(), ..grant() };

    let room = grant_room(&entries, &unreadable);

    assert_eq!(room.spent_usd, 0.0);
    assert_eq!(room.reserved_usd, 0.0);
    assert_eq!(room.remaining_usd, 5.0);
}

#[test]
fn grant_views_lists_a_live_grant_with_what_it_has_left() {
    let (live, lapsed) = grant_views(&[grant()], &[run("2026-11-04T10:55:00.000Z", 1.5)], &[running()], now());

    assert_eq!(lapsed, vec![]);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].spent_usd, 1.5);
    assert_eq!(live[0].reserved_usd, 0.0);
    assert_eq!(live[0].remaining_usd, 3.5);
    assert_eq!(live[0].grant, grant());
}

#[test]
fn grant_views_lapses_a_grant_whose_workflow_ended_naming_that_first() {
    let ended = WorkflowRecord { ended_at: Some("2026-11-04T10:30:00.000Z".to_string()), ..running() };
    let old = WorkflowRecord { started_at: "2026-11-03T09:00:00.000Z".to_string(), ..running() };

    let (live, lapsed) = grant_views(&[grant()], &[], &[ended], now());
    assert_eq!(live, vec![]);
    assert_eq!(lapsed[0].reason, GrantLapse::WorkflowEnded);
    assert_eq!(lapsed[0].view.remaining_usd, 5.0);
    assert_eq!(grant_views(&[grant()], &[], &[old], now()).1[0].reason, GrantLapse::WorkflowEnded);
}

#[test]
fn grant_views_lapses_a_grant_after_24_hours() {
    let later = timestamps::parse("2026-11-05T10:52:15.000Z").expect("a timestamp");
    let still_running = WorkflowRecord { started_at: "2026-11-04T11:00:00.000Z".to_string(), ..running() };

    assert_eq!(grant_views(&[grant()], &[], &[still_running], later).1[0].reason, GrantLapse::Expired);
}

#[test]
fn grant_views_lapses_a_grant_whose_cap_is_spent() {
    let spent = grant_views(&[grant()], &[run("2026-11-04T10:55:00.000Z", 5.0)], &[running()], now());

    assert_eq!(spent.1[0].reason, GrantLapse::CapSpent);
}

#[test]
fn grant_views_keeps_a_grant_live_while_its_cap_is_only_reserved() {
    let reserved = grant_views(&[grant()], &[reservation("run-reserved", 5.0)], &[running()], now());

    assert_eq!(reserved.1, vec![]);
    assert_eq!(reserved.0[0].remaining_usd, 0.0);
}

#[test]
fn grant_views_lapses_a_grant_someone_ended() {
    let ended = Grant { ended_at: Some("2026-11-04T10:53:00.000Z".to_string()), ..grant() };

    assert_eq!(grant_views(&[ended], &[], &[running()], now()).1[0].reason, GrantLapse::WorkflowEnded);
}

#[test]
fn grant_views_lists_a_grant_of_a_workflow_this_machine_never_started_as_live() {
    assert_eq!(grant_views(&[grant()], &[], &[], now()).0.len(), 1);
}

#[test]
fn grant_lapse_names_the_workflow_the_expiry_or_the_cap_in_that_order() {
    let room = grant_room(&[], &grant());
    let ended = WorkflowRecord { ended_at: Some("2026-11-04T10:30:00.000Z".to_string()), ..running() };
    let later = timestamps::parse("2026-11-05T10:52:15.000Z").expect("a timestamp");
    let spent = grant_room(&[run("2026-11-04T10:55:00.000Z", 5.0)], &grant());

    assert_eq!(grant_lapse(&grant(), &room, Some(&ended), now()), Some(GrantLapse::WorkflowEnded));
    assert_eq!(grant_lapse(&grant(), &room, None, later), Some(GrantLapse::Expired));
    assert_eq!(grant_lapse(&grant(), &spent, None, now()), Some(GrantLapse::CapSpent));
    assert_eq!(grant_lapse(&grant(), &room, None, now()), None);
    let unreadable = Grant { expires_at: "soon".to_string(), ..grant() };
    assert_eq!(grant_lapse(&unreadable, &room, None, now()), None);
    assert_eq!(GrantLapse::WorkflowEnded.as_str(), "workflow-ended");
    assert_eq!(GrantLapse::Expired.as_str(), "expired");
    assert_eq!(GrantLapse::CapSpent.as_str(), "cap-spent");
}

#[test]
fn live_grant_finds_the_live_grant_of_a_workflow_and_client() {
    assert_eq!(live_grant(&[grant()], "spec-293", "codex", &[], Some(&running()), now()), Some(grant()));
    assert_eq!(live_grant(&[grant()], "spec-294", "codex", &[], Some(&running()), now()), None);
    assert_eq!(live_grant(&[grant()], "spec-293", "opencode", &[], Some(&running()), now()), None);
    assert_eq!(
        live_grant(&[grant()], "spec-293", "codex", &[run("2026-11-04T10:55:00.000Z", 5.0)], Some(&running()), now()),
        None
    );
}

#[test]
fn live_grant_takes_the_last_live_grant_of_a_workflow() {
    let second = Grant { cap_usd: 9.0, id: "grant-20261104T105315Z-def456".to_string(), ..grant() };

    assert_eq!(live_grant(&[grant(), second.clone()], "spec-293", "codex", &[], Some(&running()), now()), Some(second));
}

#[test]
fn live_grant_takes_the_grant_written_last_whatever_order_the_file_holds_them_in() {
    let older = Grant { cap_usd: 3.0, id: "grant-older".to_string(), written_at: "2026-11-04T10:40:00.000Z".to_string(), ..grant() };
    let newer = Grant { cap_usd: 9.0, id: "grant-newer".to_string(), written_at: "2026-11-04T10:50:00.000Z".to_string(), ..grant() };

    assert_eq!(
        live_grant(&[newer.clone(), older.clone()], "spec-293", "codex", &[], Some(&running()), now()),
        Some(newer.clone())
    );
    assert_eq!(live_grant(&[older, newer.clone()], "spec-293", "codex", &[], Some(&running()), now()), Some(newer));
}

#[test]
fn live_grant_takes_the_later_one_in_the_file_when_two_were_written_at_the_same_moment() {
    let first = Grant { id: "grant-first".to_string(), ..grant() };
    let second = Grant { id: "grant-second".to_string(), ..grant() };

    assert_eq!(live_grant(&[first, second.clone()], "spec-293", "codex", &[], Some(&running()), now()), Some(second));
}

#[test]
fn live_grant_ignores_a_newer_grant_that_lapsed_and_takes_the_newest_live_one() {
    let older = Grant { id: "grant-older".to_string(), written_at: "2026-11-04T10:40:00.000Z".to_string(), ..grant() };
    let lapsed = Grant {
        expires_at: "2026-11-04T10:00:00.000Z".to_string(),
        id: "grant-lapsed".to_string(),
        written_at: "2026-11-04T10:50:00.000Z".to_string(),
        ..grant()
    };

    assert_eq!(live_grant(&[older.clone(), lapsed], "spec-293", "codex", &[], Some(&running()), now()), Some(older));
}

#[test]
fn live_grant_keeps_the_one_it_holds_when_a_written_at_does_not_parse() {
    let unreadable = Grant { id: "grant-unreadable".to_string(), written_at: "soon".to_string(), ..grant() };
    let readable = Grant { id: "grant-readable".to_string(), ..grant() };

    assert_eq!(
        live_grant(&[readable.clone(), unreadable.clone()], "spec-293", "codex", &[], Some(&running()), now()),
        Some(readable.clone())
    );
    assert_eq!(
        live_grant(&[unreadable.clone(), readable], "spec-293", "codex", &[], Some(&running()), now()),
        Some(unreadable)
    );
}

#[test]
fn grant_json_writes_the_keys_the_grants_file_holds() {
    assert_eq!(
        compact(&grant_json(&grant())),
        r#"{"answer":"yes, up to five dollars","cap_usd":5,"client":"codex","expires_at":"2026-11-05T10:52:15.000Z","id":"grant-20261104T105215Z-abc123","question":"may I spend up to $5 of nedap money on spec-293?","workflow":"spec-293","written_at":"2026-11-04T10:52:15.000Z"}"#
    );
    let ended = Grant { ended_at: Some("2026-11-04T10:53:00.000Z".to_string()), ..grant() };
    assert!(compact(&grant_json(&ended))
        .ends_with(r#""written_at":"2026-11-04T10:52:15.000Z","ended_at":"2026-11-04T10:53:00.000Z"}"#));
}

#[test]
fn grant_view_json_writes_the_grant_then_what_it_has_left() {
    let view = GrantView { grant: grant(), spent_usd: 1.5, reserved_usd: 0.5, remaining_usd: 3.0 };

    assert!(compact(&grant_view_json(&view))
        .ends_with(r#""written_at":"2026-11-04T10:52:15.000Z","remaining_usd":3,"reserved_usd":0.5,"spent_usd":1.5}"#));
}

#[test]
fn lapsed_grant_json_writes_the_view_then_why_it_lapsed() {
    let (_, lapsed) = grant_views(&[grant()], &[run("2026-11-04T10:55:00.000Z", 5.0)], &[running()], now());

    assert!(compact(&lapsed_grant_json(&lapsed[0]))
        .ends_with(r#""remaining_usd":0,"reserved_usd":0,"spent_usd":5,"reason":"cap-spent"}"#));
}

#[test]
fn grant_file_text_writes_the_pretty_file_the_adapter_holds() {
    let expected = r#"{
  "grants": [
    {
      "answer": "yes, up to five dollars",
      "cap_usd": 5,
      "client": "codex",
      "expires_at": "2026-11-05T10:52:15.000Z",
      "id": "grant-20261104T105215Z-abc123",
      "question": "may I spend up to $5 of nedap money on spec-293?",
      "workflow": "spec-293",
      "written_at": "2026-11-04T10:52:15.000Z"
    }
  ],
  "version": 1
}
"#;

    assert_eq!(grant_file_text(&[grant()]), expected);
    assert_eq!(grant_file_text(&[]), "{\n  \"grants\": [],\n  \"version\": 1\n}\n");
    assert_eq!(GRANTS_VERSION, 1.0);
    assert_eq!(pretty(&grant_json(&grant())).lines().count(), 10);
}
