//! Quota-aware tier choice: the windows summed from the ledger, the drop at the threshold, the key-level fallback,
//! the decision on the ledger line, and the settings that tune it.

use super::machine::{now_ms, FakeProcess, Machine};
use crate::config::Settings;
use crate::config_parse::parse_defaults;
use crate::json::{self, parse, Value};
use crate::ports::ProcessResult;
use crate::quota::{self, candidates, merge, walk, Account, Decision, Draw, KeyInfo, KeyOutcome, QuotaSettings, Reading, Source, Window};
use crate::timestamps;

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

fn settings() -> QuotaSettings {
    QuotaSettings::default()
}

fn allowance_draw(hours_ago: f64, usd: f64) -> Draw {
    Draw { at_ms: now_ms() - (hours_ago * HOUR as f64) as i64, allowance_usd: Some(usd), cost_usd: None, account: Some("opencode-go".into()) }
}

fn cost_draw(account: &str, usd: f64) -> Draw {
    Draw { at_ms: now_ms() - HOUR, allowance_usd: None, cost_usd: Some(usd), account: Some(account.into()) }
}

fn outcome(info: Option<KeyInfo>) -> KeyOutcome {
    info.map_or_else(|| KeyOutcome::Failed("no answer".into()), KeyOutcome::Info)
}

fn read(tier: &str, draws: &[Draw], key: Option<KeyInfo>) -> Reading {
    quota::reading_of(tier, draws, now_ms(), &settings(), &mut || outcome(key))
}

fn unread(tier: &str) -> Reading {
    Reading { tier: tier.into(), source: Source::None, accounts: vec![], unattributed_usd: 0.0 }
}

/// A reading whose Go account has these windows (5h, week, cycle) used, against the default limits.
fn go_reading(tier: &str, used: [f64; 3]) -> Reading {
    let limits = [12.0, 30.0, 40.0];
    let names = ["5h", "week", "cycle"];
    Reading {
        tier: tier.into(),
        source: Source::Allowance,
        unattributed_usd: 0.0,
        accounts: vec![Account { name: "opencode-go".into(), windows: (0..3).map(|at| Window { name: names[at].into(), used: used[at], limit: limits[at] }).collect() }],
    }
}

fn vibe_reading(used: [f64; 4]) -> Reading {
    let accounts = ["vibe", "vibe-study", "vibe-api", "vibe-study-api"];
    let caps = [255.0, 127.0, 25.5, 12.7];
    Reading {
        tier: "tier-vibe".into(),
        source: Source::Account,
        unattributed_usd: 0.0,
        accounts: (0..4).map(|at| Account { name: accounts[at].into(), windows: vec![Window { name: "month".into(), used: used[at], limit: caps[at] }] }).collect(),
    }
}

/// The synthetic states: what each tier's reading is, by tier. A tier not named has no signal.
fn states(go: Option<[f64; 3]>, vibe: Option<[f64; 4]>) -> impl FnMut(&str) -> Reading {
    move |tier| match tier {
        "tier-go-cheap" | "tier-strong-open" => go.map_or_else(|| unread(tier), |used| go_reading(tier, used)),
        "tier-vibe" => vibe.map_or_else(|| unread(tier), vibe_reading),
        other => unread(other),
    }
}

fn chosen(requested: &str, go: Option<[f64; 3]>, vibe: Option<[f64; 4]>) -> Decision {
    walk(&candidates(requested), 0.8, &mut states(go, vibe))
}

#[test]
fn the_tier_drops_to_the_next_cheaper_one_at_80_percent_of_a_go_window() {
    // (the requested tier, Go's 5h, week and cycle use in USD of 12, 30 and 40, the tier chosen)
    let table: [(&str, [f64; 3], &str); 15] = [
        ("tier-strong-open", [0.0, 0.0, 0.0], "tier-strong-open"),
        ("tier-strong-open", [9.5, 0.0, 0.0], "tier-strong-open"),
        ("tier-strong-open", [9.6, 0.0, 0.0], "tier-vibe"),
        ("tier-strong-open", [12.0, 0.0, 0.0], "tier-vibe"),
        ("tier-strong-open", [0.0, 23.9, 0.0], "tier-strong-open"),
        ("tier-strong-open", [0.0, 24.0, 0.0], "tier-vibe"),
        ("tier-strong-open", [0.0, 0.0, 31.9], "tier-strong-open"),
        ("tier-strong-open", [0.0, 0.0, 32.0], "tier-vibe"),
        ("tier-go-cheap", [9.6, 0.0, 0.0], "tier-vibe"),
        ("tier-go-cheap", [9.0, 23.0, 31.0], "tier-go-cheap"),
        // Two windows at the threshold: the one nearest its limit is the one the reason names.
        ("tier-go-cheap", [10.0, 28.0, 0.0], "tier-vibe"),
        ("tier-go-cheap", [0.0, 24.0, 0.0], "tier-vibe"),
        ("tier-vibe", [12.0, 30.0, 40.0], "tier-vibe"),
        ("tier-free", [12.0, 30.0, 40.0], "tier-free"),
        ("tier-review", [12.0, 30.0, 40.0], "tier-review"),
    ];
    for (requested, used, wanted) in table {
        let decision = chosen(requested, Some(used), None);
        assert_eq!(decision.chosen, wanted, "{requested} at {used:?}: {}", decision.reason);
        assert_eq!(decision.requested, requested);
    }
}

#[test]
fn a_drop_never_climbs_and_skips_a_rung_whose_plan_is_the_one_that_ran_out() {
    // Go is out: tier-strong-open passes over tier-go-cheap (the same plan) and lands on tier-vibe.
    let decision = chosen("tier-strong-open", Some([10.0, 0.0, 0.0]), None);
    let tiers: Vec<&str> = decision.readings.iter().map(|reading| reading.tier.as_str()).collect();
    assert_eq!(tiers, ["tier-strong-open", "tier-go-cheap", "tier-vibe"]);
    assert_eq!(decision.chosen, "tier-vibe");
    // Vibe is out too (every account at 80%): the run lands on the free tier, the floor.
    let both = chosen("tier-strong-open", Some([10.0, 0.0, 0.0]), Some([204.0, 101.6, 20.4, 10.2]));
    assert_eq!(both.chosen, "tier-free");
    assert!(both.reason.contains("tier-strong-open out of quota (allowance): opencode-go 5h at 83% of 12"), "{}", both.reason);
    assert!(both.reason.contains("tier-vibe out of quota (account): vibe month at 80% of 255"), "{}", both.reason);
    assert!(both.reason.ends_with("dropped to tier-free"), "{}", both.reason);
    // A tier never drops to a dearer one: every candidate is the requested tier or a cheaper rung.
    for tier in quota::LADDER {
        let rungs: Vec<usize> = candidates(tier).iter().map(|name| quota::LADDER.iter().position(|rung| rung == name).expect("a rung")).collect();
        assert!(rungs.windows(2).all(|pair| pair[0] > pair[1]), "{tier}: {rungs:?}");
        assert_eq!(candidates(tier)[0], tier);
    }
    assert_eq!(candidates("tier-review"), ["tier-review"]);
}

#[test]
fn a_vibe_tier_with_an_account_left_still_serves_and_one_out_of_every_account_does_not() {
    assert!(chosen("tier-go-cheap", Some([10.0, 28.0, 0.0]), None).reason.starts_with("tier-go-cheap out of quota (allowance): opencode-go week at 93% of 30"));
    assert_eq!(chosen("tier-vibe", None, Some([255.0, 0.0, 0.0, 0.0])).chosen, "tier-vibe");
    assert_eq!(chosen("tier-vibe", None, Some([255.0, 127.0, 25.5, 12.7])).chosen, "tier-free");
    assert_eq!(chosen("tier-vibe", None, Some([204.0, 101.6, 20.4, 10.1])).chosen, "tier-vibe");
}

#[test]
fn a_decision_says_why_and_what_was_read() {
    let within = chosen("tier-go-cheap", Some([1.0, 3.0, 4.0]), None);
    assert_eq!(within.reason, "within-quota");
    let none = chosen("tier-go-cheap", None, None);
    assert_eq!((none.chosen.as_str(), none.reason.as_str()), ("tier-go-cheap", "no-signal"));
    let json = within.to_json();
    let object = json.as_object().expect("an object");
    assert_eq!(object.get("requested").and_then(Value::as_str), Some("tier-go-cheap"));
    assert_eq!(object.get("chosen").and_then(Value::as_str), Some("tier-go-cheap"));
    let Some(Value::Arr(signals)) = object.get("signals") else { panic!("signals") };
    assert_eq!(signals.len(), 3);
    let five_hour = signals[0].as_object().expect("a signal");
    assert_eq!(five_hour.get("window").and_then(Value::as_str), Some("5h"));
    assert_eq!(five_hour.get("ratio"), Some(&Value::Num(0.0833)));
    assert_eq!(five_hour.get("source").and_then(Value::as_str), Some("allowance"));
    // A tier with no signal still appears, saying so.
    let Some(Value::Arr(unseen)) = none.to_json().as_object().and_then(|object| object.get("signals").cloned()) else { panic!("signals") };
    assert_eq!(unseen.len(), 1);
    assert_eq!(unseen[0].as_object().and_then(|object| object.get("source")).and_then(Value::as_str), Some("none"));
    for (source, name) in [(Source::Allowance, "allowance"), (Source::Account, "account"), (Source::Key, "key"), (Source::KeyError, "key-error"), (Source::None, "none")] {
        assert_eq!(source.as_str(), name);
    }
}

#[test]
fn when_every_tier_down_the_ladder_is_out_the_last_one_stays() {
    let out = |tier: &str| Reading { tier: tier.into(), source: Source::Key, accounts: vec![Account { name: "key".into(), windows: vec![Window { name: "key".into(), used: 9.0, limit: 10.0 }] }], unattributed_usd: 0.0 };
    let decision = walk(&candidates("tier-vibe"), 0.8, &mut |tier| out(tier));
    assert_eq!(decision.chosen, "tier-free");
    assert!(decision.reason.ends_with("nothing cheaper than tier-free, so it stays"), "{}", decision.reason);
    let alone = walk(&["tier-review".to_string()], 0.8, &mut |tier| out(tier));
    assert_eq!(alone.chosen, "tier-review");
    assert!(alone.reason.contains("tier-review out of quota (key): key key at 90% of 10"), "{}", alone.reason);
    // An account with no windows is never out.
    let empty = Reading { tier: "t".into(), source: Source::Key, accounts: vec![Account { name: "a".into(), windows: vec![] }], unattributed_usd: 0.0 };
    assert_eq!(walk(&["t".to_string()], 0.8, &mut |_| empty.clone()).reason, "within-quota");
}

#[test]
fn go_windows_are_summed_from_the_allowance_the_ledger_recorded() {
    // 5h: 3 + 2 inside it; 6h and 8 days ago are outside it; the week holds all but the 8-day-old one.
    let draws = [allowance_draw(1.0, 3.0), allowance_draw(4.0, 2.0), allowance_draw(6.0, 10.0), allowance_draw(8.0 * 24.0, 1.0)];
    let reading = read("tier-strong-open", &draws, None);
    assert_eq!(reading.source, Source::Allowance);
    let windows = &reading.accounts[0].windows;
    assert_eq!(windows.iter().map(|window| (window.name.as_str(), window.used, window.limit)).collect::<Vec<_>>(), [("5h", 5.0, 12.0), ("week", 15.0, 30.0), ("cycle", 16.0, 40.0)]);
    // The cycle is anchored at 2026-11-04T10:41Z, so at 10:00 on the 4th the previous cycle (from 2026-10-05T10:41Z) is open.
    assert_eq!(timestamps::to_iso(now_ms()), "2026-11-04T10:00:00.000Z");
    let draws = [Draw { at_ms: timestamps::parse("2026-10-05T10:40:00.000Z").expect("time"), ..allowance_draw(0.0, 7.0) }, allowance_draw(1.0, 1.0)];
    assert_eq!(read("tier-go-cheap", &draws, None).accounts[0].windows[2].used, 1.0);
    // A draw in the future of now is not counted; with the cycle empty there is no signal.
    assert_eq!(read("tier-go-cheap", &[Draw { at_ms: now_ms() + DAY, ..allowance_draw(0.0, 5.0) }], Some(KeyInfo { spend: 0.0, max_budget: None })).source, Source::None);
    // The cycle turns over at 10:41: the draw an hour and 42 minutes before now is in the previous cycle, and still
    // in the 5-hour and weekly windows, which are read on their own. It is a signal, not "no signal".
    let later = timestamps::parse("2026-11-04T10:42:00.000Z").expect("time");
    let anchored = quota::reading_of("tier-go-cheap", &[allowance_draw(1.0, 8.0)], later, &settings(), &mut || outcome(None));
    assert_eq!(anchored.source, Source::Allowance, "{anchored:?}");
    let used: Vec<(&str, f64)> = anchored.accounts[0].windows.iter().map(|window| (window.name.as_str(), window.used)).collect();
    assert_eq!(used, [("5h", 8.0), ("week", 8.0), ("cycle", 0.0)]);
    // With 10 of 12 USD drawn in the last 5 hours across the turn-over, the tier drops: it does not fail open.
    let hot = quota::reading_of("tier-go-cheap", &[allowance_draw(1.0, 10.0)], later, &settings(), &mut || outcome(None));
    assert!(walk(&["tier-go-cheap".into(), "tier-vibe".into()], 0.8, &mut |tier| if tier == "tier-go-cheap" { hot.clone() } else { unread(tier) }).chosen == "tier-vibe");
    // A draw older than a week and the cycle is no signal at all.
    assert_eq!(quota::reading_of("tier-go-cheap", &[allowance_draw(9.0 * 24.0, 8.0)], later, &settings(), &mut || outcome(None)).source, Source::KeyError);
}

#[test]
fn vibe_accounts_are_summed_from_the_price_litellm_put_on_each_request_since_the_first_of_the_month() {
    let early = Draw { at_ms: timestamps::parse("2026-10-31T23:00:00.000Z").expect("time"), ..cost_draw("vibe", 100.0) };
    let reading = read("tier-vibe", &[cost_draw("vibe", 3.0), cost_draw("vibe", 4.0), cost_draw("vibe-api", 1.5), early, allowance_draw(1.0, 9.0)], None);
    assert_eq!(reading.source, Source::Account);
    let used: Vec<(&str, f64, f64)> = reading.accounts.iter().map(|account| (account.name.as_str(), account.windows[0].used, account.windows[0].limit)).collect();
    assert_eq!(used, [("vibe", 7.0, 255.0), ("vibe-study", 0.0, 127.0), ("vibe-api", 1.5, 25.5), ("vibe-study-api", 0.0, 12.7)]);
    // Cost LiteLLM priced without naming a deployment is counted in no account, and is reported as unattributed.
    let loose = Draw { account: None, ..cost_draw("x", 2.5) };
    let with_loose = read("tier-vibe", &[cost_draw("vibe", 1.0), loose.clone()], None);
    assert_eq!(with_loose.unattributed_usd, 2.5);
    assert_eq!(with_loose.accounts[0].windows[0].used, 1.0);
    let json = crate::quota::Decision { requested: "tier-vibe".into(), chosen: "tier-vibe".into(), reason: String::new(), readings: vec![with_loose] }.to_json();
    assert!(crate::json::compact(&json).contains(r#""account":"unattributed","window":"month","used":2.5"#), "{}", crate::json::compact(&json));
    // A month that has just turned over has no draw in it, but the days before it do: a signal of zero, so the key is
    // not asked (the closure would fail the read).
    let first = timestamps::parse("2026-12-01T00:30:00.000Z").expect("time");
    let fresh = quota::reading_of("tier-vibe", &[Draw { at_ms: first - HOUR, ..cost_draw("vibe", 200.0) }], first, &settings(), &mut || outcome(None));
    assert_eq!((fresh.source, fresh.accounts[0].windows[0].used), (Source::Account, 0.0));
    // Draws on a plan that is not Vibe's say nothing of it, and a tier with no plan has no reading.
    assert_eq!(read("tier-vibe", &[allowance_draw(1.0, 9.0)], Some(KeyInfo { spend: 0.0, max_budget: None })).source, Source::None);
    assert_eq!(read("tier-free", &[cost_draw("vibe", 9.0)], Some(KeyInfo { spend: 9.0, max_budget: Some(10.0) })).source, Source::None);
}

#[test]
fn with_no_allowance_signal_the_key_level_cap_decides() {
    let key = |spend: f64, max_budget: Option<f64>| Some(KeyInfo { spend, max_budget });
    for (tier, info, wanted) in [
        ("tier-strong-open", key(8.0, Some(10.0)), "tier-vibe"),
        ("tier-strong-open", key(7.9, Some(10.0)), "tier-strong-open"),
        ("tier-go-cheap", key(10.0, Some(10.0)), "tier-vibe"),
        ("tier-vibe", key(8.0, Some(10.0)), "tier-free"),
        ("tier-vibe", key(5.0, Some(10.0)), "tier-vibe"),
        // A key with no cap, a cap of nothing, or no answer at all says nothing: the tier is as asked.
        ("tier-strong-open", key(99.0, None), "tier-strong-open"),
        ("tier-strong-open", key(99.0, Some(0.0)), "tier-strong-open"),
        // A key that cannot be read is not a key with no cap: the plan cannot be checked, so the tier drops.
        ("tier-strong-open", None, "tier-vibe"),
        ("tier-vibe", None, "tier-free"),
    ] {
        // The two Go tiers share one key; Vibe's is another, which says nothing unless it is the one asked for.
        let mut read_tier = |name: &str| quota::reading_of(name, &[], now_ms(), &settings(), &mut || match info {
            _ if name == "tier-vibe" && tier != name => KeyOutcome::Info(KeyInfo { spend: 0.0, max_budget: None }),
            info => outcome(info),
        });
        let decision = walk(&candidates(tier), 0.8, &mut read_tier);
        assert_eq!(decision.chosen, wanted, "{tier} {info:?}: {}", decision.reason);
        if matches!(info, Some(KeyInfo { max_budget: None, .. })) {
            assert_eq!(decision.reason, "no-signal");
        }
        if info.is_none() {
            assert!(decision.reason.contains("key-error") || decision.reason.contains("key (no answer) unreadable at 100%"), "{}", decision.reason);
        }
    }
    // The ledger's allowance wins over the key when it has something to say; the key is not even asked.
    let mut asked = false;
    let reading = quota::reading_of("tier-go-cheap", &[allowance_draw(1.0, 1.0)], now_ms(), &settings(), &mut || {
        asked = true;
        outcome(None)
    });
    assert_eq!((reading.source, asked), (Source::Allowance, false));
}

// ---- reading the key, and the ledger, off a machine ----

const TIERS: &str = r#"{
  "tier-free": {"description": "d", "models": ["pi-free:tier-free"], "tier": "tier-free", "key_models": ["free"]},
  "tier-vibe": {"description": "d", "models": ["pi-vibe:tier-vibe"], "tier": "tier-vibe", "key_models": ["vibe"]},
  "tier-go-cheap": {"description": "d", "models": ["pi-opencode-go:tier-go-cheap"], "tier": "tier-go-cheap", "key_models": ["go"]},
  "tier-strong-open": {"description": "d", "models": ["pi-opencode-go:tier-strong-open"], "tier": "tier-strong-open", "key_models": ["go"]}
}"#;

fn defaults(extra_settings: &str) -> String {
    format!(
        r#"{{"version": 1,
  "clients": {{"pi-free": {{"binary": "pi", "cost_class": "subsidised", "model_source": "live"}},
    "pi-vibe": {{"binary": "pi", "cost_class": "subsidised", "model_source": "live"}},
    "pi-opencode-go": {{"binary": "pi", "cost_class": "subsidised", "model_source": "live"}}}},
  "models": [{{"client": "pi-free", "model": "tier-free", "family": "litellm-tier", "tier": "routine", "notes": "n"}},
    {{"client": "pi-vibe", "model": "tier-vibe", "family": "litellm-tier", "tier": "routine", "notes": "n"}},
    {{"client": "pi-opencode-go", "model": "tier-go-cheap", "family": "litellm-tier", "tier": "routine", "notes": "n"}},
    {{"client": "pi-opencode-go", "model": "tier-strong-open", "family": "litellm-tier", "tier": "routine", "notes": "n"}}],
  "profiles": {TIERS},
  "settings": {{"availability_window_minutes": 60, "max_batch_delegations": 50, "max_parallel_per_client": 3,
    "metered_personal_caps": {{"per_day_usd": 5, "per_workflow_usd": 2}},
    "output_allowance_tokens": {{"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500}},
    "plan_warning_ratio": 0.8, "summary_max_chars": 1500{extra_settings}}}
}}"#
    )
}

fn pi_home(machine: &Machine, models: &str) {
    machine.write_at(&format!("{}/home/.pi/agent/models.json", machine.sandbox.text()), models);
}

fn all_providers() -> String {
    let provider = |name: &str, key: &str| format!(r#""litellm-{name}": {{"api": "openai-completions", "apiKey": "{key}", "baseUrl": "https://litellm.example/v1", "models": []}}"#);
    format!(r#"{{"providers": {{{}, {}, {}}}}}"#, provider("free", "sk-free"), provider("vibe", "sk-vibe"), provider("opencode-go", "sk-go"))
}

/// A machine whose LiteLLM says every key has no budget, so only the ledger can put a tier out of quota.
fn machine(extra_settings: &str) -> Machine {
    let process = FakeProcess::default();
    process.install("curl");
    process.answer("curl", Ok(ProcessResult { stdout: r#"{"info":{"spend":0}}"#.into(), stderr: String::new(), exit_code: 0, timed_out: false }));
    let machine = Machine::with_process(&defaults(extra_settings), now_ms(), process.clone(), Box::new(process));
    pi_home(&machine, &all_providers());
    machine
}

fn ledger_run(at_ms: i64, extra: &str) -> String {
    format!(r#"{{"at":"{}","client":"pi-opencode-go","cost_usd":0,"model":"m","outcome":"ok","run_id":"r","workflow":"w"{extra}}}"#, timestamps::to_iso(at_ms))
}

fn go_ledger(machine: &Machine, five_hour_usd: f64) {
    machine.write_ledger(&[ledger_run(now_ms() - HOUR, &format!(r#","allowance_usd":{five_hour_usd},"litellm_model_id":"opencode-go/mimo-v2.6-flash""#)), "not json".to_string()]);
}

fn chosen_json(machine: &Machine, tier: &str) -> (String, Value) {
    let input = parse(&format!(r#"{{"tier":"{tier}","prompt":"p"}}"#)).expect("JSON");
    let chosen = crate::tier::choose(input.as_object().expect("object"), &machine.runtime).expect("a tier");
    (chosen.input.get("profile").and_then(Value::as_str).expect("a profile").to_string(), chosen.decision.expect("a decision"))
}

fn reason_of(decision: &Value) -> String {
    decision.as_object().and_then(|object| object.get("reason")).and_then(Value::as_str).expect("a reason").to_string()
}

#[test]
fn a_tier_run_drops_a_tier_when_the_ledger_says_go_is_at_80_percent_of_its_five_hour_window() {
    let hot = machine("");
    go_ledger(&hot, 9.6);
    let (profile, decision) = chosen_json(&hot, "tier-strong-open");
    assert_eq!(profile, "tier-vibe");
    let object = decision.as_object().expect("an object");
    assert_eq!(object.get("requested").and_then(Value::as_str), Some("tier-strong-open"));
    assert_eq!(object.get("chosen").and_then(Value::as_str), Some("tier-vibe"));
    assert!(reason_of(&decision).contains("opencode-go 5h at 80% of 12"), "{decision:?}");
    let cool = machine("");
    go_ledger(&cool, 9.5);
    assert_eq!(chosen_json(&cool, "tier-strong-open").0, "tier-strong-open");
    // The threshold is the machine's to move.
    let strict = machine(r#", "quota": {"threshold": 0.5}"#);
    go_ledger(&strict, 6.0);
    assert_eq!(chosen_json(&strict, "tier-go-cheap").0, "tier-vibe");
}

#[test]
fn a_tier_the_defaults_do_not_state_or_whose_key_the_machine_lacks_is_not_dropped_to() {
    let without_vibe = defaults("").replace(r#"  "tier-vibe": {"description": "d", "models": ["pi-vibe:tier-vibe"], "tier": "tier-vibe", "key_models": ["vibe"]},"#, "");
    let sparse = Machine::new(&without_vibe);
    pi_home(&sparse, &all_providers());
    go_ledger(&sparse, 12.0);
    assert_eq!(chosen_json(&sparse, "tier-strong-open").0, "tier-free");
    let keyless = Machine::new(&defaults(""));
    let provider = |name: &str| format!(r#""litellm-{name}": {{"apiKey": "k", "baseUrl": "https://x/v1"}}"#);
    pi_home(&keyless, &format!(r#"{{"providers": {{{}, {}}}}}"#, provider("free"), provider("opencode-go")));
    go_ledger(&keyless, 12.0);
    assert_eq!(chosen_json(&keyless, "tier-strong-open").0, "tier-free");
}

fn answer_key_info(process: &FakeProcess, body: &str, exit_code: i32) {
    process.answer("curl", Ok(ProcessResult { stdout: body.to_string(), stderr: String::new(), exit_code, timed_out: false }));
}

#[test]
fn with_no_allowance_in_the_ledger_a_tier_run_reads_the_keys_own_cap_from_litellm() {
    let process = FakeProcess::default();
    process.install("curl");
    let machine = Machine::with_process(&defaults(""), now_ms(), process.clone(), Box::new(process.clone()));
    pi_home(&machine, &all_providers());
    answer_key_info(&process, r#"{"key":"x","info":{"spend":8.5,"max_budget":10.0}}"#, 0);
    let (profile, decision) = chosen_json(&machine, "tier-strong-open");
    // The stub answers for every key alike, so the key of tier-vibe is at 85% of its cap too.
    assert_eq!(profile, "tier-free", "the key is at 85% of its cap");
    assert!(reason_of(&decision).contains("tier-strong-open out of quota (key): key key at 85% of 10"), "{}", reason_of(&decision));
    // A key with no budget says nothing, so the tier is as asked.
    answer_key_info(&process, r#"{"info":{"spend":99}}"#, 0);
    let (profile, decision) = chosen_json(&machine, "tier-strong-open");
    assert_eq!((profile.as_str(), reason_of(&decision).as_str()), ("tier-strong-open", "no-signal"));
    // A key that could not be asked is not a key with no budget: the plan cannot be checked, so the tier drops, and the
    // decision says why.
    for (what, answer, code) in [("failed", r#"{"info":{"spend":99,"max_budget":1}}"#, 22), ("not JSON", "<html>", 0), ("no info", r#"{"error":"x"}"#, 0), ("no spend", r#"{"info":{"max_budget":1}}"#, 0)] {
        answer_key_info(&process, answer, code);
        let (profile, decision) = chosen_json(&machine, "tier-strong-open");
        assert_eq!(profile, "tier-free", "{what}");
        assert!(reason_of(&decision).contains("tier-strong-open out of quota (key-error): key (LiteLLM"), "{what}: {}", reason_of(&decision));
        assert!(json::compact(&decision).contains(r#""source":"key-error""#), "{what}");
    }
    // A curl that cannot start is the same.
    process.answer("curl", Err("no curl".to_string()));
    assert_eq!(chosen_json(&machine, "tier-strong-open").0, "tier-free");
    // With a ledger signal under 80% the key is not asked, so a proxy that is down does not move the tier.
    go_ledger(&machine, 1.0);
    assert_eq!(chosen_json(&machine, "tier-strong-open").0, "tier-strong-open");
}

#[test]
fn a_key_that_cannot_be_read_or_holds_a_control_byte_is_never_sent_to_curl() {
    let process = FakeProcess::default();
    process.install("curl");
    answer_key_info(&process, r#"{"info":{"spend":9,"max_budget":10}}"#, 0);
    let machine = Machine::with_process(&defaults(""), now_ms(), process.clone(), Box::new(process.clone()));
    let key = |provider: &str| match quota::key_info(&machine.runtime, provider) {
        KeyOutcome::Info(info) => Some(info),
        KeyOutcome::Failed(_) => None,
    };
    assert_eq!(key("litellm-go"), None, "no models.json");
    pi_home(&machine, "{");
    assert_eq!(key("litellm-go"), None, "not JSON");
    pi_home(&machine, r#"{"providers": {"litellm-go": {"apiKey": "sk-a\nb", "baseUrl": "https://x/v1"}, "litellm-ok": {"apiKey": "sk-ok", "baseUrl": "https://x/v1"}, "litellm-nobase": {"apiKey": "k"}, "litellm-cmd": {"apiKey": "!rm -rf", "baseUrl": "https://x/v1"}}}"#);
    assert_eq!(key("litellm-go"), None, "a key with a control byte");
    assert_eq!(key("litellm-nobase"), None);
    assert_eq!(key("litellm-cmd"), None, "a key command the gateway does not run");
    assert_eq!(key("litellm-ok"), Some(KeyInfo { spend: 9.0, max_budget: Some(10.0) }));
    // Shapes of models.json that hold no key: no providers, a provider that is no object, a base URL or key that is no string.
    for (what, models) in [
        ("a list", r#"[]"#),
        ("no providers", r#"{}"#),
        ("a provider that is no object", r#"{"providers": {"litellm-x": 3}}"#),
        ("a base URL that is no string", r#"{"providers": {"litellm-x": {"baseUrl": 3, "apiKey": "k"}}}"#),
        ("a key that is no string", r#"{"providers": {"litellm-x": {"baseUrl": "https://x/v1", "apiKey": 3}}}"#),
    ] {
        pi_home(&machine, models);
        assert_eq!(key("litellm-x"), None, "{what}");
    }
    // Answers that hold no key's info: a list, or an info that is no object.
    pi_home(&machine, r#"{"providers": {"litellm-x": {"baseUrl": "https://x/v1", "apiKey": "k"}}}"#);
    for answer in ["[]", r#"{"info": 3}"#] {
        answer_key_info(&process, answer, 0);
        assert_eq!(key("litellm-x"), None, "{answer}");
    }
}

#[test]
fn the_draws_are_the_runs_of_the_ledger_that_recorded_one() {
    let machine = machine("");
    machine.write_ledger(&[ledger_run(now_ms() - HOUR, r#","allowance_usd":2.5,"litellm_cost_usd":0.5,"litellm_model_id":"vibe-study/deepseek""#)]);
    assert_eq!(quota::draws(&machine.runtime, now_ms()), [Draw { at_ms: now_ms() - HOUR, allowance_usd: Some(2.5), cost_usd: Some(0.5), account: Some("vibe-study".into()) }]);
    // Reservations, settlements, runs without a draw or with a time that is none are not draws.
    machine.write_ledger(&[
        r#"{"kind":"settled","run_id":"r","workflow":"w","client":"c","cost_class":"free","cost_usd":1,"reserved_at":"2026-11-04T09:00:00.000Z","settled_at":"2026-11-04T09:01:00.000Z"}"#.to_string(),
        ledger_run(now_ms(), ""),
        r#"{"at":"soon","allowance_usd":1}"#.to_string(),
        r#"{"at":"2026-11-04T09:00:00.000Z","allowance_usd":1,"litellm_model_id":"nobody"}"#.to_string(),
    ]);
    assert_eq!(quota::draws(&machine.runtime, now_ms()).iter().map(|draw| draw.account.clone()).collect::<Vec<_>>(), [None]);
    assert_eq!(quota::draws(&Machine::new(&defaults("")).runtime, now_ms()), []);
}

// ---- the settings ----

fn quota_of(text: &str) -> Result<QuotaSettings, String> {
    let raw = parse(text).expect("JSON");
    merge(&settings(), Some(&raw)).map_err(|error| error.message)
}

#[test]
fn the_defaults_state_a_limit_per_tier_and_the_settings_override_them() {
    let defaults = settings();
    let limits: Vec<usize> = ["tier-free", "tier-vibe", "tier-go-cheap", "tier-strong-open", "tier-review", "other"].iter().map(|tier| defaults.parallel_of(tier)).collect();
    assert_eq!(limits, [4, 3, 2, 1, 3, 2]);
    assert_eq!((defaults.threshold, defaults.go_five_hour_usd, defaults.go_weekly_usd), (0.8, 12.0, 30.0));
    assert_eq!((defaults.accounts["vibe"], defaults.accounts["opencode-go"]), (255.0, 40.0));
    assert_eq!(merge(&defaults, None), Ok(defaults.clone()));
    let tuned = quota_of(r#"{"threshold": 0.9, "go_five_hour_usd": 20, "go_weekly_usd": 50, "accounts": {"opencode-go": 60}, "parallel": {"tier-vibe": 5, "tier-new": 1}, "default_parallel": 7, "slot_wait_s": 5}"#).expect("valid");
    assert_eq!((tuned.threshold, tuned.go_five_hour_usd, tuned.go_weekly_usd, tuned.accounts["opencode-go"], tuned.accounts["vibe"]), (0.9, 20.0, 50.0, 60.0, 255.0));
    assert_eq!([tuned.parallel_of("tier-vibe"), tuned.parallel_of("tier-new"), tuned.parallel_of("tier-free"), tuned.parallel_of("x")], [5, 1, 4, 7]);
    assert_eq!(tuned.slot_wait_s, 5.0);
}

#[test]
fn a_setting_that_is_not_one_is_refused_by_name() {
    for (text, said) in [
        (r#"3"#, "settings.quota must be an object"),
        (r#"{"limit": 1}"#, "settings.quota.limit is not a setting: it has threshold,"),
        (r#"{"threshold": 0}"#, "settings.quota.threshold must be a number above 0"),
        (r#"{"threshold": "x"}"#, "settings.quota.threshold must be a number above 0"),
        (r#"{"threshold": 1.5}"#, "settings.quota.threshold must be a share of a window"),
        (r#"{"go_weekly_usd": -1}"#, "settings.quota.go_weekly_usd must be a number above 0"),
        (r#"{"accounts": 3}"#, "settings.quota.accounts must be an object"),
        (r#"{"accounts": {"vibe": 0}}"#, "settings.quota.accounts.vibe must be a number above 0"),
        (r#"{"parallel": {"tier-vibe": 0}}"#, "settings.quota.parallel.tier-vibe must be a whole number from 1 to 1024"),
        (r#"{"parallel": {"tier-vibe": 1.5}}"#, "settings.quota.parallel.tier-vibe must be a whole number from 1 to 1024"),
        (r#"{"parallel": 3}"#, "settings.quota.parallel must be an object"),
        (r#"{"go_five_hour_usd": 0}"#, "settings.quota.go_five_hour_usd must be a number above 0"),
        (r#"{"slot_wait_s": -1}"#, "settings.quota.slot_wait_s must be a number of seconds from 0 to 86400"),
        (r#"{"slot_wait_s": 1e300}"#, "settings.quota.slot_wait_s must be a number of seconds from 0 to 86400"),
        (r#"{"slot_wait_s": "x"}"#, "settings.quota.slot_wait_s must be a number of seconds from 0 to 86400"),
        (r#"{"parallel": {"tier-vibe": 100000}}"#, "settings.quota.parallel.tier-vibe must be a whole number from 1 to 1024"),
        (r#"{"slot_stale_s": 5}"#, "settings.quota.slot_stale_s is not a setting"),
        (r#"{"default_parallel": 0}"#, "settings.quota.default_parallel must be a whole number from 1 to 1024"),
    ] {
        let refused = quota_of(text).expect_err(text);
        assert!(refused.starts_with(said), "{text}: {refused}");
    }
}

#[test]
fn the_shipped_defaults_and_a_machine_file_both_carry_the_settings() {
    let config = parse_defaults(&parse(&defaults(r#", "quota": {"default_parallel": 6}"#)).expect("JSON")).expect("defaults");
    assert_eq!(config.settings.quota.default_parallel, 6);
    let plain: Settings = parse_defaults(&parse(&defaults("")).expect("JSON")).expect("defaults").settings;
    assert_eq!(plain.quota, QuotaSettings::default());
    let refused = parse_defaults(&parse(&defaults(r#", "quota": {"nope": 1}"#)).expect("JSON")).expect_err("refused");
    assert_eq!(refused.code, "invalid-config");
    // A machine file moves a setting over the defaults.
    let machine = Machine::new(&defaults(""));
    machine.write_override(r#"{"settings": {"quota": {"threshold": 0.6}}}"#);
    let loaded = crate::config_parse::load_config(&machine.runtime).expect("config");
    assert_eq!(loaded.settings.quota.threshold, 0.6);
}

#[test]
fn only_the_last_32_days_of_a_large_ledger_are_read() {
    let machine = machine("");
    // 4000 old runs (about 900 KB, over three chunks), then 1500 recent ones (over one chunk by themselves).
    let old = now_ms() - 100 * DAY;
    let mut lines: Vec<String> = (0..4000).map(|at| ledger_run(old + at, r#","allowance_usd":1,"litellm_model_id":"opencode-go/old""#)).collect();
    lines.push(r#"{"kind":"settled","run_id":"r","workflow":"w","client":"c","cost_class":"free","cost_usd":1,"reserved_at":"2026-08-04T09:00:00.000Z","settled_at":"2026-08-04T09:01:00.000Z"}"#.to_string());
    lines.extend((0..1500).map(|at| ledger_run(now_ms() - HOUR - at, r#","allowance_usd":0.01,"litellm_model_id":"opencode-go/new""#)));
    machine.write_ledger(&lines);
    let drawn = quota::draws(&machine.runtime, now_ms());
    assert_eq!(drawn.len(), 1500);
    assert!(drawn.iter().all(|draw| draw.at_ms >= now_ms() - 32 * DAY && draw.allowance_usd == Some(0.01)));
    // A ledger that is all recent is read whole, one that is not there or cannot be read gives nothing.
    machine.write_ledger(&lines[4001..4010]);
    assert_eq!(quota::draws(&machine.runtime, now_ms()).len(), 9);
    std::fs::remove_file(machine.ledger_file()).expect("remove");
    assert_eq!(quota::draws(&machine.runtime, now_ms()), []);
    std::fs::create_dir_all(machine.ledger_file()).expect("a directory where the ledger would be");
    assert_eq!(quota::draws(&machine.runtime, now_ms()), []);
}

#[test]
fn a_tier_run_that_finds_its_tiers_slots_all_taken_is_refused_before_anything_launches() {
    let machine = machine(r#", "quota": {"parallel": {"tier-free": 1}, "slot_wait_s": 0}"#);
    let argv: Vec<String> = ["run", "--tier", "tier-free", "--json", r#"{"workflow":"w","mode":"read-only","prompt":"p","escalate":false}"#].iter().map(|arg| arg.to_string()).collect();
    let held = crate::tier_slots::acquire(&machine.runtime, &crate::tier_slots::SlotPlan { tier: "tier-free".into(), limit: 1, wait_s: 0.0 }).expect("the slot");
    let refused = crate::cli::run_cli(&argv, &machine.runtime);
    assert_ne!(refused.exit_code, 0, "{}{}", refused.stdout, refused.stderr);
    assert!(refused.stdout.contains("tier-busy") || refused.stderr.contains("tier-busy"), "{}{}", refused.stdout, refused.stderr);
    assert_eq!(machine.ledger_text(), "", "nothing ran");
    drop(held);
    // With the slot free the run gets past the slot (and fails later for want of a Pi that can launch here).
    let next = crate::cli::run_cli(&argv, &machine.runtime);
    assert!(!(next.stdout.contains("tier-busy") || next.stderr.contains("tier-busy")), "{}{}", next.stdout, next.stderr);
}
