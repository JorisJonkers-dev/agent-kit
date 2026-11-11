use super::*;
use std::cell::RefCell;
use std::path::PathBuf;

/// 2026-11-11T09:30:00Z.
const NOW: u64 = 1_794_389_400;
const NEDAP: &str = "https://github.com/nedap/healthcare.git";
const OTHER: &str = "git@github.com:JorisJonkers-dev/agent-kit.git";
const FAMILIES: &str = "# agent id, family\nvibe\tmistral\nagy\tgoogle\nclaude-work\tanthropic\ncodex\topenai\ncodex-spark\topenai\n";
const LIVE: &str = "2026-11-12T09:30:00Z";

struct Fake {
    remotes: Result<Vec<(String, String)>, String>,
    env: Vec<(&'static str, String)>,
    out: RefCell<String>,
    err: RefCell<String>,
}

impl Clock for Fake {
    fn unix_seconds(&self) -> u64 {
        NOW
    }
}

impl Host for Fake {
    fn remotes(&self, _repo: &Path) -> Result<Vec<(String, String)>, String> {
        self.remotes.clone()
    }
    fn env(&self, key: &str) -> Option<String> {
        self.env
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
    }
    fn print(&self, stderr: bool, text: &str) {
        let target = if stderr { &self.err } else { &self.out };
        target.borrow_mut().push_str(text);
    }
}

/// One scenario: a scratch directory holding the inputs, the fake host and
/// the arguments of the call.
struct Case {
    dir: PathBuf,
    host: Fake,
    args: Vec<(String, String)>,
}

fn case(tag: &str, instance: &str, remotes: &[&str]) -> Case {
    let dir = std::env::temp_dir().join(format!("gb-run-start-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let mut c = Case {
        host: Fake {
            remotes: Ok(remotes
                .iter()
                .enumerate()
                .map(|(i, url)| {
                    (
                        if i == 0 {
                            "origin".to_string()
                        } else {
                            format!("r{i}")
                        },
                        url.to_string(),
                    )
                })
                .collect()),
            env: vec![("HOME", dir.join("home").to_string_lossy().into_owned())],
            out: RefCell::default(),
            err: RefCell::default(),
        },
        args: Vec::new(),
        dir,
    };
    c.workflow(WRAPPED);
    c.families(FAMILIES);
    let (dir, report) = (c.path("."), c.path("reports"));
    for (flag, value) in [
        ("instance", instance),
        ("repo", dir.as_str()),
        ("workflow-name", "build-feature"),
        ("run-id", "run-9"),
        ("attempt", "3"),
        ("workflow", c.path("workflow.json").as_str()),
        ("families", c.path("families.tsv").as_str()),
        ("report-dir", report.as_str()),
    ] {
        c.set(flag, value);
    }
    c
}

impl Case {
    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }
    fn write(&self, name: &str, text: &str) {
        fs::write(self.dir.join(name), text).unwrap();
    }
    fn workflow(&mut self, text: &str) {
        self.write("workflow.json", text);
    }
    fn families(&mut self, text: &str) {
        self.write("families.tsv", text);
    }
    fn set(&mut self, flag: &str, value: &str) {
        self.args.retain(|(f, _)| f != flag);
        self.args.push((flag.to_string(), value.to_string()));
    }
    fn unset(&mut self, flag: &str) {
        self.args.retain(|(f, _)| f != flag);
    }
    fn allowlist(&mut self, text: &str) {
        self.write("allowlist.json", text);
        let path = self.path("allowlist.json");
        self.set("allowlist", &path);
    }
    fn grants(&mut self, text: &str) {
        self.write("grants.json", text);
        let path = self.path("grants.json");
        self.set("grants", &path);
    }
    fn command_line(&self) -> Vec<String> {
        let mut line = vec!["check".to_string()];
        for (flag, value) in &self.args {
            line.push(format!("--{flag}"));
            line.push(value.clone());
        }
        line
    }
    fn go(&self) -> Outcome {
        self.go_with(self.command_line())
    }
    fn go_with(&self, line: Vec<String>) -> Outcome {
        self.host.out.borrow_mut().clear();
        self.host.err.borrow_mut().clear();
        let code = run_with(line.iter().map(OsString::from), &self.host);
        let report = gold_band_report::read(&self.dir.join("reports").join(REPORT_FILE)).ok();
        Outcome {
            code,
            out: self.host.out.borrow().clone(),
            err: self.host.err.borrow().clone(),
            report,
        }
    }
}

struct Outcome {
    code: i32,
    out: String,
    err: String,
    report: Option<Report>,
}

impl Outcome {
    /// A refusal: exit 1, the report names the failed check and carries the
    /// run, the attempt, the exit code and the hash of what was printed.
    fn refused(&self, check: &str) -> &str {
        assert_eq!(self.code, 1, "{}", self.err);
        let report = self.report.as_ref().expect("a report is written");
        assert_eq!(report.failed_check.as_deref(), Some(check));
        assert_eq!(report.exit_code, 1);
        assert_eq!(
            report.output_hash,
            gold_band_report::sha256_hex(self.err.as_bytes())
        );
        assert!(self.out.is_empty());
        for line in self.err.lines() {
            assert!(line.starts_with("gold-band-run-start: "), "{line}");
        }
        &self.err
    }

    fn passed(&self) {
        assert_eq!(self.code, 0, "{}", self.err);
        assert_eq!(self.out, "gold-band-run-start: ok\n");
        assert!(self.err.is_empty());
        let report = self.report.as_ref().expect("a report is written");
        assert_eq!(report.exit_code, 0);
        assert_eq!(report.failed_check, None);
        assert_eq!(
            report.output_hash,
            gold_band_report::sha256_hex(self.out.as_bytes())
        );
    }

    fn bad_input(&self, check: &str) -> &str {
        assert_eq!(self.code, 2, "{}", self.err);
        let report = self.report.as_ref().expect("a failed report is written");
        assert_eq!(
            (report.exit_code, report.failed_check.as_deref()),
            (2, Some(check))
        );
        assert_eq!(
            report.output_hash,
            gold_band_report::sha256_hex(self.err.as_bytes())
        );
        &self.err
    }
}

/// An authoring wrapper: one vibe worker bound by slot to `agy`, a codex
/// worker by provider, and a manual-check node that is not a worker.
const WRAPPED: &str = r#"{"workflow":{"nodes":[
  {"type":"worker","id":"draft","executionSlotId":"slot-draft","provider":"vibe"},
  {"type":"worker","id":"review","executionSlotId":"slot-review","provider":"claude-work"},
  {"type":"manual","id":"human"}],
  "edges":[]},
 "modelBindings":{"bindings":[{"executionSlotId":"slot-draft","agentId":"vibe"},
                              {"executionSlotId":"slot-unused","agentId":"agy"}]}}"#;

fn workflow_with(agents: &[(&str, &str)]) -> String {
    let nodes: Vec<String> = agents
        .iter()
        .map(|(id, agent)| format!(r#"{{"type":"worker","id":"{id}","provider":"{agent}"}}"#))
        .collect();
    format!(r#"{{"nodes":[{}],"edges":[]}}"#, nodes.join(","))
}

fn entry(code: bool, families: &str) -> String {
    format!(
        r#"{{"code":{code},"families":[{families}],"history":false,"updated_at":"2026-11-01T00:00:00Z"}}"#
    )
}

fn allowlist_of(key: &str, entry: &str) -> String {
    format!(r#"{{"version":1,"repositories":{{"{key}":{entry}}}}}"#)
}

fn grant(workflow: &str, client: &str, expires: &str, ended: Option<&str>) -> String {
    let ended = ended.map_or(String::new(), |t| format!(r#","ended_at":"{t}""#));
    format!(
        r#"{{"answer":"yes","cap_usd":5,"client":"{client}","expires_at":"{expires}","id":"g-1","question":"may codex run?","workflow":"{workflow}","written_at":"2026-11-11T08:00:00Z"{ended}}}"#
    )
}

fn grants_of(list: &[String]) -> String {
    format!(r#"{{"version":1,"grants":[{}]}}"#, list.join(","))
}

const RUN_NAME: &str = "build-feature@github.com/nedap/healthcare@run-9";

// ---- repository rule ---------------------------------------------------

#[test]
fn personal_refuses_every_nedap_url_form_and_case() {
    for url in [
        "https://github.com/nedap/x.git",
        "https://github.com/Nedap/x",
        "https://user:token@github.com/NEDAP/x/",
        "http://github.com/nedap/x.git",
        "ssh://git@github.com:22/nedap/x.git",
        "ssh://git@github.com/nedap/x.git",
        "git@github.com:Nedap/x.git",
        "github.com:nedap/x",
        "git://github.com/nedap/x.git",
    ] {
        let c = case("nedap-forms", "gold-band-personal", &[url]);
        let o = c.go();
        let text = o.refused("repository-rule");
        assert!(
            text.contains(&format!("remote origin ({url}) is owned by nedap")),
            "{text}"
        );
        assert!(text.contains("gold-band-personal refuses nedap"), "{text}");
    }
}

#[test]
fn personal_allows_other_owners_and_no_remote() {
    for remotes in [
        &[OTHER][..],
        &[][..],
        &["https://gitlab.com/group/x.git", OTHER][..],
    ] {
        let c = case("personal-ok", "gold-band-personal", remotes);
        c.go().passed();
    }
}

#[test]
fn personal_refuses_when_any_remote_is_nedap() {
    let c = case("personal-mixed", "gold-band-personal", &[OTHER, NEDAP]);
    let o = c.go();
    let text = o.refused("repository-rule");
    assert_eq!(text.lines().count(), 2, "{text}"); // the nedap remote, and the family rule on a nedap repo
    assert!(text.contains("remote r1 (https://github.com/nedap/healthcare.git) is owned by nedap"));
}

#[test]
fn work_refuses_a_remote_not_owned_by_nedap() {
    let c = case("work-other", "gold-band", &[OTHER]);
    let o = c.go();
    let text = o.refused("repository-rule");
    assert!(
        text.contains(&format!(
            "remote origin ({OTHER}) is owned by jorisjonkers-dev"
        )),
        "{text}"
    );
    assert!(
        text.contains("gold-band runs only on nedap repositories"),
        "{text}"
    );
}

#[test]
fn work_refuses_when_any_remote_is_not_nedap() {
    let c = case("work-mixed", "gold-band", &[NEDAP, OTHER]);
    let text = c.go().refused("repository-rule").to_string();
    assert!(text.contains("remote r1"), "{text}");
    assert!(!text.contains("remote origin"), "{text}");
}

#[test]
fn work_refuses_a_repository_with_no_remote() {
    let c = case("work-none", "gold-band", &[]);
    let o = c.go();
    assert!(o
        .refused("repository-rule")
        .contains("repository has no remote"));
}

#[test]
fn a_remote_without_a_repository_key_is_refused_by_both_instances() {
    for instance in ["gold-band", "gold-band-personal"] {
        for url in [
            "/srv/git/x.git",
            "/srv/a:b/c.git",
            "file:///srv/x.git",
            "https://github.com/x",
            "https://github.com//x/y",
            "ssh://git@host",
        ] {
            let c = case("no-key", instance, &[url]);
            let o = c.go();
            assert!(
                o.refused("repository-rule")
                    .contains("is not a repository URL"),
                "{url}"
            );
        }
    }
}

#[test]
fn origin_is_listed_first_and_each_remote_once() {
    let text = "up\thttps://a/b/c (fetch)\nup\thttps://a/b/c (push)\norigin\thttps://d/e/f (fetch)\norigin\thttps://d/e/f (push)\n\nodd\n";
    assert_eq!(
        parse_remotes(text),
        [
            ("origin".to_string(), "https://d/e/f".to_string()),
            ("up".to_string(), "https://a/b/c".to_string())
        ]
    );
}

// ---- family allowlist --------------------------------------------------

#[test]
fn no_entry_and_no_allowlist_file_is_the_personal_default() {
    let mut c = case("default", "gold-band-personal", &[OTHER]);
    c.set("allowlist", &c.path("absent.json"));
    c.go().passed();
}

#[test]
fn an_entry_without_the_bound_family_names_node_family_and_entry() {
    let mut c = case("forbidden", "gold-band-personal", &[OTHER]);
    // Entry keys are lowercased by the allowlist, so a mixed-case key still matches.
    c.allowlist(&allowlist_of(
        "GitHub.com/JorisJonkers-dev/Agent-Kit",
        &entry(true, r#""anthropic""#),
    ));
    let o = c.go();
    let text = o.refused("family-allowlist");
    assert!(text.contains("node draft: family mistral (agent vibe) is not allowed by entry github.com/jorisjonkers-dev/agent-kit"), "{text}");
    assert!(!text.contains("node review"), "{text}");
}

#[test]
fn an_entry_with_code_off_refuses_a_workflow_that_sends_code() {
    let mut c = case("code-off", "gold-band-personal", &[OTHER]);
    c.allowlist(&allowlist_of(
        "github.com/jorisjonkers-dev/agent-kit",
        &entry(false, r#""mistral","anthropic""#),
    ));
    let o = c.go();
    assert!(o
        .refused("family-allowlist")
        .contains("workflow: entry github.com/jorisjonkers-dev/agent-kit does not allow code"));
}

#[test]
fn an_entry_that_allows_every_bound_family_and_code_passes_without_history() {
    let mut c = case("entry-ok", "gold-band-personal", &[OTHER]);
    c.allowlist(&allowlist_of(
        "github.com/jorisjonkers-dev/agent-kit",
        &entry(true, r#""mistral","anthropic""#),
    ));
    c.go().passed();
    // An entry with no history field, no code field and no families field at all: code is off.
    c.allowlist(&allowlist_of("github.com/jorisjonkers-dev/agent-kit", "{}"));
    let o = c.go();
    let text = o.refused("family-allowlist");
    assert!(
        text.contains("does not allow code") && text.contains("family mistral"),
        "{text}"
    );
}

#[test]
fn a_workflow_with_no_worker_sends_no_code() {
    let mut c = case("no-code", "gold-band-personal", &[OTHER]);
    c.workflow(r#"{"nodes":[{"type":"manual","id":"human"}],"edges":[]}"#);
    c.allowlist(&allowlist_of(
        "github.com/jorisjonkers-dev/agent-kit",
        &entry(false, ""),
    ));
    c.go().passed();
}

#[test]
fn every_remote_with_an_entry_must_allow_the_family() {
    let mut c = case(
        "two-entries",
        "gold-band-personal",
        &[OTHER, "https://github.com/someone/fork.git"],
    );
    let allowed = entry(true, r#""mistral","anthropic""#);
    c.allowlist(&format!(
        r#"{{"version":1,"repositories":{{"github.com/jorisjonkers-dev/agent-kit":{allowed},"github.com/someone/fork":{}}}}}"#,
        entry(true, r#""anthropic""#)
    ));
    let o = c.go();
    let text = o.refused("family-allowlist");
    assert!(text.contains("entry github.com/someone/fork"), "{text}");
    assert!(
        !text.contains("entry github.com/jorisjonkers-dev/agent-kit"),
        "{text}"
    );
}

#[test]
fn an_agent_missing_from_the_families_file_is_refused() {
    let mut c = case("no-family", "gold-band-personal", &[OTHER]);
    c.workflow(&workflow_with(&[("a", "vibe"), ("b", "mystery")]));
    let o = c.go();
    let text = o.refused("family-allowlist");
    assert!(
        text.contains("node b: agent mystery has no family in"),
        "{text}"
    );
    assert_eq!(text.lines().count(), 1);
}

#[test]
fn nedap_repositories_reach_only_claude_work_and_codex_whatever_an_entry_says() {
    let mut c = case("nedap-agents", "gold-band", &[NEDAP]);
    // The entry would forbid everything; it is not even read (it is not valid JSON).
    c.allowlist("not json at all");
    c.workflow(&workflow_with(&[
        ("a", "claude-work"),
        ("b", "vibe"),
        ("c", "codex"),
    ]));
    c.grants(&grants_of(&[grant(RUN_NAME, "codex", LIVE, None)]));
    let o = c.go();
    let text = o.refused("family-allowlist");
    assert!(
        text.contains("node b: agent vibe is not allowed on a nedap repository"),
        "{text}"
    );
    assert_eq!(text.lines().count(), 1, "{text}");
}

#[test]
fn nedap_repositories_allow_code_and_ignore_a_narrow_entry() {
    let mut c = case("nedap-ok", "gold-band", &[NEDAP]);
    c.allowlist(&allowlist_of(
        "github.com/nedap/healthcare",
        &entry(false, ""),
    ));
    c.workflow(&workflow_with(&[("a", "claude-work"), ("b", "codex")]));
    c.grants(&grants_of(&[grant(RUN_NAME, "codex", LIVE, None)]));
    c.go().passed();
}

#[test]
fn the_wrapper_bindings_providers_and_dynamic_strategies_are_all_bound_agents() {
    let mut c = case("bound", "gold-band-personal", &[OTHER]);
    c.workflow(
        r#"{"workflow":{"nodes":[
          {"type":"worker","id":"w","executionSlotId":"s1","provider":"vibe"},
          {"type":"worker","id":"x","executionSlotId":"unbound"},
          {"type":"worker","id":"blank","provider":"  "},
          {"type":"ai-dynamic","id":"d","agentStrategy":{"provider":"claude-work","availableAgents":[{"provider":"agy"},{"provider":"agy"},{}]}},
          {"type":"ai-dynamic","id":"bare"},
          {"type":"worker","provider":"vibe","executionSlotId":"s1"}]},
         "modelBindings":{"bindings":[{"executionSlotId":"s1","agentId":"agy"},{"executionSlotId":"s1"}]}}"#,
    );
    c.allowlist(&allowlist_of(
        "github.com/jorisjonkers-dev/agent-kit",
        &entry(true, r#""mistral""#),
    ));
    let o = c.go();
    let text = o.refused("family-allowlist");
    let lines: Vec<&str> = text.lines().collect();
    // w: slot binding agy (google) is refused, its provider vibe (mistral) is fine;
    // d: claude-work (anthropic) and agy (google, once); the unnamed node binds agy through s1.
    assert_eq!(lines.len(), 4, "{text}");
    assert!(
        lines[0].contains("node w: family google (agent agy)"),
        "{text}"
    );
    assert!(
        lines[1].contains("node d: family anthropic (agent claude-work)"),
        "{text}"
    );
    assert!(
        lines[2].contains("node d: family google (agent agy)"),
        "{text}"
    );
    assert!(
        lines[3].contains("node ?: family google (agent agy)"),
        "{text}"
    );
}

#[test]
fn a_bad_allowlist_is_unreadable_input() {
    for (text, why) in [
        ("{", "not JSON"),
        (r#"{"version":2,"repositories":{}}"#, "version must be 1"),
        (r#"{"version":1}"#, "repositories must be an object"),
    ] {
        let mut c = case("bad-allowlist", "gold-band-personal", &[OTHER]);
        c.allowlist(text);
        let o = c.go();
        assert!(o.bad_input("input").contains(why), "{why}");
    }
    let mut c = case("dir-allowlist", "gold-band-personal", &[OTHER]);
    c.set("allowlist", &c.path("."));
    assert!(c.go().bad_input("input").contains("cannot read"));
}

// ---- grants ------------------------------------------------------------

fn codex_case(tag: &str) -> Case {
    let mut c = case(tag, "gold-band", &[NEDAP]);
    c.workflow(&workflow_with(&[
        ("plan", "claude-work"),
        ("code", "codex"),
        ("review", "codex"),
    ]));
    c
}

#[test]
fn work_refuses_a_codex_run_with_no_grant_and_says_how_to_ask() {
    let mut c = codex_case("no-grant");
    c.set("grants", &c.path("absent.json"));
    let o = c.go();
    let text = o.refused("grant");
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(
        text.contains("node code (agent codex), node review (agent codex)"),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "no live grant covers workflow {RUN_NAME}, so codex may not run: ask the owner"
        )),
        "{text}"
    );
    assert!(text.contains("model-gateway grant write --json '{\"workflow\":\"build-feature@github.com/nedap/healthcare@run-9\",\"client\":\"codex\",\"cap_usd\":5,"), "{text}");
    assert!(text.contains("(as of 2026-11-11T09:30:00Z)"), "{text}");
}

#[test]
fn a_live_grant_for_this_run_passes() {
    let mut c = codex_case("live");
    c.grants(&grants_of(&[
        grant("other-workflow@x@y", "codex", LIVE, None),
        grant(RUN_NAME, "codex", "2026-11-12T09:30:00.000Z", None),
    ]));
    c.go().passed();
}

#[test]
fn a_grant_for_another_run_or_client_does_not_cover_this_run() {
    for (workflow, client) in [
        ("build-feature@github.com/nedap/healthcare@run-8", "codex"),
        ("build-feature@github.com/nedap/other@run-9", "codex"),
        ("other@github.com/nedap/healthcare@run-9", "codex"),
        (RUN_NAME, "claude"),
    ] {
        let mut c = codex_case("other-grant");
        c.grants(&grants_of(&[grant(workflow, client, LIVE, None)]));
        let o = c.go();
        let text = o.refused("grant");
        assert!(text.contains("no live grant covers workflow"), "{text}");
        assert!(!text.contains("grant g-1"), "{text}");
    }
}

#[test]
fn an_ended_or_expired_grant_is_not_live_and_is_named() {
    let mut c = codex_case("lapsed");
    c.grants(&grants_of(&[grant(
        RUN_NAME,
        "codex",
        LIVE,
        Some("2026-11-11T09:00:00Z"),
    )]));
    assert!(c.go().refused("grant").contains("(grant g-1 ended)"));
    let mut c = codex_case("expired");
    c.grants(&grants_of(&[grant(
        RUN_NAME,
        "codex",
        "2026-11-11T09:29:59Z",
        None,
    )]));
    assert!(c
        .go()
        .refused("grant")
        .contains("(grant g-1 expired at 2026-11-11T09:29:59Z)"));
}

#[test]
fn a_grant_is_live_until_the_second_it_expires() {
    let mut c = codex_case("boundary");
    c.grants(&grants_of(&[grant(
        RUN_NAME,
        "codex",
        "2026-11-11T09:30:01Z",
        None,
    )]));
    c.go().passed();
    c.set("now", "2026-11-11T09:30:01Z");
    assert!(c
        .go()
        .refused("grant")
        .contains("expired at 2026-11-11T09:30:01Z"));
    c.set("now", "2026-11-11T09:30:00.500Z");
    c.go().passed();
}

#[test]
fn a_run_with_no_codex_agent_needs_no_grant_and_never_reads_the_file() {
    let mut c = case("no-codex", "gold-band", &[NEDAP]);
    c.grants("not json");
    c.workflow(&workflow_with(&[("a", "claude-work")]));
    c.go().passed();
}

#[test]
fn any_agent_id_containing_codex_needs_the_grant() {
    // The repository rule refuses this run too; the grant is named on top of it.
    let mut c = case("codex-spark", "gold-band", &[OTHER]);
    c.workflow(&workflow_with(&[("a", "Codex-Spark")]));
    c.families("Codex-Spark\topenai\n");
    let o = c.go();
    let text = o.refused("repository-rule");
    assert!(text.contains("node a (agent Codex-Spark): no live grant covers workflow build-feature@github.com/jorisjonkers-dev/agent-kit@run-9"), "{text}");
}

#[test]
fn the_personal_instance_never_asks_for_a_grant() {
    let mut c = case("personal-codex", "gold-band-personal", &[OTHER]);
    c.workflow(&workflow_with(&[("a", "codex")]));
    c.grants("not json");
    c.go().passed();
}

#[test]
fn the_run_is_named_by_origin_and_a_missing_remote_leaves_that_to_the_rule() {
    let mut c = case(
        "origin-name",
        "gold-band",
        &[NEDAP, "https://github.com/nedap/mirror.git"],
    );
    c.workflow(&workflow_with(&[("a", "codex")]));
    c.grants(&grants_of(&[grant(RUN_NAME, "codex", LIVE, None)]));
    c.go().passed();
    // No remote: the repository rule refuses, and there is no repository to name a grant by.
    let c = case("no-remote-codex", "gold-band", &[]);
    let mut c = c;
    c.workflow(&workflow_with(&[("a", "codex")]));
    let o = c.go();
    assert_eq!(o.refused("repository-rule").lines().count(), 1);
}

#[test]
fn a_malformed_grants_file_is_unreadable_input() {
    let live = grant(RUN_NAME, "codex", LIVE, None);
    let cases: Vec<(String, &str)> = vec![
        ("{".to_string(), "not JSON"),
        (
            r#"{"version":2,"grants":[]}"#.to_string(),
            "version must be 1",
        ),
        (r#"{"version":1}"#.to_string(), "grants must be a list"),
        (
            grants_of(&[live.replace(r#""cap_usd":5"#, r#""cap_usd":0"#)]),
            "grants[0].cap_usd must be a number above 0",
        ),
        (
            grants_of(&[live.replace(r#""cap_usd":5"#, r#""cap_usd":"5""#)]),
            "grants[0].cap_usd",
        ),
        (
            grants_of(&[live.replace(r#""answer":"yes""#, r#""answer":"""#)]),
            "grants[0].answer must be a non-empty string",
        ),
        (
            grants_of(&[live.replace(r#""id":"g-1","#, "")]),
            "grants[0].id must be a non-empty string",
        ),
        (
            grants_of(&[live.replace(LIVE, "tomorrow")]),
            "grants[0].expires_at must be an ISO-8601 UTC timestamp",
        ),
        (
            grants_of(&[live.replace(
                r#""written_at":"2026-11-11T08:00:00Z""#,
                r#""written_at":1"#,
            )]),
            "grants[0].written_at",
        ),
        (
            grants_of(&[grant(RUN_NAME, "codex", LIVE, Some("later"))]),
            "grants[0].ended_at",
        ),
    ];
    for (text, why) in cases {
        let mut c = codex_case("bad-grants");
        c.grants(&text);
        let o = c.go();
        assert!(o.bad_input("input").contains(why), "{why}: {}", o.err);
    }
    let mut c = codex_case("dir-grants");
    c.set("grants", &c.path("."));
    assert!(c.go().bad_input("input").contains("cannot read"));
}

// ---- inputs, defaults and usage ---------------------------------------

#[test]
fn unreadable_workflow_families_or_repository_exit_2_with_a_failed_report() {
    let mut c = case("bad-workflow", "gold-band-personal", &[OTHER]);
    c.set("workflow", &c.path("absent.json"));
    assert!(c
        .go()
        .bad_input("input")
        .contains("absent.json: cannot read"));
    for (text, why) in [
        ("{", "not JSON"),
        (r#"{"nodes":3}"#, "nodes must be a list"),
        ("[]", "nodes must be a list"),
    ] {
        c.workflow(text);
        c.set("workflow", &c.path("workflow.json"));
        assert!(c.go().bad_input("input").contains(why), "{why}");
    }
    let mut c = case("bad-families", "gold-band-personal", &[OTHER]);
    c.families("vibe mistral\n");
    assert!(c
        .go()
        .bad_input("input")
        .contains("line 1: is not <agent id><TAB><family>"));
    c.set("families", &c.path("absent.tsv"));
    assert!(c
        .go()
        .bad_input("input")
        .contains("absent.tsv: cannot read"));
    let mut c = case("bad-repo", "gold-band-personal", &[OTHER]);
    c.host.remotes = Err("fatal: not a git repository".to_string());
    assert!(c
        .go()
        .bad_input("input")
        .contains("cannot list the remotes: fatal: not a git repository"));
}

#[test]
fn the_default_paths_follow_xdg_config_home_then_home() {
    let mut c = codex_case("default-paths");
    let config = c.dir.join("xdg/agent-kit");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("model-gateway-grants.json"),
        grants_of(&[grant(RUN_NAME, "codex", LIVE, None)]),
    )
    .unwrap();
    c.host.env = vec![("XDG_CONFIG_HOME", c.path("xdg"))];
    c.go().passed();
    // HOME alone: ~/.config/agent-kit, which holds nothing here.
    c.host.env = vec![("HOME", c.path("home"))];
    assert!(c.go().refused("grant").contains("no live grant"));
    // The personal allowlist default path is read the same way.
    let mut p = case("default-allowlist", "gold-band-personal", &[OTHER]);
    let config = p.dir.join("xdg/agent-kit");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("critique-allowlist.json"),
        allowlist_of("github.com/jorisjonkers-dev/agent-kit", &entry(true, "")),
    )
    .unwrap();
    p.host.env = vec![("XDG_CONFIG_HOME", p.path("xdg"))];
    assert!(p
        .go()
        .refused("family-allowlist")
        .contains("family mistral"));
    // Neither variable: the path is unknown, so say so.
    let mut n = case("no-env", "gold-band-personal", &[OTHER]);
    n.host.env.clear();
    assert!(n.go().bad_input("input").contains("pass it by flag"));
    let mut w = codex_case("no-env-grants");
    w.host.env.clear();
    assert!(w
        .go()
        .bad_input("input")
        .contains("model-gateway-grants.json is unknown"));
}

#[test]
fn bad_usage_exits_2_prints_usage_and_still_reports_when_it_can() {
    let c = case("usage", "gold-band-personal", &[OTHER]);
    let report_dir = c.path("reports");
    for (line, why) in [
        (vec![], "the one subcommand is check"),
        (vec!["verify".to_string()], "the one subcommand is check"),
        (
            vec!["check".into(), "stray".into()],
            "unexpected argument stray",
        ),
        (
            vec!["check".into(), "--bogus".into(), "x".into()],
            "unexpected argument --bogus",
        ),
        (
            vec!["check".into(), "--repo".into()],
            "--repo needs a value",
        ),
        (
            vec![
                "check".into(),
                "--repo".into(),
                "a".into(),
                "--repo".into(),
                "b".into(),
            ],
            "--repo given twice",
        ),
    ] {
        let o = c.go_with(line);
        assert_eq!(o.code, 2);
        assert!(o.err.contains(why), "{}", o.err);
        assert!(
            o.err
                .contains("usage: gold-band-run-start check --instance"),
            "{}",
            o.err
        );
        assert!(o.report.is_none(), "no report dir was named");
        c.host.err.borrow_mut().clear();
    }
    // With a report dir on the line, a bad flag still leaves a failed report with what is known.
    let line = vec![
        "check".to_string(),
        "--run-id".into(),
        "run-3".into(),
        "--attempt".into(),
        "2".into(),
        "--report-dir".into(),
        report_dir,
        "--wat".into(),
        "1".into(),
    ];
    let o = c.go_with(line);
    let report = o.report.as_ref().unwrap();
    assert_eq!((o.code, report.failed_check.as_deref()), (2, Some("usage")));
    assert_eq!((report.run_id.as_str(), report.attempt), ("run-3", 2));
    assert_eq!(report.exit_code, 2);
    assert!(report
        .command
        .starts_with("gold-band-run-start check --run-id run-3"));
}

#[test]
fn a_missing_or_malformed_flag_value_is_usage_and_reports_unknown_run_id() {
    let mut c = case("flags", "gold-band-personal", &[OTHER]);
    for (flag, value, why) in [
        (
            "instance",
            "gold-band-other",
            "--instance gold-band-other is not",
        ),
        ("attempt", "-1", "--attempt must be a non-negative integer"),
        ("now", "soon", "--now must be RFC 3339 UTC"),
    ] {
        let mut d = case("flags-bad", "gold-band-personal", &[OTHER]);
        d.set(flag, value);
        let o = d.go();
        assert!(o.bad_input("usage").contains(why), "{why}: {}", o.err);
    }
    for flag in [
        "instance",
        "repo",
        "workflow-name",
        "run-id",
        "attempt",
        "workflow",
        "families",
    ] {
        let mut d = case("flags-missing", "gold-band-personal", &[OTHER]);
        d.unset(flag);
        let o = d.go();
        assert!(
            o.bad_input("usage")
                .contains(&format!("--{flag} is required")),
            "{flag}"
        );
        let report = o.report.unwrap();
        if flag == "run-id" {
            assert_eq!(report.run_id, "unknown");
        }
        if flag == "attempt" {
            assert_eq!(report.attempt, 0);
        }
    }
    c.unset("report-dir");
    let o = c.go();
    assert_eq!(o.code, 2);
    assert!(o.report.is_none());
}

#[test]
fn an_unusable_report_directory_is_exit_2_whatever_the_check_found() {
    let mut c = case("report-dir", "gold-band-personal", &[OTHER]);
    c.write("blocker", "a file");
    c.set("report-dir", &c.path("blocker"));
    let o = c.go();
    assert_eq!(o.code, 2);
    assert!(o.err.contains("cannot write the report"), "{}", o.err);
    // A usage failure with that directory only says it could not report.
    let o = c.go_with(vec![
        "check".into(),
        "--report-dir".into(),
        c.path("blocker"),
    ]);
    assert_eq!(o.code, 2);
    assert!(o.err.contains("--instance is required") && o.err.contains("cannot write the report"));
}

#[test]
fn the_report_carries_the_command_the_clock_and_the_hash_of_the_printed_output() {
    let c = case("report", "gold-band-personal", &[NEDAP]);
    let o = c.go();
    o.refused("repository-rule");
    let report = o.report.unwrap();
    assert_eq!(report.run_id, "run-9");
    assert_eq!(report.attempt, 3);
    assert_eq!(report.timestamp, "2026-11-11T09:30:00Z");
    assert!(report
        .command
        .starts_with("gold-band-run-start check --instance gold-band-personal --repo "));
    assert!(report.command.contains("--report-dir"));
    // The file on disk is the stable-order JSON of the same report.
    let on_disk = fs::read_to_string(c.dir.join("reports").join(REPORT_FILE)).unwrap();
    assert_eq!(on_disk, format!("{}\n", report.to_json()));
}

// ---- the real host -----------------------------------------------------

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn real_case(tag: &str, instance: &str, remotes: &[(&str, &str)]) -> Case {
    let c = case(tag, instance, &[]);
    let repo = c.dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    for (name, url) in remotes {
        git(&repo, &["remote", "add", name, url]);
    }
    let mut c = c;
    c.set("repo", &repo.to_string_lossy());
    c
}

fn real(c: &Case) -> i32 {
    run(c.command_line().iter().map(OsString::from))
}

#[test]
fn the_real_host_reads_every_remote_from_git() {
    let c = real_case(
        "real-nedap",
        "gold-band-personal",
        &[("up", OTHER), ("origin", NEDAP)],
    );
    let remotes = RealHost::new().remotes(&c.dir.join("repo")).unwrap();
    assert_eq!(remotes[0], ("origin".to_string(), NEDAP.to_string()));
    assert_eq!(remotes[1], ("up".to_string(), OTHER.to_string()));
    assert_eq!(real(&c), 1);
    let report = gold_band_report::read(&c.dir.join("reports").join(REPORT_FILE)).unwrap();
    assert_eq!(report.failed_check.as_deref(), Some("repository-rule"));
}

#[test]
fn the_real_host_passes_a_repository_with_no_remote_on_the_personal_instance() {
    let mut c = real_case("real-bare", "gold-band-personal", &[]);
    assert!(RealHost::new()
        .remotes(&c.dir.join("repo"))
        .unwrap()
        .is_empty());
    c.set("allowlist", &c.path("absent.json"));
    assert_eq!(real(&c), 0);
    let report = gold_band_report::read(&c.dir.join("reports").join(REPORT_FILE)).unwrap();
    assert_eq!((report.exit_code, report.failed_check), (0, None));
    assert!(parse_rfc3339(&report.timestamp).unwrap() > 1_794_000_000);
}

#[test]
fn the_real_host_refuses_a_directory_that_is_not_a_repository() {
    let c = case("real-nongit", "gold-band-personal", &[]);
    let not_repo = c.dir.join("plain");
    fs::create_dir_all(&not_repo).unwrap();
    // A scratch directory under /tmp may sit inside another repository only if
    // the machine's temp dir is one; GIT_CEILING_DIRECTORIES is not needed for a fresh path.
    let err = RealHost::new().remotes(&c.dir.join("absent")).unwrap_err();
    assert!(!err.is_empty());
    let mut c = c;
    c.set("repo", &c.path("absent"));
    assert_eq!(real(&c), 2);
}

#[test]
fn a_missing_git_executable_is_unreadable_input() {
    let host = RealHost {
        git: "gb-no-such-git",
    };
    let err = host.remotes(Path::new(".")).unwrap_err();
    assert!(err.starts_with("cannot run git"), "{err}");
    assert_eq!(RealHost::default().git, "git");
}

#[test]
fn the_real_host_reads_the_process_environment() {
    assert!(RealHost::new().env("PATH").is_some());
    assert_eq!(
        RealHost::new().env("GOLD_BAND_RUN_START_SURELY_UNSET"),
        None
    );
    assert!(RealHost::new().unix_seconds() > 1_794_000_000);
}
