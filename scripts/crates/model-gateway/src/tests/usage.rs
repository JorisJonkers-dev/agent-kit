//! Tests for the usage report: what every client can do now, what it has
//! spent, and how its plan windows stand.

use crate::caps::round;
use crate::config::{
    ClientConfig, CostClass, GatewayConfig, MeteredPersonalCaps, ModelSource, PlanLimit, PlanWindow, Settings,
};
use crate::json::{compact, parse, Value};
use crate::ledger::{LedgerEntry, LedgerRecord, ReservationRecord, RunTokens};
use crate::timestamps;
use crate::usage::{
    availability_of, by_day, by_workflow, describe_window, plan_windows, read_usage_input, refusal_reason,
    standing_refusals, totals_of, usage_report, warnings_of, Availability, ClientReading, ClientStatusReading, PlanState,
    PlanWindowUsage, SignedIn, StandingRefusal, UsageInput, DEFAULT_USAGE_DAYS,
};

/// 2026-11-04T12:00:00.000Z, the instant every test reads the ledger at.
fn now() -> i64 {
    timestamps::parse("2026-11-04T12:00:00.000Z").expect("a timestamp")
}

fn settings() -> Settings {
    Settings {
        plan_warning_ratio: 0.9,
        summary_max_chars: 2000.0,
        max_parallel_per_client: 4.0,
        max_batch_delegations: 8.0,
        availability_window_minutes: 60.0,
        metered_personal_caps: MeteredPersonalCaps { per_workflow_usd: 5.0, per_day_usd: 20.0 },
        output_allowance_tokens: [1000.0, 2000.0, 4000.0, 8000.0],
    }
}

fn client(binary: &str, cost_class: CostClass) -> ClientConfig {
    ClientConfig {
        binary: binary.to_string(),
        cost_class,
        model_source: ModelSource::Declared,
        families: None,
        provider_cost_classes: None,
        providers: None,
        plan_limits: None,
    }
}

fn config(clients: Vec<(&str, ClientConfig)>) -> GatewayConfig {
    GatewayConfig {
        version: 1.0,
        clients: clients.into_iter().map(|(id, client)| (id.to_string(), client)).collect(),
        models: vec![],
        settings: settings(),
        disabled: vec![],
    }
}

fn tokens(cached: f64, input: f64, output: f64) -> Option<RunTokens> {
    Some(RunTokens { input, output, cached })
}

fn record(at: &str, client: &str, cost_usd: f64, workflow: &str, reported: Option<RunTokens>) -> LedgerRecord {
    LedgerRecord {
        at: at.to_string(),
        client: client.to_string(),
        cost_class: None,
        cost_usd,
        model: "gpt-5.5".to_string(),
        ok: true,
        reason: None,
        unavailable_until: None,
        run_id: format!("{client}-{at}"),
        tokens: reported,
        workflow: workflow.to_string(),
    }
}

fn entry(at: &str, client: &str, cost_usd: f64, workflow: &str, reported: Option<RunTokens>) -> LedgerEntry {
    LedgerEntry::Run(record(at, client, cost_usd, workflow, reported))
}

fn failed(at: &str, client: &str, reason: &str, unavailable_until: Option<&str>) -> LedgerRecord {
    LedgerRecord {
        ok: false,
        reason: Some(reason.to_string()),
        unavailable_until: unavailable_until.map(str::to_string),
        ..record(at, client, 0.0, "spec-293", None)
    }
}

fn failed_of(at: &str, client: &str, model: &str, reason: &str, unavailable_until: Option<&str>) -> LedgerRecord {
    LedgerRecord { model: model.to_string(), ..failed(at, client, reason, unavailable_until) }
}

fn reading(client: &str, installed: bool, status: Option<ClientStatusReading>) -> ClientReading {
    ClientReading { client: client.to_string(), installed, status }
}

fn status(command: &str, signed_in: SignedIn, detail: &str) -> Option<ClientStatusReading> {
    Some(ClientStatusReading { command: command.to_string(), signed_in, detail: detail.to_string() })
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).expect("a member")
}

fn client_of(report: &Value, id: &str) -> Value {
    let Value::Arr(clients) = field(report, "clients") else {
        panic!("clients is a list")
    };
    clients
        .iter()
        .find(|client| field(client, "client").as_str() == Some(id))
        .expect("a client")
        .clone()
}

fn report(input: &UsageInput, config: &GatewayConfig, readings: &[ClientReading], entries: &[LedgerEntry]) -> Value {
    usage_report(input, config, readings, entries, now())
}

fn days(days: f64) -> UsageInput {
    UsageInput { days, workflow: None }
}

#[test]
fn read_usage_input_reads_the_days_and_the_workflow() {
    assert_eq!(read_usage_input(&parse("{}").expect("JSON")), Ok(UsageInput { days: DEFAULT_USAGE_DAYS, workflow: None }));
    assert_eq!(read_usage_input(&parse(r#"{"days":30}"#).expect("JSON")), Ok(days(30.0)));
    assert_eq!(
        read_usage_input(&parse(r#"{"days":1,"workflow":"spec-293"}"#).expect("JSON")),
        Ok(UsageInput { days: 1.0, workflow: Some("spec-293".to_string()) })
    );
    assert_eq!(DEFAULT_USAGE_DAYS, 7.0);
}

#[test]
fn read_usage_input_refuses_what_is_not_a_usage_call() {
    let cases = [
        ("[]", "the input must be a JSON object"),
        ("7", "the input must be a JSON object"),
        (r#"{"days":null}"#, "days must be a whole number of days above 0"),
        (r#"{"days":"7"}"#, "days must be a whole number of days above 0"),
        (r#"{"days":0}"#, "days must be a whole number of days above 0"),
        (r#"{"days":-1}"#, "days must be a whole number of days above 0"),
        (r#"{"days":1.5}"#, "days must be a whole number of days above 0"),
        (r#"{"workflow":""}"#, "workflow must be a non-empty string"),
        (r#"{"workflow":7}"#, "workflow must be a non-empty string"),
        (r#"{"workflow":"a b"}"#, "workflow \"a b\" is not a workflow name: a letter or digit, then letters, digits and . _ : / - only, at most 80 characters"),
    ];
    for (raw, message) in cases {
        let error = read_usage_input(&parse(raw).expect(raw)).expect_err(raw);
        assert_eq!(error.code, "invalid-input", "{raw}");
        assert_eq!(error.message, message, "{raw}");
    }
    let not_a_number = Value::Num(f64::NAN);
    let mut object = crate::json::Object::new();
    object.set("days", not_a_number);
    assert_eq!(
        read_usage_input(&Value::Obj(object)).expect_err("NaN").message,
        "days must be a whole number of days above 0"
    );
}

#[test]
fn usage_report_writes_every_client_with_its_availability_usage_and_quota() {
    let config = config(vec![
        ("codex", client("codex", CostClass::MeteredEmployer)),
        ("vibe", client("vibe", CostClass::Subsidised)),
    ]);
    let readings = vec![
        reading("codex", true, status("codex login status", SignedIn::Yes, "Logged in using ChatGPT")),
        reading("vibe", false, None),
    ];
    let entries = vec![
        entry("2026-11-04T09:00:00.000Z", "vibe", 0.5, "spec-293", tokens(0.0, 200.0, 20.0)),
        entry("2026-11-04T10:00:00.000Z", "codex", 9.0, "spec-293", tokens(0.0, 1.0, 1.0)),
    ];

    let printed = compact(&report(&days(7.0), &config, &readings, &entries));

    let expected = concat!(
        r#"{"clients":["#,
        r#"{"availability":{"installed":true,"reason":"codex login status: Logged in using ChatGPT","signed_in":"yes","status":"available"},"#,
        r#""by_day":[{"day":"2026-11-04","cost_usd":9,"runs":1,"tokens":{"cached":0,"input":1,"output":1}}],"#,
        r#""by_workflow":[{"workflow":"spec-293","cost_usd":9,"runs":1,"tokens":{"cached":0,"input":1,"output":1}}],"#,
        r#""client":"codex","cost_class":"metered-employer","plan_limits":[],"#,
        r#""quota":{"reason":"codex login status reports no quota","status":"unknown"},"#,
        r#""reserved_runs":0,"reserved_usd":0,"#,
        r#""usage":{"cost_usd":9,"runs":1,"tokens":{"cached":0,"input":1,"output":1}},"warnings":[]},"#,
        r#"{"availability":{"installed":false,"reason":"not installed: vibe is not on PATH","signed_in":"unknown","status":"unavailable"},"#,
        r#""by_day":[{"day":"2026-11-04","cost_usd":0.5,"runs":1,"tokens":{"cached":0,"input":200,"output":20}}],"#,
        r#""by_workflow":[{"workflow":"spec-293","cost_usd":0.5,"runs":1,"tokens":{"cached":0,"input":200,"output":20}}],"#,
        r#""client":"vibe","cost_class":"subsidised","plan_limits":[],"#,
        r#""quota":{"reason":"vibe has no status command, so the gateway cannot read its quota","status":"unknown"},"#,
        r#""reserved_runs":0,"reserved_usd":0,"#,
        r#""usage":{"cost_usd":0.5,"runs":1,"tokens":{"cached":0,"input":200,"output":20}},"warnings":[]}],"#,
        r#""days":7,"status":"ok"}"#,
    );
    assert_eq!(printed, expected);
}

#[test]
fn usage_report_narrows_to_one_workflow_and_names_it() {
    let config = config(vec![("vibe", client("vibe", CostClass::Subsidised))]);
    let readings = vec![reading("vibe", true, None)];
    let entries = vec![
        entry("2026-11-04T09:00:00.000Z", "vibe", 0.5, "spec-293", tokens(0.0, 200.0, 20.0)),
        entry("2026-11-04T10:00:00.000Z", "vibe", 0.125, "review", tokens(0.0, 50.0, 5.0)),
    ];
    let input = UsageInput { days: 7.0, workflow: Some("review".to_string()) };

    let printed = report(&input, &config, &readings, &entries);

    assert_eq!(field(&printed, "workflow").as_str(), Some("review"));
    assert_eq!(
        compact(field(&client_of(&printed, "vibe"), "usage")),
        r#"{"cost_usd":0.125,"runs":1,"tokens":{"cached":0,"input":50,"output":5}}"#
    );
    assert_eq!(
        compact(field(&client_of(&printed, "vibe"), "by_workflow")),
        r#"[{"workflow":"review","cost_usd":0.125,"runs":1,"tokens":{"cached":0,"input":50,"output":5}}]"#
    );
}

#[test]
fn usage_report_covers_the_last_days_days_ending_today() {
    let config = config(vec![("vibe", client("vibe", CostClass::Subsidised))]);
    let readings = vec![reading("vibe", true, None)];
    let entries = vec![
        entry("2026-10-28T09:00:00.000Z", "vibe", 1.0, "spec-293", tokens(0.0, 0.0, 0.0)),
        entry("2026-10-29T00:00:00.000Z", "vibe", 2.0, "spec-293", tokens(0.0, 0.0, 0.0)),
    ];

    assert_eq!(
        compact(field(&client_of(&report(&days(7.0), &config, &readings, &entries), "vibe"), "usage")),
        r#"{"cost_usd":2,"runs":1,"tokens":{"cached":0,"input":0,"output":0}}"#
    );
    assert_eq!(
        compact(field(&client_of(&report(&days(30.0), &config, &readings, &entries), "vibe"), "usage")),
        r#"{"cost_usd":3,"runs":2,"tokens":{"cached":0,"input":0,"output":0}}"#
    );
}

#[test]
fn usage_report_leaves_the_token_totals_unknown_for_a_run_that_reported_none() {
    let config = config(vec![("vibe", client("vibe", CostClass::Subsidised))]);
    let readings = vec![reading("vibe", true, None)];
    let entries = vec![entry("2026-11-04T09:00:00.000Z", "vibe", 0.5, "spec-293", None)];

    let printed = report(&days(7.0), &config, &readings, &entries);

    assert_eq!(compact(field(&client_of(&printed, "vibe"), "usage")), r#"{"cost_usd":0.5,"runs":1}"#);
    assert_eq!(
        compact(field(&client_of(&printed, "vibe"), "by_day")),
        r#"[{"day":"2026-11-04","cost_usd":0.5,"runs":1}]"#
    );
}

#[test]
fn usage_report_counts_the_runs_by_day_and_by_workflow_in_order() {
    let config = config(vec![("vibe", client("vibe", CostClass::Subsidised))]);
    let readings = vec![reading("vibe", true, None)];
    let entries = vec![
        entry("2026-11-04T10:00:00.000Z", "vibe", 0.125, "review", tokens(0.0, 50.0, 5.0)),
        entry("2026-11-03T09:00:00.000Z", "vibe", 0.25, "spec-293", tokens(0.0, 100.0, 10.0)),
        entry("2026-11-04T09:00:00.000Z", "vibe", 0.5, "spec-293", tokens(5.0, 200.0, 20.0)),
    ];

    let printed = report(&days(7.0), &config, &readings, &entries);

    assert_eq!(
        compact(field(&client_of(&printed, "vibe"), "by_day")),
        concat!(
            r#"[{"day":"2026-11-03","cost_usd":0.25,"runs":1,"tokens":{"cached":0,"input":100,"output":10}},"#,
            r#"{"day":"2026-11-04","cost_usd":0.625,"runs":2,"tokens":{"cached":5,"input":250,"output":25}}]"#,
        )
    );
    assert_eq!(
        compact(field(&client_of(&printed, "vibe"), "by_workflow")),
        concat!(
            r#"[{"workflow":"review","cost_usd":0.125,"runs":1,"tokens":{"cached":0,"input":50,"output":5}},"#,
            r#"{"workflow":"spec-293","cost_usd":0.75,"runs":2,"tokens":{"cached":5,"input":300,"output":30}}]"#,
        )
    );
}

#[test]
fn usage_report_holds_what_the_live_reservations_of_a_client_hold() {
    let config = config(vec![("codex", client("codex", CostClass::MeteredEmployer))]);
    let readings = vec![reading("codex", true, None)];
    let entries = vec![
        LedgerEntry::Reservation(ReservationRecord {
            run_id: "run-1".to_string(),
            workflow: "spec-293".to_string(),
            client: "codex".to_string(),
            cost_class: CostClass::MeteredEmployer,
            model: "gpt-5.5".to_string(),
            estimate_usd: 0.5,
            reserved_at: "2026-11-04T11:00:00.000Z".to_string(),
            expires_at: "2026-11-04T13:00:00.000Z".to_string(),
            run_dir: "/tmp/run".to_string(),
        }),
        LedgerEntry::Reservation(ReservationRecord {
            run_id: "run-2".to_string(),
            workflow: "spec-293".to_string(),
            client: "codex".to_string(),
            cost_class: CostClass::MeteredEmployer,
            model: "gpt-5.5".to_string(),
            estimate_usd: 0.25,
            reserved_at: "2026-11-04T11:00:00.000Z".to_string(),
            expires_at: "2026-11-04T13:00:00.000Z".to_string(),
            run_dir: "/tmp/run".to_string(),
        }),
    ];

    let printed = client_of(&report(&days(7.0), &config, &readings, &entries), "codex");

    assert_eq!(compact(field(&printed, "reserved_runs")), "2");
    assert_eq!(compact(field(&printed, "reserved_usd")), "0.75");
}

#[test]
fn usage_report_watches_each_plan_window_and_warns_at_the_warning_ratio() {
    let mut opencode = client("opencode", CostClass::Subsidised);
    opencode.plan_limits = Some(vec![
        PlanLimit::Usd { window: PlanWindow::FiveHours, limit_usd: 12.0 },
        PlanLimit::Usd { window: PlanWindow::Week, limit_usd: 30.0 },
        PlanLimit::Usd { window: PlanWindow::Month, limit_usd: 60.0 },
    ]);
    let config = config(vec![("opencode", opencode)]);
    let readings = vec![reading("opencode", true, None)];
    let entries = vec![
        entry("2026-11-02T10:00:00.000Z", "opencode", 25.0, "spec-293", None),
        entry("2026-11-04T09:00:00.000Z", "opencode", 3.0, "spec-293", None),
    ];

    let printed = client_of(&report(&days(7.0), &config, &readings, &entries), "opencode");

    assert_eq!(
        compact(field(&printed, "plan_limits")),
        concat!(
            r#"[{"limit_usd":12,"ratio":0.25,"spent_usd":3,"state":"ok","window":"5h"},"#,
            r#"{"limit_usd":30,"ratio":0.9333,"spent_usd":28,"state":"warning","window":"week"},"#,
            r#"{"limit_usd":60,"ratio":0.4667,"spent_usd":28,"state":"ok","window":"month"}]"#,
        )
    );
    assert_eq!(
        compact(field(&printed, "warnings")),
        r#"["opencode has used 93% of its week plan limit ($28.00 of $30.00)"]"#
    );
    assert_eq!(compact(field(field(&printed, "availability"), "status")), r#""available""#);
}

#[test]
fn usage_report_reports_a_client_at_100_percent_of_a_window_as_unable_to_start() {
    let mut opencode = client("opencode", CostClass::Subsidised);
    opencode.plan_limits = Some(vec![PlanLimit::Usd { window: PlanWindow::FiveHours, limit_usd: 12.0 }]);
    let config = config(vec![("opencode", opencode)]);
    let readings = vec![reading("opencode", true, None)];
    let entries = vec![entry("2026-11-04T09:00:00.000Z", "opencode", 12.0, "spec-293", None)];

    let printed = client_of(&report(&days(7.0), &config, &readings, &entries), "opencode");

    assert_eq!(
        compact(field(&printed, "availability")),
        r#"{"installed":true,"reason":"at 100% of its 5h plan limit ($12.00 of $12.00)","signed_in":"unknown","status":"unable_to_start"}"#
    );
    assert_eq!(
        compact(field(&printed, "plan_limits")),
        r#"[{"limit_usd":12,"ratio":1,"spent_usd":12,"state":"exhausted","window":"5h"}]"#
    );
}

#[test]
fn usage_report_counts_a_windows_runs_when_the_plan_limit_names_runs() {
    let mut vibe = client("vibe", CostClass::Subsidised);
    vibe.plan_limits = Some(vec![PlanLimit::Runs { window: PlanWindow::Day, limit_runs: 10.0 }]);
    let config = config(vec![("vibe", vibe)]);
    let readings = vec![reading("vibe", true, None)];
    let entries: Vec<LedgerEntry> = (0..9)
        .map(|hour| entry(&format!("2026-11-04T0{hour}:00:00.000Z"), "vibe", 0.0, "spec-293", None))
        .collect();

    let printed = client_of(&report(&days(7.0), &config, &readings, &entries), "vibe");

    assert_eq!(
        compact(field(&printed, "plan_limits")),
        r#"[{"limit_runs":10,"ratio":0.9,"runs":9,"state":"warning","window":"day"}]"#
    );
    assert_eq!(
        compact(field(&printed, "warnings")),
        r#"["vibe has used 90% of its day plan limit (9 of 10 runs)"]"#
    );
}

#[test]
fn usage_report_leaves_a_client_without_plan_limits_unwatched() {
    let config = config(vec![("vibe", client("vibe", CostClass::Subsidised))]);
    let readings = vec![reading("vibe", true, None)];
    let entries = vec![entry("2026-11-04T09:00:00.000Z", "vibe", 100.0, "spec-293", None)];

    let printed = client_of(&report(&days(7.0), &config, &readings, &entries), "vibe");

    assert_eq!(compact(field(&printed, "plan_limits")), "[]");
    assert_eq!(compact(field(&printed, "warnings")), "[]");
}

#[test]
fn usage_report_reads_a_client_it_has_no_reading_for_as_not_installed() {
    let config = config(vec![("vibe", client("vibe", CostClass::Subsidised))]);

    let printed = client_of(&report(&days(7.0), &config, &[], &[]), "vibe");

    assert_eq!(
        compact(field(&printed, "availability")),
        r#"{"installed":false,"reason":"not installed: vibe is not on PATH","signed_in":"unknown","status":"unavailable"}"#
    );
}

#[test]
fn availability_of_reports_a_client_that_is_not_installed_as_unavailable() {
    let not_installed = reading("vibe", false, None);

    let availability = availability_of("vibe", "vibe", Some(&not_installed), &[], now(), 60.0, &[]);

    assert_eq!(
        availability,
        Availability {
            status: "unavailable",
            installed: false,
            signed_in: SignedIn::Unknown,
            reason: "not installed: vibe is not on PATH".to_string(),
        }
    );
    assert_eq!(availability_of("vibe", "vibe", None, &[], now(), 60.0, &[]), availability);
}

#[test]
fn availability_of_reads_a_client_without_a_status_command_as_available() {
    let installed = reading("vibe", true, None);

    let availability = availability_of("vibe", "vibe", Some(&installed), &[], now(), 60.0, &[]);

    assert_eq!(availability.status, "available");
    assert_eq!(availability.signed_in, SignedIn::Unknown);
    assert_eq!(availability.reason, "vibe has no status command, so the gateway cannot tell whether it is signed in");
}

#[test]
fn availability_of_reports_a_client_whose_status_command_says_it_is_signed_out() {
    let signed_out = reading("codex", true, status("codex login status", SignedIn::No, "Not logged in"));

    let availability = availability_of("codex", "codex", Some(&signed_out), &[], now(), 60.0, &[]);

    assert_eq!(availability.status, "unavailable");
    assert_eq!(availability.signed_in, SignedIn::No);
    assert_eq!(availability.reason, "codex login status: Not logged in");
}

#[test]
fn availability_of_reports_a_client_at_100_percent_of_a_plan_window_as_unable_to_start() {
    let installed = reading("opencode", true, None);
    let windows = vec![
        PlanWindowUsage::Usd { window: PlanWindow::FiveHours, limit_usd: 12.0, spent_usd: 3.0, ratio: 0.25, state: PlanState::Ok },
        PlanWindowUsage::Usd { window: PlanWindow::Week, limit_usd: 30.0, spent_usd: 30.0, ratio: 1.0, state: PlanState::Exhausted },
    ];

    let availability = availability_of("opencode", "opencode", Some(&installed), &[], now(), 60.0, &windows);

    assert_eq!(availability.status, "unable_to_start");
    assert_eq!(availability.reason, "at 100% of its week plan limit ($30.00 of $30.00)");
}

#[test]
fn availability_of_reports_a_client_model_whose_answer_still_stands_as_unavailable() {
    let installed = reading("codex", true, status("codex login status", SignedIn::Yes, "Logged in using ChatGPT"));
    let records = vec![failed("2026-11-04T11:00:00.000Z", "codex", "rate-limited", Some("2026-11-04T13:00:00.000Z"))];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    let availability = availability_of("codex", "codex", Some(&installed), &mine, now(), 60.0, &[]);

    assert_eq!(availability.status, "unavailable");
    assert_eq!(availability.reason, "rate-limited at 2026-11-04T11:00:00.000Z: unavailable until 2026-11-04T13:00:00.000Z");
}

#[test]
fn availability_of_ignores_a_model_the_account_cannot_use() {
    let installed = reading("codex", true, None);
    let records = vec![failed("2026-11-04T11:00:00.000Z", "codex", "model-unavailable", None)];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    assert_eq!(availability_of("codex", "codex", Some(&installed), &mine, now(), 60.0, &[]).status, "available");
}

#[test]
fn availability_of_reports_a_client_rate_limited_in_the_window_as_unavailable() {
    let installed = reading("vibe", true, None);
    let records = vec![failed("2026-11-04T11:40:00.000Z", "vibe", "rate-limited", None)];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    let availability = availability_of("vibe", "vibe", Some(&installed), &mine, now(), 60.0, &[]);

    assert_eq!(availability.status, "unavailable");
    assert_eq!(availability.reason, "rate-limited at 2026-11-04T11:40:00.000Z, with no later run of it that succeeded");
}

#[test]
fn availability_of_lets_a_later_success_clear_the_errors_before_it() {
    let installed = reading("vibe", true, None);
    let records = vec![
        failed("2026-11-04T11:40:00.000Z", "vibe", "rate-limited", None),
        record("2026-11-04T11:45:00.000Z", "vibe", 0.0, "spec-293", None),
    ];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    assert_eq!(availability_of("vibe", "vibe", Some(&installed), &mine, now(), 60.0, &[]).status, "available");
}

#[test]
fn availability_of_ignores_an_error_older_than_the_window_and_takes_the_window_it_is_given() {
    let installed = reading("vibe", true, None);
    let records = vec![failed("2026-11-04T10:00:00.000Z", "vibe", "auth", None)];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    assert_eq!(availability_of("vibe", "vibe", Some(&installed), &mine, now(), 60.0, &[]).status, "available");
    let wide = availability_of("vibe", "vibe", Some(&installed), &mine, now(), 180.0, &[]);
    assert_eq!(wide.status, "unavailable");
    assert_eq!(wide.reason, "auth at 2026-11-04T10:00:00.000Z, with no later run of it that succeeded");
}

#[test]
fn availability_of_ignores_a_failure_that_is_not_a_rate_limit_or_auth_error() {
    let installed = reading("vibe", true, None);
    let records = vec![failed("2026-11-04T11:40:00.000Z", "vibe", "timeout", None)];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    assert_eq!(availability_of("vibe", "vibe", Some(&installed), &mine, now(), 60.0, &[]).status, "available");
}

#[test]
fn availability_of_ignores_a_failure_that_names_no_reason() {
    let installed = reading("vibe", true, None);
    let records = vec![LedgerRecord { reason: None, ..failed("2026-11-04T11:40:00.000Z", "vibe", "auth", None) }];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    assert_eq!(availability_of("vibe", "vibe", Some(&installed), &mine, now(), 60.0, &[]).status, "available");
}

#[test]
fn availability_of_counts_a_failure_whose_moment_does_not_parse() {
    let installed = reading("vibe", true, None);
    let records = vec![failed("soon", "vibe", "auth", None)];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    assert_eq!(availability_of("vibe", "vibe", Some(&installed), &mine, now(), 60.0, &[]).status, "unavailable");
}

#[test]
fn standing_refusals_keeps_an_answer_until_the_moment_it_named() {
    let records = vec![
        failed_of("2026-11-04T10:00:00.000Z", "codex", "gpt-5.5", "rate-limited", Some("2026-11-04T13:00:00.000Z")),
        failed_of("2026-11-04T10:30:00.000Z", "codex", "gpt-5.4", "model-unavailable", None),
    ];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    let standing = standing_refusals(&mine, now());

    assert_eq!(
        standing,
        vec![
            StandingRefusal {
                at: "2026-11-04T10:00:00.000Z".to_string(),
                client: "codex".to_string(),
                model: "gpt-5.5".to_string(),
                reason: "rate-limited at 2026-11-04T10:00:00.000Z: unavailable until 2026-11-04T13:00:00.000Z".to_string(),
            },
            StandingRefusal {
                at: "2026-11-04T10:30:00.000Z".to_string(),
                client: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                reason: "model-unavailable at 2026-11-04T10:30:00.000Z".to_string(),
            },
        ]
    );
    assert_eq!(
        standing[0].to_skipped_model(),
        crate::models::SkippedModel {
            client: "codex".to_string(),
            model: "gpt-5.5".to_string(),
            reason: "rate-limited at 2026-11-04T10:00:00.000Z: unavailable until 2026-11-04T13:00:00.000Z".to_string(),
        }
    );
}

#[test]
fn standing_refusals_drops_an_answer_whose_moment_passed_or_that_a_later_run_cleared() {
    let passed = vec![failed("2026-11-04T10:00:00.000Z", "codex", "rate-limited", Some("2026-11-04T11:00:00.000Z"))];
    let cleared = vec![
        failed("2026-11-04T10:00:00.000Z", "codex", "rate-limited", None),
        record("2026-11-04T10:30:00.000Z", "codex", 0.0, "spec-293", None),
    ];
    let unreadable = vec![failed("2026-11-04T10:00:00.000Z", "codex", "rate-limited", Some("soon"))];
    let other = vec![failed("2026-11-04T10:00:00.000Z", "codex", "timeout", None)];

    assert_eq!(standing_refusals(&passed.iter().collect::<Vec<_>>(), now()), vec![]);
    assert_eq!(standing_refusals(&cleared.iter().collect::<Vec<_>>(), now()), vec![]);
    assert_eq!(standing_refusals(&unreadable.iter().collect::<Vec<_>>(), now()), vec![]);
    assert_eq!(standing_refusals(&other.iter().collect::<Vec<_>>(), now()), vec![]);
}

#[test]
fn standing_refusals_takes_the_answer_that_ends_last_for_one_client_model() {
    let records = vec![
        failed("2026-11-04T10:00:00.000Z", "codex", "rate-limited", Some("2026-11-04T13:00:00.000Z")),
        failed("2026-11-04T10:30:00.000Z", "codex", "rate-limited", Some("2026-11-04T12:30:00.000Z")),
    ];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    let standing = standing_refusals(&mine, now());

    assert_eq!(standing.len(), 1);
    assert_eq!(standing[0].at, "2026-11-04T10:00:00.000Z");
}

#[test]
fn standing_refusals_keeps_the_files_order_for_a_moment_that_does_not_parse() {
    let records = vec![
        failed("soon", "codex", "rate-limited", None),
        failed("2026-11-04T10:00:00.000Z", "opencode", "rate-limited", None),
    ];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    let standing = standing_refusals(&mine, now());

    assert_eq!(standing.iter().map(|refusal| refusal.client.as_str()).collect::<Vec<_>>(), ["codex", "opencode"]);
}

#[test]
fn refusal_reason_names_the_answer_and_the_reset_it_named() {
    let named = failed("2026-11-04T10:00:00.000Z", "codex", "rate-limited", Some("2026-11-04T13:00:00.000Z"));
    let unnamed = failed("2026-11-04T10:00:00.000Z", "codex", "model-unavailable", None);
    let no_reason = LedgerRecord { reason: None, ..unnamed.clone() };

    assert_eq!(
        refusal_reason(&named),
        "rate-limited at 2026-11-04T10:00:00.000Z: unavailable until 2026-11-04T13:00:00.000Z"
    );
    assert_eq!(refusal_reason(&unnamed), "model-unavailable at 2026-11-04T10:00:00.000Z");
    assert_eq!(refusal_reason(&no_reason), "failed at 2026-11-04T10:00:00.000Z");
}

#[test]
fn totals_of_leaves_the_tokens_out_when_one_run_reported_none() {
    let known = vec![
        record("2026-11-04T09:00:00.000Z", "vibe", 0.25, "spec-293", tokens(0.0, 100.0, 10.0)),
        record("2026-11-04T10:00:00.000Z", "vibe", 0.5, "spec-293", tokens(5.0, 200.0, 20.0)),
    ];
    let mixed = vec![
        record("2026-11-04T09:00:00.000Z", "vibe", 0.25, "spec-293", tokens(0.0, 100.0, 10.0)),
        record("2026-11-04T10:00:00.000Z", "vibe", 0.5, "spec-293", None),
    ];

    let totals = totals_of(&known.iter().collect::<Vec<_>>());
    assert_eq!(totals.runs, 2.0);
    assert_eq!(totals.cost_usd, 0.75);
    assert_eq!(totals.tokens, tokens(5.0, 300.0, 30.0));
    assert_eq!(totals_of(&mixed.iter().collect::<Vec<_>>()).tokens, None);
    assert_eq!(totals_of(&[]).runs, 0.0);
}

#[test]
fn by_day_and_by_workflow_group_the_runs_and_sort_them() {
    let records = vec![
        record("2026-11-04T10:00:00.000Z", "vibe", 0.125, "review", tokens(0.0, 50.0, 5.0)),
        record("2026-11-03T09:00:00.000Z", "vibe", 0.25, "spec-293", tokens(0.0, 100.0, 10.0)),
        record("2026-11-04T09:00:00.000Z", "vibe", 0.5, "spec-293", tokens(5.0, 200.0, 20.0)),
    ];
    let mine: Vec<&LedgerRecord> = records.iter().collect();

    let days = by_day(&mine);
    assert_eq!(days.iter().map(|day| day.day.as_str()).collect::<Vec<_>>(), ["2026-11-03", "2026-11-04"]);
    assert_eq!(days[1].totals.cost_usd, 0.625);
    let workflows = by_workflow(&mine);
    assert_eq!(workflows.iter().map(|usage| usage.workflow.as_str()).collect::<Vec<_>>(), ["review", "spec-293"]);
    assert_eq!(workflows[1].totals.runs, 2.0);
}

#[test]
fn plan_windows_reaches_back_from_now_for_each_window() {
    let records = vec![
        record("2026-11-04T09:00:00.000Z", "opencode", 3.0, "spec-293", None),
        record("2026-11-02T10:00:00.000Z", "opencode", 25.0, "spec-293", None),
        record("2026-11-04T09:00:00.000Z", "vibe", 100.0, "spec-293", None),
    ];
    let mine: Vec<&LedgerRecord> = records.iter().collect();
    let limits = [
        PlanLimit::Usd { window: PlanWindow::FiveHours, limit_usd: 12.0 },
        PlanLimit::Runs { window: PlanWindow::Week, limit_runs: 2.0 },
    ];

    let windows = plan_windows("opencode", &limits, &mine, now(), 0.9);

    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].window(), PlanWindow::FiveHours);
    assert_eq!(windows[0].state(), PlanState::Ok);
    assert_eq!(windows[0].ratio(), 0.25);
    assert_eq!(windows[1].window(), PlanWindow::Week);
    assert_eq!(windows[1].state(), PlanState::Exhausted);
    assert_eq!(windows[1].ratio(), 1.0);
    assert_eq!(plan_windows("opencode", &[], &mine, now(), 0.9), vec![]);
}

#[test]
fn warnings_of_names_every_window_at_or_above_the_warning_ratio() {
    let windows = vec![
        PlanWindowUsage::Usd { window: PlanWindow::FiveHours, limit_usd: 12.0, spent_usd: 3.0, ratio: 0.25, state: PlanState::Ok },
        PlanWindowUsage::Usd { window: PlanWindow::Week, limit_usd: 30.0, spent_usd: 28.0, ratio: 0.9333, state: PlanState::Warning },
        PlanWindowUsage::Runs { window: PlanWindow::Day, limit_runs: 10.0, runs: 10.0, ratio: 1.0, state: PlanState::Exhausted },
    ];

    assert_eq!(
        warnings_of("opencode", &windows),
        vec![
            "opencode has used 93% of its week plan limit ($28.00 of $30.00)".to_string(),
            "opencode has used 100% of its day plan limit (10 of 10 runs)".to_string(),
        ]
    );
    assert_eq!(warnings_of("opencode", &windows[..1]), Vec::<String>::new());
}

#[test]
fn describe_window_names_the_span_and_what_is_spent_of_what_it_allows() {
    let usd = PlanWindowUsage::Usd { window: PlanWindow::Week, limit_usd: 30.0, spent_usd: 28.0, ratio: 0.9333, state: PlanState::Warning };
    let runs = PlanWindowUsage::Runs { window: PlanWindow::Day, limit_runs: 10.0, runs: 9.0, ratio: 0.9, state: PlanState::Warning };

    assert_eq!(describe_window(&usd), "its week plan limit ($28.00 of $30.00)");
    assert_eq!(describe_window(&runs), "its day plan limit (9 of 10 runs)");
}

#[test]
fn the_names_the_report_prints_are_the_ones_the_contract_names() {
    assert_eq!(SignedIn::Yes.as_str(), "yes");
    assert_eq!(SignedIn::No.as_str(), "no");
    assert_eq!(SignedIn::Unknown.as_str(), "unknown");
    assert_eq!(PlanState::Ok.as_str(), "ok");
    assert_eq!(PlanState::Warning.as_str(), "warning");
    assert_eq!(PlanState::Exhausted.as_str(), "exhausted");
    assert_eq!(round(0.93333, 4), 0.9333);
}
