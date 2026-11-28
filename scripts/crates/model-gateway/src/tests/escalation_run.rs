//! Tests for the attempt loop of a delegation: the retry, the climb, the swap of
//! a client that cannot start and the results a delegation ends on. The runs are
//! scripted, so nothing is launched.

use super::machine::{sleeping, Machine, DEFAULTS};
use crate::error::GatewayError;
use crate::escalation::{AttemptLink, EscalationEnd, UnavailableResult};
use crate::free_fallback::FallbackReport;
use crate::retry_wait::RetryReport;
use crate::escalation_run::{AttemptError, AttemptedRun, Delegation, Escalated, EscalatedRun};
use crate::json::parse;
use crate::ports::ProcessResult;
use crate::run::{read_run_input, RunInput};
use std::collections::VecDeque;

/// One finished attempt, as much of a result as the escalation reads.
#[derive(Clone, Debug, PartialEq)]
struct FakeRun {
    number: usize,
    root: String,
    client: String,
    model: String,
    ok: bool,
    reason: Option<String>,
    refusal: Option<String>,
}

impl AttemptedRun for FakeRun {
    fn is_ok(&self) -> bool {
        self.ok
    }
    fn run_id(&self) -> &str {
        Box::leak(format!("run-{}", self.number).into_boxed_str())
    }
    fn client(&self) -> &str {
        &self.client
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn family(&self) -> Option<&str> {
        match self.model.as_str() {
            model if model.starts_with("claude") => Some("anthropic"),
            model if model.starts_with("zai") => Some("zai"),
            _ => None,
        }
    }
    fn summary(&self) -> Option<&str> {
        self.family().map(|_| "done")
    }
    fn set_summary(&mut self, summary: String) {
        self.model = format!("{}|{summary}", self.model);
    }
    fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
    fn output_path(&self) -> &str {
        Box::leak(format!("{}/runs/{}/output.txt", self.root, self.number).into_boxed_str())
    }
    fn check_output_path(&self) -> Option<&str> {
        Some(Box::leak(format!("{}/runs/{}/check.txt", self.root, self.number).into_boxed_str()))
    }
    fn free_refusal(&self) -> Option<String> {
        self.refusal.clone()
    }
}

/// What an attempt of a script comes to.
enum Step {
    Fail(&'static str),
    /// A failed run that names no reason.
    FailSilently,
    /// A failed run whose client said no free copy of its model is available.
    Refused(&'static str),
    Pass,
    Error(AttemptError),
}

/// What one attempt was launched with.
struct Seen {
    input: RunInput,
    link: Option<AttemptLink>,
    had_check: bool,
    fallback: Option<FallbackReport>,
    retry: Option<RetryReport>,
}

fn input(extra: &str) -> RunInput {
    let text = format!(r#"{{"client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","workflow":"w"{extra}}}"#);
    read_run_input(&parse(&text).expect("JSON")).expect("a run input")
}

fn drive(machine: &Machine, run: &RunInput, steps: Vec<Step>) -> (Result<Delegation<FakeRun>, GatewayError>, Vec<Seen>) {
    let mut steps: VecDeque<Step> = steps.into();
    let mut seen = Vec::new();
    let root = machine.sandbox.text();
    let result = (EscalatedRun { runtime: &machine.runtime }).run(run, &mut |attempt, options| {
        seen.push(Seen { input: attempt.clone(), link: options.link.clone(), had_check: options.start_check.is_some(), fallback: options.fallback.clone(), retry: options.retry.clone() });
        let number = seen.len();
        let mut refusal = None;
        let (ok, reason) = match steps.pop_front().expect("a scripted step") {
            Step::Fail(reason) => (false, Some(reason.to_string())),
            Step::FailSilently => (false, None),
            Step::Refused(text) => {
                refusal = Some(text.to_string());
                (false, Some("rate-limited".to_string()))
            }
            Step::Pass => (true, None),
            Step::Error(error) => return Err(error),
        };
        Ok(FakeRun { number, root: root.clone(), client: attempt.client.clone(), model: attempt.model.clone(), ok, reason, refusal })
    });
    (result, seen)
}

fn ran(result: Result<Delegation<FakeRun>, GatewayError>) -> Escalated<FakeRun> {
    match result.expect("the delegation ends") {
        Delegation::Run(escalated) => escalated,
        Delegation::Unavailable(unavailable) => panic!("unavailable: {unavailable:?}"),
    }
}

fn unavailable(result: Result<Delegation<FakeRun>, GatewayError>) -> UnavailableResult {
    match result.expect("the delegation ends") {
        Delegation::Unavailable(unavailable) => unavailable,
        Delegation::Run(escalated) => panic!("ran: {escalated:?}"),
    }
}

fn models(seen: &[Seen]) -> Vec<&str> {
    seen.iter().map(|attempt| attempt.input.model.as_str()).collect()
}

fn statuses(escalated: &Escalated<FakeRun>) -> Vec<(bool, Option<&str>)> {
    escalated.report.attempts.iter().map(|attempt| (attempt.ok, attempt.reason.as_deref())).collect()
}

#[test]
fn a_delegation_that_may_not_escalate_runs_once_and_lists_its_one_attempt() {
    let machine = Machine::new(DEFAULTS);
    let (result, seen) = drive(&machine, &input(r#","escalate":false"#), vec![Step::Fail("exit-1")]);
    let escalated = ran(result);
    assert_eq!(models(&seen), ["claude-sonnet-5-5"]);
    assert!(seen[0].link.is_none() && !seen[0].had_check);
    assert_eq!(statuses(&escalated), [(false, Some("exit-1"))]);
    assert_eq!(escalated.report.escalation, None);
}

#[test]
fn a_delegation_that_may_not_escalate_ends_with_the_error_its_attempt_ended_with() {
    let machine = Machine::new(DEFAULTS);
    let refused = GatewayError::new("policy-refused", "no");
    let (result, _) = drive(&machine, &input(r#","escalate":false"#), vec![Step::Error(AttemptError::Refused(refused.clone()))]);
    assert_eq!(result.expect_err("refused"), refused);
    let (result, _) = drive(&machine, &input(r#","escalate":false"#), vec![Step::Error(AttemptError::CannotStart("signed out".to_string()))]);
    assert_eq!(result.expect_err("refused"), GatewayError::new("unexpected", "signed out"));
}

#[test]
fn a_result_whose_reason_is_a_refusal_comes_back_as_it_is_after_one_attempt() {
    let machine = Machine::new(DEFAULTS);
    let (result, seen) = drive(&machine, &input(""), vec![Step::Fail("cap-exceeded")]);
    let escalated = ran(result);
    assert_eq!(seen.iter().map(|attempt| attempt.input.prompt.as_str()).collect::<Vec<_>>(), ["p"]);
    assert_eq!(statuses(&escalated), [(false, Some("cap-exceeded"))]);
    assert!(seen[0].had_check);
    assert_eq!(escalated.report.escalation, None);
}

#[test]
fn a_clean_run_comes_back_with_its_summary_untouched_when_its_own_family_answered() {
    let machine = Machine::new(DEFAULTS);
    let (result, seen) = drive(&machine, &input(""), vec![Step::Pass]);
    let escalated = ran(result);
    assert_eq!(seen.len(), 1);
    assert_eq!(escalated.run.model, "claude-sonnet-5-5");
    assert_eq!(statuses(&escalated), [(true, None)]);
}

#[test]
fn a_clean_run_that_names_no_family_is_returned_without_reading_the_catalogue() {
    let machine = Machine::new("this is not json");
    let (result, _) = drive(&machine, &input(r#","model":"mystery""#), vec![Step::Pass]);
    assert_eq!(ran(result).run.model, "mystery");
}

#[test]
fn a_failed_run_is_retried_once_on_the_same_model_with_the_failure_attached() {
    let machine = Machine::new(DEFAULTS);
    machine.write_at(&format!("{}/runs/1/stderr.txt", machine.sandbox.text()), "boom\n");
    let (result, seen) = drive(&machine, &input(""), vec![Step::Fail("exit-1"), Step::Pass]);
    let escalated = ran(result);
    assert_eq!(models(&seen), ["claude-sonnet-5-5", "claude-sonnet-5-5"]);
    assert_eq!(seen[1].input.prompt, "p\n\nA previous attempt failed: exit-1; boom");
    assert_eq!(seen[1].link, Some(AttemptLink { attempt: 2, attempt_of: Some("run-1".to_string()) }));
    assert_eq!(statuses(&escalated), [(false, Some("exit-1")), (true, None)]);
    assert_eq!(escalated.report.escalation, None);
}

#[test]
fn what_a_failed_attempt_tells_the_next_one_comes_from_its_check_then_stderr_then_its_output() {
    let machine = Machine::new(DEFAULTS);
    let root = machine.sandbox.text();
    machine.write_at(&format!("{root}/runs/1/check.txt"), "lint failed");
    machine.write_at(&format!("{root}/runs/1/stderr.txt"), "stderr says");
    let (_, seen) = drive(&machine, &input(""), vec![Step::Fail("check-failed"), Step::Pass]);
    assert_eq!(seen[1].input.prompt, "p\n\nA previous attempt failed: check-failed; lint failed");
    // A check that printed nothing leaves stderr, and a stderr that printed nothing leaves the output.
    machine.write_at(&format!("{root}/runs/1/check.txt"), "  \n");
    let (_, seen) = drive(&machine, &input(""), vec![Step::Fail("check-failed"), Step::Pass]);
    assert_eq!(seen[1].input.prompt, "p\n\nA previous attempt failed: check-failed; stderr says");
    machine.write_at(&format!("{root}/runs/1/stderr.txt"), "\n");
    machine.write_at(&format!("{root}/runs/1/output.txt"), "the answer");
    let (_, seen) = drive(&machine, &input(""), vec![Step::Fail("exit-1"), Step::Pass]);
    assert_eq!(seen[1].input.prompt, "p\n\nA previous attempt failed: exit-1; the answer");
    // Whatever cannot be read says nothing.
    std::fs::remove_file(format!("{root}/runs/1/output.txt")).expect("remove");
    let (_, seen) = drive(&machine, &input(""), vec![Step::Fail("timeout"), Step::Pass]);
    assert_eq!(seen[1].input.prompt, "p\n\nA previous attempt failed: timeout");
}

#[test]
fn a_second_failure_climbs_to_the_next_stronger_model_and_a_third_ends_the_delegation() {
    let machine = Machine::new(DEFAULTS);
    let (result, seen) = drive(&machine, &input(""), vec![Step::Fail("exit-1"), Step::Fail("exit-1"), Step::Pass]);
    let escalated = ran(result);
    assert_eq!(models(&seen), ["claude-sonnet-5-5", "claude-sonnet-5-5", "claude-opus-5"]);
    assert_eq!(seen[2].link, Some(AttemptLink { attempt: 3, attempt_of: Some("run-1".to_string()) }));
    assert_eq!(escalated.run.model, "claude-opus-5");
    let (result, seen) = drive(&machine, &input(""), vec![Step::Fail("exit-1"), Step::Fail("exit-1"), Step::Fail("timeout")]);
    let escalated = ran(result);
    assert_eq!(seen.len(), 3);
    assert_eq!(escalated.report.escalation, Some(EscalationEnd::Exhausted));
    assert_eq!(escalated.run.reason.as_deref(), Some("timeout"));
}

#[test]
fn a_delegation_with_one_attempt_ends_on_its_first_failure() {
    let machine = Machine::new(DEFAULTS);
    let (result, seen) = drive(&machine, &input(r#","max_attempts":1"#), vec![Step::Fail("exit-1")]);
    assert_eq!(seen.len(), 1);
    assert_eq!(ran(result).report.escalation, Some(EscalationEnd::Exhausted));
}

#[test]
fn a_delegation_on_the_strongest_model_has_nowhere_to_climb_and_ends_exhausted_at_once() {
    let machine = Machine::new(DEFAULTS);
    let (result, seen) = drive(&machine, &input(r#","model":"claude-opus-5""#), vec![Step::Fail("exit-1"), Step::Fail("exit-1")]);
    assert_eq!(seen.len(), 2);
    let escalated = ran(result);
    assert_eq!(escalated.report.escalation, Some(EscalationEnd::Exhausted));
    assert_eq!(statuses(&escalated).len(), 2);
}

#[test]
fn a_client_that_cannot_start_is_swapped_for_another_model_of_its_cost_class_that_covers_the_tier() {
    let machine = Machine::new(DEFAULTS);
    machine.write_override(r#"{"settings":{"summary_max_chars":40}}"#);
    let (result, seen) = drive(&machine, &input(r#","family_binding":false"#), vec![Step::Fail("rate-limited"), Step::Pass]);
    let escalated = ran(result);
    assert_eq!(models(&seen), ["claude-sonnet-5-5", "zai-glm-5-3"]);
    assert_eq!(seen[1].input.client, "vibe");
    // The retry prompt does not enter: a client that could not start was not a failed attempt.
    assert_eq!(seen[1].input.prompt, "p");
    assert_eq!(seen[1].link, Some(AttemptLink { attempt: 2, attempt_of: Some("run-1".to_string()) }));
    // The answer of another family opens with the escalation, cut to the machine's limit of 40 characters.
    assert_eq!(escalated.run.model, "zai-glm-5-3|escalated from anthropic (claude: rate-l");
    assert_eq!(escalated.report.skipped.len(), 1);
    assert_eq!(
        (escalated.report.skipped[0].client.as_str(), escalated.report.skipped[0].model.as_str(), escalated.report.skipped[0].reason.as_str()),
        ("claude", "claude-sonnet-5-5", "rate-limited")
    );
    assert_eq!(statuses(&escalated), [(false, Some("rate-limited")), (true, None)]);
}

#[test]
fn an_answer_of_another_family_after_a_climb_names_the_first_attempts_failure_as_the_cause() {
    let machine = Machine::new(DEFAULTS);
    let loose = input(r#","client":"vibe","model":"zai-glm-5-3","family_binding":false"#);
    let (result, seen) = drive(&machine, &loose, vec![Step::Fail("exit-1"), Step::Fail("exit-1"), Step::Pass]);
    assert_eq!(models(&seen), ["zai-glm-5-3", "zai-glm-5-3", "claude-opus-5"]);
    assert_eq!(ran(result).run.model, "claude-opus-5|escalated from zai (vibe: exit-1) to anthropic\ndone");
    // A failed run that named no reason is a plain failure.
    let (result, _) = drive(&machine, &loose, vec![Step::FailSilently, Step::FailSilently, Step::Pass]);
    assert_eq!(ran(result).run.model, "claude-opus-5|escalated from zai (vibe: failed) to anthropic\ndone");
    let (_, seen) = drive(&machine, &loose, vec![Step::FailSilently, Step::Pass]);
    assert_eq!(seen[1].input.prompt, "p\n\nA previous attempt failed: failed");
}

#[test]
fn a_family_binding_keeps_the_swap_inside_the_family() {
    let machine = Machine::new(DEFAULTS);
    let (result, seen) = drive(&machine, &input(""), vec![Step::Fail("auth"), Step::Pass]);
    assert_eq!(models(&seen), ["claude-sonnet-5-5", "claude-opus-5"]);
    assert_eq!(ran(result).run.model, "claude-opus-5");
}

#[test]
fn a_client_that_cannot_start_before_it_runs_is_swapped_too() {
    let machine = Machine::new(DEFAULTS);
    let reason = "unavailable: not installed: claude is not on PATH".to_string();
    let (result, seen) = drive(&machine, &input(""), vec![Step::Error(AttemptError::CannotStart(reason.clone())), Step::Pass]);
    let escalated = ran(result);
    assert_eq!(models(&seen), ["claude-sonnet-5-5", "claude-opus-5"]);
    // No attempt ran, so the swapped one is the first and nothing links to anything.
    assert!(seen[1].link.is_none());
    assert_eq!(escalated.report.skipped[0].reason, reason);
    assert_eq!(statuses(&escalated), [(true, None)]);
}

#[test]
fn a_delegation_no_model_of_its_cost_class_can_start_is_unavailable() {
    let machine = Machine::new(DEFAULTS);
    let result = unavailable(drive(&machine, &input(r#","model":"claude-opus-5""#), vec![Step::Fail("rate-limited")]).0);
    assert_eq!(
        result.message,
        "claude:claude-opus-5 cannot start, and no other permitted subsidised client model covers its tier; the gateway never moves into a costlier cost class by itself; the family anthropic is binding, so no other family answers"
    );
    assert_eq!((result.client.as_str(), result.model.as_str(), result.workflow.as_str()), ("claude", "claude-opus-5", "w"));
    assert_eq!(result.reasons.len(), 1);
    assert_eq!(result.attempts.len(), 1);
    let open = unavailable(drive(&machine, &input(r#","model":"claude-opus-5","family_binding":false"#), vec![Step::Fail("rate-limited")]).0);
    assert!(open.message.ends_with("never moves into a costlier cost class by itself"), "{}", open.message);
}

#[test]
fn a_model_the_catalogue_does_not_know_has_its_family_read_from_its_id_and_nothing_to_swap_to() {
    let machine = Machine::new(DEFAULTS);
    let result = unavailable(drive(&machine, &input(r#","model":"claude-brand-new-9""#), vec![Step::Error(AttemptError::CannotStart("signed out".to_string()))]).0);
    assert!(result.message.contains("the family anthropic is binding"), "{}", result.message);
    assert!(result.attempts.is_empty());
}

#[test]
fn a_swap_that_runs_out_after_a_failed_attempt_stops_the_delegation_with_what_it_could_not_start_with() {
    let machine = Machine::new(DEFAULTS);
    let (result, seen) = drive(&machine, &input(r#","model":"claude-opus-5""#), vec![Step::Fail("exit-1"), Step::Fail("auth")]);
    assert_eq!(seen.len(), 2);
    let escalated = ran(result);
    assert_eq!(escalated.report.escalation, Some(EscalationEnd::Stopped));
    assert_eq!(escalated.report.stopped_reason.as_deref(), Some("auth"));
    // The result is the last attempt that really ran.
    assert_eq!(escalated.run.number, 1);
    assert_eq!(statuses(&escalated), [(false, Some("exit-1")), (false, Some("auth"))]);
}

#[test]
fn a_climb_whose_target_cannot_start_ends_the_delegation_exhausted() {
    let machine = Machine::new(DEFAULTS);
    let (result, seen) = drive(&machine, &input(""), vec![Step::Fail("exit-1"), Step::Fail("timeout"), Step::Fail("rate-limited")]);
    assert_eq!(models(&seen), ["claude-sonnet-5-5", "claude-sonnet-5-5", "claude-opus-5"]);
    let escalated = ran(result);
    assert_eq!(escalated.report.escalation, Some(EscalationEnd::Exhausted));
    assert_eq!(escalated.run.number, 2);
    assert_eq!(escalated.report.stopped_reason, None);
}

#[test]
fn an_attempt_the_gateway_refuses_after_a_run_stops_the_delegation_naming_the_refusal() {
    let machine = Machine::new(DEFAULTS);
    let refusal = GatewayError::new("policy-refused", "not for this repository");
    let (result, _) = drive(&machine, &input(""), vec![Step::Fail("exit-1"), Step::Error(AttemptError::Refused(refusal))]);
    let escalated = ran(result);
    assert_eq!(escalated.report.escalation, Some(EscalationEnd::Stopped));
    assert_eq!(escalated.report.stopped_reason.as_deref(), Some("policy-refused: not for this repository"));
    assert_eq!(escalated.run.number, 1);
    let (result, _) = drive(&machine, &input(""), vec![Step::Fail("exit-1"), Step::Error(AttemptError::Other("disk full".to_string()))]);
    assert_eq!(ran(result).report.stopped_reason.as_deref(), Some("disk full"));
}

#[test]
fn an_attempt_that_fails_before_any_run_ends_the_delegation_with_its_error() {
    let machine = Machine::new(DEFAULTS);
    let refusal = GatewayError::new("policy-refused", "not for this repository");
    let (result, _) = drive(&machine, &input(""), vec![Step::Error(AttemptError::Refused(refusal.clone()))]);
    assert_eq!(result.expect_err("refused"), refusal);
    let (result, _) = drive(&machine, &input(""), vec![Step::Error(AttemptError::Other("disk full".to_string()))]);
    assert_eq!(result.expect_err("failed"), GatewayError::new("unexpected", "disk full"));
}

#[test]
fn a_delegation_that_cannot_read_its_catalogue_ends_with_the_error() {
    let machine = Machine::new("this is not json");
    let (result, _) = drive(&machine, &input(""), vec![Step::Fail("exit-1"), Step::Fail("exit-1")]);
    assert_eq!(result.expect_err("no catalogue").code, "invalid-config");
}

#[test]
fn the_attempt_can_ask_whether_its_client_can_start() {
    let machine = Machine::new(DEFAULTS);
    let mut checks: Vec<Option<String>> = Vec::new();
    let run = input("");
    let result = (EscalatedRun { runtime: &machine.runtime }).run(&run, &mut |attempt, options| {
        let check = options.start_check.expect("the escalation hands the check over");
        checks.push(check(attempt).expect("a reading"));
        machine.process.install("claude");
        checks.push(check(attempt).expect("a reading"));
        // A client the gateway does not know has no availability, so nothing stops it.
        checks.push(check(&input(r#","client":"agy","model":"m""#)).expect("a reading"));
        Ok::<FakeRun, AttemptError>(FakeRun {
            number: 1,
            root: String::new(),
            client: attempt.client.clone(),
            model: attempt.model.clone(),
            ok: true,
            reason: None,
            refusal: None,
        })
    });
    assert!(result.is_ok());
    assert_eq!(checks, [Some("unavailable: not installed: claude is not on PATH".to_string()), None, None]);
}

#[test]
fn the_check_fails_with_the_error_when_the_catalogue_cannot_be_read() {
    let machine = Machine::new("this is not json");
    let run = input(r#","model":"mystery""#);
    let result = (EscalatedRun { runtime: &machine.runtime }).run(&run, &mut |attempt, options| {
        let check = options.start_check.expect("the escalation hands the check over");
        assert_eq!(check(attempt).expect_err("no catalogue").code, "invalid-config");
        Ok::<FakeRun, AttemptError>(FakeRun { number: 1, root: String::new(), client: attempt.client.clone(), model: attempt.model.clone(), ok: true, reason: None, refusal: None })
    });
    assert!(result.is_ok());
}


// The free fallback: a run on a free LiteLLM model that finds no free copy is answered once by OpenCode's own
// free copy of the same model, whatever the escalation does.

const FREE_DEFAULTS: &str = r#"{
  "version": 1,
  "clients": {
    "pi-free": {"binary": "pi", "cost_class": "subsidised", "model_source": "declared", "data_scope": "public"},
    "opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "declared"}
  },
  "models": [
    {"client": "pi-free", "family": "deepseek", "model": "free/deepseek-v4.1-flash", "notes": "n", "tier": "routine"}
  ],
  "settings": {
    "availability_window_minutes": 60,
    "max_batch_delegations": 50,
    "max_parallel_per_client": 3,
    "metered_personal_caps": {"per_day_usd": 5, "per_workflow_usd": 2},
    "output_allowance_tokens": {"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500},
    "plan_warning_ratio": 0.8,
    "summary_max_chars": 1500
  }
}"#;

const NO_FREE_COPY: &str = "No deployments available for selected model, Try again in a moment: free limits [limiter refusal, no provider was called]: every free deployment of deepseek-v4.1-flash is limited; the first is back in 90s";

fn free_machine() -> Machine {
    let machine = Machine::new(FREE_DEFAULTS);
    machine.process.answer("opencode", Ok(ProcessResult { stdout: "opencode/deepseek-v4.1-flash-free\nopencode/other-free\n".to_string(), stderr: String::new(), exit_code: 0, timed_out: false }));
    machine
}

fn free_input(extra: &str) -> RunInput {
    let text = format!(r#"{{"client":"pi-free","mode":"read-only","model":"free/deepseek-v4.1-flash","prompt":"p","workflow":"w"{extra}}}"#);
    read_run_input(&parse(&text).expect("JSON")).expect("a run input")
}

#[test]
fn a_free_model_with_no_free_copy_is_answered_once_by_opencodes_own_free_copy_of_it() {
    let machine = free_machine();
    let (result, seen) = drive(&machine, &free_input(""), vec![Step::Refused(NO_FREE_COPY), Step::Pass]);
    let escalated = ran(result);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash", "opencode/deepseek-v4.1-flash-free"]);
    assert_eq!(seen[1].input.client, "opencode");
    assert!(seen[0].fallback.is_none());
    let fallback = seen[1].fallback.as_ref().expect("the fallback is handed to the attempt");
    assert_eq!(fallback.from, "pi-free free/deepseek-v4.1-flash");
    assert_eq!(fallback.to, "opencode opencode/deepseek-v4.1-flash-free");
    assert_eq!(fallback.reason, NO_FREE_COPY);
    assert_eq!(fallback.from_run_id, "run-1");
    assert_eq!(fallback.available_again_in_s, Some(90));
    assert_eq!(statuses(&escalated), [(true, None)]);
}

#[test]
fn the_fallback_does_not_depend_on_escalate_and_is_tried_only_once() {
    let machine = free_machine();
    let (result, seen) = drive(&machine, &free_input(r#","escalate":false"#), vec![Step::Refused(NO_FREE_COPY), Step::Refused(NO_FREE_COPY)]);
    let escalated = ran(result);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash", "opencode/deepseek-v4.1-flash-free"]);
    assert_eq!(statuses(&escalated), [(false, Some("rate-limited"))]);
}

#[test]
fn a_fallback_that_cannot_run_leaves_the_refused_run_as_the_answer() {
    let machine = free_machine();
    let refused = GatewayError::new("policy-refused", "no");
    let (result, seen) = drive(&machine, &free_input(r#","escalate":false"#), vec![Step::Refused(NO_FREE_COPY), Step::Error(AttemptError::Refused(refused))]);
    let escalated = ran(result);
    assert_eq!(seen.len(), 2);
    assert_eq!(statuses(&escalated), [(false, Some("rate-limited"))]);
}

#[test]
fn a_run_that_failed_any_other_way_falls_back_to_nothing() {
    let machine = free_machine();
    let (_, seen) = drive(&machine, &free_input(r#","escalate":false"#), vec![Step::Fail("exit-1")]);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash"]);
}

#[test]
fn a_run_that_named_its_client_or_turned_the_fallback_off_never_falls_back() {
    let machine = free_machine();
    for extra in [r#","escalate":false,"client_pinned":true"#, r#","escalate":false,"free_fallback":false"#] {
        let (_, seen) = drive(&machine, &free_input(extra), vec![Step::Refused(NO_FREE_COPY)]);
        assert_eq!(models(&seen), ["free/deepseek-v4.1-flash"]);
    }
}

#[test]
fn a_fallback_the_catalogue_or_the_policy_cannot_place_leaves_the_run_failed() {
    let machine = free_machine();
    // No Zen copy of the model: OpenCode's listing does not name it.
    machine.process.answer("opencode", Ok(ProcessResult { stdout: "opencode/other-free\n".to_string(), stderr: String::new(), exit_code: 0, timed_out: false }));
    let (_, seen) = drive(&machine, &free_input(r#","escalate":false"#), vec![Step::Refused(NO_FREE_COPY)]);
    assert_eq!(seen.len(), 1);
    // A configuration that cannot be read ends the plan quietly.
    let broken = Machine::new("this is not json");
    let (_, seen) = drive(&broken, &free_input(r#","escalate":false"#), vec![Step::Refused(NO_FREE_COPY)]);
    assert_eq!(seen.len(), 1);
}

fn waits(slept: &std::sync::Arc<std::sync::Mutex<Vec<u64>>>) -> Vec<u64> {
    slept.lock().expect("the sleeps").clone()
}

fn refused_for(seconds: u32) -> &'static str {
    Box::leak(format!("429: free limits [limiter refusal, no provider was called]: every free deployment of deepseek-v4.1-flash is limited; the first is back in {seconds}s").into_boxed_str())
}

#[test]
fn a_short_wait_the_proxy_names_is_waited_out_and_the_same_model_runs_again() {
    let (machine, slept) = sleeping(free_machine());
    let (result, seen) = drive(&machine, &free_input(""), vec![Step::Refused(refused_for(5)), Step::Pass]);
    let escalated = ran(result);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash", "free/deepseek-v4.1-flash"]);
    assert!(seen[0].retry.is_none() && seen[1].fallback.is_none());
    let retry = seen[1].retry.as_ref().expect("the retry is handed to the attempt");
    assert_eq!((retry.retry, retry.of_run_id.as_str(), retry.back_in_s, retry.waited_s, retry.total_waited_s), (1, "run-1", 5, 6.0, 6.0));
    assert_eq!(waits(&slept), [6000]);
    assert_eq!(statuses(&escalated), [(true, None)]);
}

#[test]
fn the_retries_are_bounded_and_the_free_fallback_takes_over_when_they_run_out() {
    let (machine, slept) = sleeping(free_machine());
    let steps = vec![Step::Refused(refused_for(5)), Step::Refused(refused_for(5)), Step::Refused(refused_for(5)), Step::Refused(refused_for(5)), Step::Pass];
    let (result, seen) = drive(&machine, &free_input(r#","escalate":false"#), steps);
    ran(result);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash", "free/deepseek-v4.1-flash", "free/deepseek-v4.1-flash", "free/deepseek-v4.1-flash", "opencode/deepseek-v4.1-flash-free"]);
    assert_eq!(waits(&slept), [6000, 6000, 6000]);
    assert_eq!(seen[3].retry.as_ref().map(|retry| (retry.retry, retry.total_waited_s)), Some((3, 18.0)));
    assert!(seen[4].retry.is_none() && seen[4].fallback.is_some());
}

#[test]
fn a_wait_that_does_not_fit_the_total_cap_is_not_taken_and_the_fallback_runs_at_once() {
    let (machine, slept) = sleeping(free_machine());
    // 41s fits the 60s cap once; the second 41s would make 82s.
    let (result, seen) = drive(&machine, &free_input(r#","escalate":false"#), vec![Step::Refused(refused_for(40)), Step::Refused(refused_for(40)), Step::Pass]);
    ran(result);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash", "free/deepseek-v4.1-flash", "opencode/deepseek-v4.1-flash-free"]);
    assert_eq!(waits(&slept), [41_000]);
    // A first wait that is already too long is never slept at all.
    let (machine, slept) = sleeping(free_machine());
    let (_, seen) = drive(&machine, &free_input(r#","escalate":false"#), vec![Step::Refused(NO_FREE_COPY), Step::Pass]);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash", "opencode/deepseek-v4.1-flash-free"]);
    assert!(waits(&slept).is_empty());
}

#[test]
fn other_refusals_and_other_failures_are_not_waited_for() {
    let (machine, slept) = sleeping(free_machine());
    // A 429 that names no time, a daily limit and a plain failure.
    for step in [Step::Refused("429: slow down"), Step::Refused("daily limit reached; resets at 00:00 UTC"), Step::Fail("exit-1")] {
        let (_, seen) = drive(&machine, &free_input(r#","escalate":false,"free_fallback":false"#), vec![step]);
        assert_eq!(seen.len(), 1);
    }
    assert!(waits(&slept).is_empty());
}

#[test]
fn a_run_that_named_its_client_is_waited_for_too_since_the_same_model_runs_again() {
    let (machine, slept) = sleeping(free_machine());
    let (_, seen) = drive(&machine, &free_input(r#","escalate":false,"client_pinned":true"#), vec![Step::Refused(refused_for(5)), Step::Pass]);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash", "free/deepseek-v4.1-flash"]);
    assert_eq!(waits(&slept), [6000]);
}

#[test]
fn a_retry_that_cannot_start_leaves_the_refusal_to_the_fallback() {
    let (machine, _) = sleeping(free_machine());
    let steps = vec![Step::Refused(refused_for(5)), Step::Error(AttemptError::Other("cannot".into())), Step::Pass];
    let (_, seen) = drive(&machine, &free_input(r#","escalate":false"#), steps);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash", "free/deepseek-v4.1-flash", "opencode/deepseek-v4.1-flash-free"]);
}

#[test]
fn the_wait_is_the_machines_to_set_and_turned_off_with_zero_retries() {
    let off = FREE_DEFAULTS.replace("\"settings\": {", "\"settings\": {\n    \"retry_wait\": {\"max_retries\": 0},");
    let (machine, slept) = sleeping(Machine::new(&off));
    machine.process.answer("opencode", Ok(ProcessResult { stdout: "opencode/deepseek-v4.1-flash-free\n".to_string(), stderr: String::new(), exit_code: 0, timed_out: false }));
    let (_, seen) = drive(&machine, &free_input(r#","escalate":false"#), vec![Step::Refused(refused_for(5)), Step::Pass]);
    assert_eq!(models(&seen), ["free/deepseek-v4.1-flash", "opencode/deepseek-v4.1-flash-free"]);
    assert!(waits(&slept).is_empty());
}
