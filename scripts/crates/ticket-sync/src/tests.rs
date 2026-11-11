//! Tests at the two seams: the `Client` interface (GitHub faked in memory)
//! for the plan and the apply, and the `Runner` for the real client's
//! request shapes.

use crate::body::{body, labels, normalize, ranges, title};
use crate::client::*;
use crate::model::{stories, Model, Result, SPEC_KEY};
use crate::close::{approve, close, closings, fingerprint, Closing};
use crate::model::dispositions;
use crate::plan::{apply, plan, Action, Applied};
use crate::{approval_path, parse_args, record_numbers, run, sync, Mode, Options};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

const YAML: &str = "\
spec: 1
labels:
- ready-for-agent
- 'area: agents'
branches:
  B0:
    title: B0 Foundations
    issue: 10
    what: The base.
  B1:
    title: B1 Next
    what: The next.
tickets:
- key: B0-01
  issue: 11
  branch: B0
  stories:
  - 1
  - 2
  - 3
  - 5
  title: First
  blocked_by: []
  supersedes:
  - 50
  lanes:
  - direct
  hosts:
  - linux
  - mac
  what: Build the first.
  ac:
  - It works.
  - It is tested.
- key: B0-02
  branch: B0
  title: Second
  blocked_by:
  - B0-01
  lanes:
  - direct
  hosts:
  - linux
  what: Build the second.
  ac:
  - Done.
- key: B1-01
  branch: B1
  stories:
  - 4
  title: Third
  blocked_by:
  - B0-02
  lanes:
  - direct
  hosts:
  - linux
  what: Build the third.
  ac:
  - Done.
";

const SPEC: &str = "# Spec\n\n1. As a dev, I want one.\n2. As a dev, I want two.\n3. As a dev, I want three.\n4. As a dev, I want four.\n5. As a dev, I want five.\nnot a story\n12 As nothing\n";

fn model() -> Model {
    Model::parse(YAML).unwrap()
}

fn stories_map() -> BTreeMap<u64, String> {
    stories(SPEC)
}

#[derive(Default)]
struct Fake {
    issues: BTreeMap<u64, Issue>,
    subs: BTreeMap<u64, Vec<u64>>,
    deps: BTreeMap<u64, Vec<u64>>,
    next: u64,
    log: Vec<String>,
    fail_on: Option<&'static str>,
}

impl Fake {
    fn new() -> Fake {
        Fake { next: 100, ..Fake::default() }
    }
    fn put(&mut self, number: u64, title: &str, body: &str, open: bool) {
        self.issues.insert(number, Issue { number, id: number + 1000, title: title.into(), body: body.into(), open });
    }
    fn link(&self, n: &u64) -> Link {
        Link { number: *n, id: n + 1000 }
    }
    fn check(&self, op: &'static str) -> Result<()> {
        if self.fail_on == Some(op) { Err(format!("{op} failed")) } else { Ok(()) }
    }
    fn number_of(&self, id: u64) -> u64 {
        id - 1000
    }
    /// Make GitHub hold exactly what the model describes.
    fn published(model: &Model) -> Fake {
        let st = stories_map();
        let mut f = Fake::new();
        f.put(model.spec, "Spec", "spec body", true);
        for key in model.keys() {
            if let Some(n) = model.number(&key) {
                f.put(n, &title(model, &key), &body(model, &key, &st).unwrap(), true);
            }
        }
        f.put(50, "Old", "old", true);
        for key in model.keys() {
            if let Some(n) = model.number(&key) {
                let parent = model.number(&model.parent(&key)).unwrap();
                f.subs.entry(parent).or_default().push(n);
                if let Some(t) = model.ticket(&key) {
                    for b in &t.blocked_by {
                        f.deps.entry(n).or_default().push(model.number(b).unwrap());
                    }
                }
            }
        }
        f
    }
}

impl Client for Fake {
    fn issue(&mut self, number: u64) -> Result<Issue> {
        self.check("issue")?;
        self.issues.get(&number).cloned().ok_or_else(|| format!("no issue {number}"))
    }
    fn create(&mut self, title: &str, body: &str, labels: &[String]) -> Result<Issue> {
        self.check("create")?;
        self.next += 1;
        let n = self.next;
        self.put(n, title, body, true);
        self.log.push(format!("create {n} {labels:?}"));
        Ok(self.issues[&n].clone())
    }
    fn edit(&mut self, number: u64, title: Option<&str>, body: Option<&str>) -> Result<()> {
        self.check("edit")?;
        let i = self.issues.get_mut(&number).unwrap();
        if let Some(t) = title {
            i.title = t.into();
        }
        if let Some(b) = body {
            i.body = b.into();
        }
        self.log.push(format!("edit {number}"));
        Ok(())
    }
    fn sub_issues(&mut self, parent: u64) -> Result<Vec<Link>> {
        self.check("sub_issues")?;
        Ok(self.subs.get(&parent).map(|v| v.iter().map(|n| self.link(n)).collect()).unwrap_or_default())
    }
    fn add_sub_issue(&mut self, parent: u64, child_id: u64) -> Result<()> {
        self.check("add_sub_issue")?;
        let c = self.number_of(child_id);
        self.subs.entry(parent).or_default().push(c);
        self.log.push(format!("link {parent} {c}"));
        Ok(())
    }
    fn remove_sub_issue(&mut self, parent: u64, child_id: u64) -> Result<()> {
        self.check("remove_sub_issue")?;
        let c = self.number_of(child_id);
        self.subs.get_mut(&parent).unwrap().retain(|n| *n != c);
        self.log.push(format!("unlink {parent} {c}"));
        Ok(())
    }
    fn blocked_by(&mut self, number: u64) -> Result<Vec<Link>> {
        self.check("blocked_by")?;
        Ok(self.deps.get(&number).map(|v| v.iter().map(|n| self.link(n)).collect()).unwrap_or_default())
    }
    fn add_blocked_by(&mut self, number: u64, blocker_id: u64) -> Result<()> {
        self.check("add_blocked_by")?;
        let b = self.number_of(blocker_id);
        self.deps.entry(number).or_default().push(b);
        self.log.push(format!("block {number} {b}"));
        Ok(())
    }
    fn remove_blocked_by(&mut self, number: u64, blocker_id: u64) -> Result<()> {
        self.check("remove_blocked_by")?;
        let b = self.number_of(blocker_id);
        self.deps.get_mut(&number).unwrap().retain(|n| *n != b);
        self.log.push(format!("unblock {number} {b}"));
        Ok(())
    }
    fn close(&mut self, number: u64, comment: &str, completed: bool) -> Result<()> {
        self.check("close")?;
        self.issues.get_mut(&number).unwrap().open = false;
        self.log.push(format!("close {number} completed={completed}: {comment}"));
        Ok(())
    }
}

fn full_published() -> (Model, Fake) {
    let mut m = model();
    m.set_number("B1", 12);
    m.set_number("B0-02", 13);
    m.set_number("B1-01", 14);
    let f = Fake::published(&m);
    (m, f)
}

#[test]
fn a_run_against_the_published_state_plans_nothing() {
    let (m, mut f) = full_published();
    let mut p = plan(&m, &stories_map(), &mut f).unwrap();
    // The only thing left is the superseded issue, which stays listed.
    assert_eq!(p.actions, vec![Action::Close { number: 50, by: "B0-01".into() }]);
    f.issues.get_mut(&50).unwrap().open = false;
    p = plan(&m, &stories_map(), &mut f).unwrap();
    assert!(p.is_empty());
    assert!(p.render(&m).contains("nothing to do"));
}

#[test]
fn a_dry_run_lists_every_kind_of_change_and_writes_nothing() {
    let mut m = model();
    m.set_number("B0-02", 13);
    let mut f = Fake::published(&m);
    // Drift: wrong title, wrong body, an extra and a missing link.
    f.issues.get_mut(&11).unwrap().title = "Old title".into();
    f.issues.get_mut(&13).unwrap().body = "stale".into();
    f.subs.get_mut(&10).unwrap().retain(|n| *n != 13);
    f.subs.entry(1).or_default().push(13);
    f.deps.entry(13).or_default().push(10);
    f.deps.get_mut(&13).unwrap().retain(|n| *n != 11);
    f.deps.entry(13).or_default().push(777);
    // Issues outside the graph, linked under a graph parent, are left alone.
    f.subs.entry(10).or_default().push(888);
    let p = plan(&m, &stories_map(), &mut f).unwrap();
    let text = p.render(&m);
    for expect in [
        "create    B1: B1 Next (spec #1)",
        "create    B1-01: Third",
        "retitle   B0-01 #11: \"Old title\" -> \"First\"",
        "rebody    B0-02 #13",
        "unlink    B0-02 #13 from SPEC #1",
        "link      B0-02 #13 under B0 #10",
        "unblock   B0-02 #13 no longer blocked by B0 #10",
        "block     B0-02 #13 blocked by B0-01 #11",
        "block     B1-01 (new) blocked by B0-02 #13",
        "link      B1 (new) under SPEC #1",
        "close     #50 superseded by B0-01 #11  (needs an approved plan: --approve-plan, then --close)",
        "plan: 2 create, 1 retitle, 1 rebody, 1 unlink, 3 link, 1 unblock, 2 block, 1 close",
    ] {
        assert!(text.contains(expect), "missing {expect:?} in\n{text}");
    }
    assert!(f.log.is_empty(), "planning must not write");
}

#[test]
fn applying_converges_and_the_next_plan_is_empty() {
    let mut m = model();
    m.set_number("B0-02", 13);
    let mut f = Fake::published(&m);
    f.issues.get_mut(&11).unwrap().title = "Old title".into();
    f.issues.get_mut(&13).unwrap().body = "stale".into();
    f.subs.get_mut(&10).unwrap().retain(|n| *n != 13);
    f.subs.entry(1).or_default().push(13);
    f.deps.entry(13).or_default().push(10);
    let st = stories_map();
    let p = plan(&m, &st, &mut f).unwrap();
    let mut done = Applied::default();
    apply(&p, &mut m, &st, &mut f, &mut done).unwrap();
    assert_eq!(done.created, vec![("B1".to_string(), 101), ("B1-01".to_string(), 102)]);
    assert_eq!(done.skipped_closes, 1);
    assert!(f.issues[&50].open && !f.log.iter().any(|l| l.starts_with("close")), "an apply never closes");
    assert!(f.log.contains(&"create 102 [\"ready-for-agent\", \"area: agents\", \"type: feature\"]".to_string()));
    // The child body names the branch number that did not exist at plan time.
    assert!(f.issues[&102].body.starts_with("## Parent\n\n#101 (B1 Next), spec #1"));
    let again = plan(&m, &st, &mut f).unwrap();
    // Only the close, which waits for its own approved action, is left.
    assert_eq!(again.actions, vec![Action::Close { number: 50, by: "B0-01".into() }], "second run planned {:?}", again.actions);
}

#[test]
fn an_apply_leaves_the_closes_alone() {
    let (m, mut f) = full_published();
    let mut m = m;
    let st = stories_map();
    let p = plan(&m, &st, &mut f).unwrap();
    let mut done = Applied::default();
    apply(&p, &mut m, &st, &mut f, &mut done).unwrap();
    assert_eq!(done.skipped_closes, 1);
    assert!(f.issues[&50].open);
}

#[test]
fn errors_from_github_stop_the_plan_and_the_apply() {
    for op in ["issue", "sub_issues", "blocked_by"] {
        let (m, mut f) = full_published();
        f.fail_on = Some(op);
        assert_eq!(plan(&m, &stories_map(), &mut f).unwrap_err(), format!("{op} failed"));
    }
    let st = stories_map();
    for op in ["create", "edit", "add_sub_issue", "remove_sub_issue", "add_blocked_by", "remove_blocked_by", "issue"] {
        let mut m = model();
        m.set_number("B0-02", 13);
        let mut f = Fake::published(&m);
        f.issues.get_mut(&11).unwrap().title = "x".into();
        f.issues.get_mut(&13).unwrap().body = "x".into();
        f.subs.get_mut(&10).unwrap().retain(|n| *n != 13);
        f.subs.entry(1).or_default().push(13);
        f.deps.entry(13).or_default().push(10);
        f.deps.get_mut(&13).unwrap().retain(|n| *n != 11);
        let p = plan(&m, &st, &mut f).unwrap();
        f.fail_on = Some(op);
        assert_eq!(apply(&p, &mut m, &st, &mut f, &mut Applied::default()).unwrap_err(), format!("{op} failed"), "{op}");
    }
    // The spec issue itself must exist.
    let (mut m, mut f) = full_published();
    f.issues.remove(&1);
    assert!(plan(&m, &stories_map(), &mut f).is_err());
    assert!(apply(&Default::default(), &mut m, &st, &mut f, &mut Applied::default()).is_err());
}

#[test]
fn bodies_follow_the_published_template() {
    let m = model();
    let st = stories_map();
    assert_eq!(
        body(&m, "B0-01", &st).unwrap(),
        "## Parent\n\n#10 (B0 Foundations), spec #1\n\n## What to build\n\nBuild the first.\n\n## Acceptance criteria\n\n- [ ] It works.\n- [ ] It is tested.\n\n## Scope\n\n- Ticket key: `B0-01`\n- Lanes: direct\n- Hosts: linux, mac\n- User stories: 1-3, 5 (spec #1)\n\n## Replaces\n\n- #50\n\nThese are closed with a pointer to this ticket by the approved sync (B0-06).\n"
    );
    let supporting = body(&m, "B0-02", &st).unwrap();
    assert!(supporting.contains("- User stories: none (supporting ticket)\n"));
    assert!(!supporting.contains("Replaces"));
    assert!(supporting.starts_with("## Parent\n\n#10 (B0 Foundations)"));
    assert_eq!(
        body(&m, "B0", &st).unwrap(),
        "## Parent\n\n#1\n\n## What this branch covers\n\nThe base.\n\nIts tickets are the sub-issues of this issue; their blocking edges are set as native dependencies. Blockers in other branches are tickets of the same spec #1.\n\n## User stories\n\nThe stories of spec #1 that this branch's tickets deliver, numbered as in the spec.\n\n1. As a dev, I want one.\n2. As a dev, I want two.\n3. As a dev, I want three.\n5. As a dev, I want five.\n"
    );
    assert!(body(&m, "B1-01", &st).unwrap().starts_with("## Parent\n\n#? (B1 Next)"));
    assert!(body(&m, "B0", &BTreeMap::new()).unwrap_err().contains("story 1 is not in the spec"));
    assert!(body(&m, "nope", &st).is_err());
    assert_eq!(stories("99999999999999999999999. As huge").len(), 0);
    assert_eq!(title(&m, "B0"), "B0 Foundations (spec #1)");
    assert_eq!(title(&m, "B0-01"), "First");
    assert_eq!(title(&m, "nope"), "");
    assert_eq!(labels(&m, "B0"), vec!["area: agents", "type: feature"]);
    assert_eq!(labels(&m, "B0-01"), vec!["ready-for-agent", "area: agents", "type: feature"]);
    assert_eq!(ranges(&[5, 1, 2, 3, 7, 8, 1]), "1-3, 5, 7-8");
    assert_eq!(ranges(&[]), "");
    assert_eq!(normalize("a\r\nb  \n\n"), "a\nb");
}

#[test]
fn the_graph_is_validated() {
    assert!(Model::parse(": not yaml [").unwrap_err().starts_with("tickets file:"));
    let dup = YAML.replace("key: B0-02", "key: B0-01");
    assert_eq!(Model::parse(&dup).unwrap_err(), "duplicate ticket key B0-01");
    let branch = YAML.replace("branch: B1\n", "branch: B9\n");
    assert_eq!(Model::parse(&branch).unwrap_err(), "B1-01: unknown branch B9");
    let blocker = YAML.replace("- B0-02\n", "- B7-07\n");
    assert_eq!(Model::parse(&blocker).unwrap_err(), "B1-01: blocked by unknown ticket B7-07");
    let mut m = model();
    assert_eq!(m.number(SPEC_KEY), Some(1));
    assert_eq!(m.number("B1"), None);
    assert_eq!(m.number("zzz"), None);
    m.set_number("zzz", 5);
    assert_eq!(m.keys(), vec!["B0", "B1", "B0-01", "B0-02", "B1-01"]);
    assert_eq!(m.parent("B0"), SPEC_KEY);
    assert_eq!(m.parent("B0-01"), "B0");
}

#[test]
fn issue_numbers_are_written_back_with_line_edits() {
    let out = record_numbers(YAML, &[("B1".into(), 12), ("B0-02".into(), 13)]).unwrap();
    let m = Model::parse(&out).unwrap();
    assert_eq!((m.number("B1"), m.number("B0-02")), (Some(12), Some(13)));
    assert_eq!(out.lines().count(), YAML.lines().count() + 2);
    assert!(record_numbers(YAML, &[("ghost".into(), 99)]).unwrap_err().contains("ghost"));
}

fn s(a: &[&str]) -> Vec<OsString> {
    a.iter().map(OsString::from).collect()
}

#[test]
fn arguments() {
    let o = parse_args(s(&["--repo", "o/r", "--tickets", "t", "--spec", "s", "--approval", "a", "--apply"])).unwrap().unwrap();
    assert_eq!(o, Options { repo: "o/r".into(), tickets: "t".into(), spec: "s".into(), approval: "a".into(), mode: Mode::Apply });
    assert_eq!(parse_args(s(&["--approve-plan"])).unwrap().unwrap().mode, Mode::ApprovePlan);
    assert_eq!(parse_args(s(&["--close"])).unwrap().unwrap().mode, Mode::Close);
    let d = parse_args(s(&[])).unwrap().unwrap();
    assert!(d.mode == Mode::DryRun && d.approval.ends_with("agent-kit/ticket-sync-close-approval") && d.tickets.ends_with("tickets.yaml") && d.spec.ends_with("spec.md") && d.repo.ends_with("agent-kit"));
    assert!(parse_args(s(&["-h"])).unwrap().is_none());
    assert!(parse_args(s(&["--help"])).unwrap().is_none());
    for flag in ["--repo", "--tickets", "--spec", "--approval"] {
        assert!(parse_args(s(&[flag])).unwrap_err().contains("needs a value"));
    }
    assert!(parse_args(s(&["--wat"])).unwrap_err().contains("unknown argument"));
    for pair in [["--apply", "--close"], ["--close", "--approve-plan"], ["--approve-plan", "--apply"]] {
        assert!(parse_args(s(&pair)).unwrap_err().contains("mutually exclusive"), "{pair:?}");
    }
    // The approval file lives under XDG_STATE_HOME, else under the home directory.
    assert_eq!(approval_path(Some("/state".into()), Some("/h".into())), "/state/agent-kit/ticket-sync-close-approval");
    assert_eq!(approval_path(Some(String::new()), Some("/h".into())), "/h/.local/state/agent-kit/ticket-sync-close-approval");
    assert_eq!(approval_path(None, None), "/.local/state/agent-kit/ticket-sync-close-approval");
}

// ---- the real client over a fake runner ----

type Calls = std::rc::Rc<std::cell::RefCell<Vec<(Vec<String>, Option<String>)>>>;

struct Rec {
    calls: Calls,
    replies: Vec<Result<String>>,
}

impl Runner for Rec {
    fn run(&mut self, args: &[String], stdin: Option<&str>) -> Result<String> {
        self.calls.borrow_mut().push((args.to_vec(), stdin.map(str::to_string)));
        if self.replies.is_empty() { Ok("{}".into()) } else { self.replies.remove(0) }
    }
}

fn gh(replies: Vec<Result<String>>) -> (GhClient, Calls) {
    let calls = Calls::default();
    (GhClient::new("o/r", Box::new(Rec { calls: calls.clone(), replies })), calls)
}

fn last(calls: &Calls) -> String {
    let (a, i) = calls.borrow().last().unwrap().clone();
    format!("{} | {}", a.join(" "), i.unwrap_or_default())
}

#[test]
fn the_gh_client_sends_the_documented_requests() {
    let issue = r#"{"number":7,"id":70,"title":"T","body":null,"state":"open"}"#;
    let (mut c, calls) = gh(vec![Ok(issue.into()), Ok(issue.into())]);
    let got = c.issue(7).unwrap();
    assert_eq!(got, Issue { number: 7, id: 70, title: "T".into(), body: String::new(), open: true });
    assert_eq!(last(&calls), "api -X GET repos/o/r/issues/7 | ");
    c.create("T", "B", &["x".to_string()]).unwrap();
    assert_eq!(last(&calls), "api -X POST repos/o/r/issues --input - | {\"body\":\"B\",\"labels\":[\"x\"],\"title\":\"T\"}");

    let (mut c, calls) = gh(vec![]);
    c.edit(7, Some("T"), None).unwrap();
    assert_eq!(last(&calls), "api -X PATCH repos/o/r/issues/7 --input - | {\"title\":\"T\"}");
    c.edit(7, None, Some("B")).unwrap();
    assert_eq!(last(&calls), "api -X PATCH repos/o/r/issues/7 --input - | {\"body\":\"B\"}");
    c.add_sub_issue(1, 2).unwrap();
    assert_eq!(last(&calls), "api -X POST repos/o/r/issues/1/sub_issues --input - | {\"sub_issue_id\":2}");
    c.remove_sub_issue(1, 2).unwrap();
    assert_eq!(last(&calls), "api -X DELETE repos/o/r/issues/1/sub_issue --input - | {\"sub_issue_id\":2}");
    c.add_blocked_by(1, 2).unwrap();
    assert_eq!(last(&calls), "api -X POST repos/o/r/issues/1/dependencies/blocked_by --input - | {\"issue_id\":2}");
    c.remove_blocked_by(1, 2).unwrap();
    assert_eq!(last(&calls), "api -X DELETE repos/o/r/issues/1/dependencies/blocked_by/2 | ");
    c.close(1, "bye", false).unwrap();
    assert_eq!(calls.borrow().len(), 6 + 2);
    // The comment goes first, then the close.
    assert_eq!(calls.borrow()[6].1.as_deref(), Some("{\"body\":\"bye\"}"));
    assert_eq!(last(&calls), "api -X PATCH repos/o/r/issues/1 --input - | {\"state\":\"closed\",\"state_reason\":\"not_planned\"}");
    c.close(1, "done", true).unwrap();
    assert!(last(&calls).ends_with("{\"state\":\"closed\",\"state_reason\":\"completed\"}"));
}

#[test]
fn lists_are_read_across_pages() {
    let pages = r#"[{"number":1,"id":10,"x":1}][{"number":2,"id":20}]"#;
    let (mut c, calls) = gh(vec![Ok(pages.into()), Ok("[]".into()), Ok("[{\"number\":3,\"id\":30}]".into())]);
    let subs = c.sub_issues(9).unwrap();
    assert_eq!(subs, vec![Link { number: 1, id: 10 }, Link { number: 2, id: 20 }]);
    assert_eq!(last(&calls), "api --paginate repos/o/r/issues/9/sub_issues?per_page=100 | ");
    assert!(c.blocked_by(9).unwrap().is_empty());
    assert_eq!(c.blocked_by(9).unwrap().len(), 1);
    assert_eq!(last(&calls), "api --paginate repos/o/r/issues/9/dependencies/blocked_by?per_page=100 | ");
}

#[test]
fn the_gh_client_reports_failures() {
    let (mut c, _) = gh(vec![Ok("nonsense".into()), Err("boom".into()), Ok("{oops".into()), Err("down".into())]);
    assert!(c.issue(1).unwrap_err().starts_with("issue response:"));
    assert_eq!(c.issue(1).unwrap_err(), "boom");
    assert!(c.sub_issues(1).unwrap_err().contains("issues/1/sub_issues"));
    assert_eq!(c.blocked_by(1).unwrap_err(), "down");
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ticket-sync-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[path = "../../../test-support/fake_exe.rs"]
mod fake_exe;

fn script(held: &std::sync::MutexGuard<'static, ()>, dir: &std::path::Path, text: &str) -> String {
    let p = dir.join("gh");
    fake_exe::write_executable(held, &p, &format!("#!/bin/sh\n{text}\n"));
    p.to_string_lossy().into_owned()
}

#[test]
fn the_process_runner_runs_a_program() {
    let held = fake_exe::lock();
    let d = scratch("proc");
    let ok = script(&held, &d, "echo \"$@\"; if [ \"$1\" = in ]; then cat; fi");
    assert_eq!(ProcessRunner::new(&ok).run(&["a".into(), "b".into()], None).unwrap(), "a b\n");
    assert_eq!(ProcessRunner::new(&ok).run(&["in".into()], Some("piped")).unwrap(), "in\npiped");
    let bad = script(&held, &d, "echo nope >&2; exit 3");
    let e = ProcessRunner::new(&bad).run(&["x".into()], None).unwrap_err();
    assert!(e.ends_with("x: nope"), "{e}");
    let e = ProcessRunner::new("/nonexistent/gh").run(&[], None).unwrap_err();
    assert!(e.starts_with("/nonexistent/gh:"));
    // A program that exits without reading its input: the write fails.
    let quick = script(&held, &d, "exec 0<&-");
    let big = "x".repeat(4_000_000);
    assert!(ProcessRunner::new(&quick).run(&[], Some(&big)).is_err());
}

// ---- the command ----

struct Broken;
impl std::io::Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("closed"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn go(args: &[&str], gh: &str) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(s(args), gh, &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn the_command_dry_runs_and_applies_through_gh() {
    let held = fake_exe::lock();
    let d = scratch("cmd");
    let tickets = d.join("tickets.yaml");
    std::fs::write(&tickets, "spec: 1\nlabels:\n- l\nbranches:\n  B0:\n    title: T\n    what: w\ntickets: []\n").unwrap();
    let spec = d.join("spec.md");
    std::fs::write(&spec, SPEC).unwrap();
    let gh = script(
        &held,
        &d,
        r#"case "$*" in
*"-X GET repos/o/r/issues/9") echo '{"number":9,"id":1009,"title":"T (spec #1)","body":"stale","state":"open"}';;
*"-X GET repos/o/r/issues/1") echo '{"number":1,"id":1001,"title":"S","body":"b","state":"open"}';;
*"-X POST repos/o/r/issues --input -") cat >/dev/null; echo '{"number":9,"id":1009,"title":"T","body":"b","state":"open"}';;
*"--paginate"*) echo '[]';;
*) cat >/dev/null; echo '{}';;
esac"#,
    );
    let (t, sp) = (tickets.to_str().unwrap(), spec.to_str().unwrap());
    let (code, out, err) = go(&["--repo", "o/r", "--tickets", t, "--spec", sp], &gh);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("create    B0: T (spec #1)") && out.contains("dry run: nothing was written"), "{out}");
    assert!(!std::fs::read_to_string(&tickets).unwrap().contains("issue: 9"));

    let (code, out, _) = go(&["--repo", "o/r", "--tickets", t, "--spec", sp, "--apply"], &gh);
    assert_eq!(code, 0);
    assert!(out.contains("applied: 2 actions, 1 created, 0 closes left for --close"), "{out}");
    assert!(std::fs::read_to_string(&tickets).unwrap().contains("  B0:\n    title: T\n    issue: 9\n"));

    // Now published: an apply that creates nothing leaves the file alone.
    let (code, out, _) = go(&["--repo", "o/r", "--tickets", t, "--spec", sp, "--apply"], &gh);
    assert_eq!(code, 0);
    assert!(out.contains("applied: 2 actions, 0 created"), "{out}");
}

#[test]
fn the_command_reports_what_went_wrong() {
    let held = fake_exe::lock();
    let (code, out, _) = go(&["--help"], "gh");
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: ticket-sync"));
    let (code, _, err) = go(&["--nope"], "gh");
    assert_eq!((code, err.as_str()), (1, "ticket-sync: unknown argument --nope\n"));
    let (code, _, err) = go(&["--tickets", "/nonexistent/t.yaml"], "gh");
    assert!(code == 1 && err.contains("/nonexistent/t.yaml"));
    let d = scratch("missing-spec");
    let t = d.join("t.yaml");
    std::fs::write(&t, YAML).unwrap();
    let (code, _, err) = go(&["--tickets", t.to_str().unwrap(), "--spec", "/nonexistent/spec.md"], "gh");
    assert!(code == 1 && err.contains("/nonexistent/spec.md"));
    // gh itself failing surfaces as an error.
    let bad = script(&held, &d, "echo denied >&2; exit 1");
    let sp = d.join("spec.md");
    std::fs::write(&sp, SPEC).unwrap();
    let (code, _, err) = go(&["--tickets", t.to_str().unwrap(), "--spec", sp.to_str().unwrap()], &bad);
    assert!(code == 1 && err.contains("denied"), "{err}");
    // Output that cannot be written, and a file that cannot be rewritten.
    let mut f = Fake::published(&model());
    let o = Options { repo: "o/r".into(), tickets: t.to_str().unwrap().into(), spec: sp.to_str().unwrap().into(), approval: d.join("approval").to_str().unwrap().into(), mode: Mode::DryRun };
    assert_eq!(sync(&o, &mut f, &mut Broken).unwrap_err(), "closed");
    let mut out = Vec::new();
    let (mut m, mut f2) = full_published();
    m.set_number("B0-02", 13);
    f2.issues.get_mut(&1).unwrap().id = 1001;
    let ro = d.join("ro.yaml");
    std::fs::write(&ro, YAML).unwrap();
    let mut perm = std::fs::metadata(&ro).unwrap().permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(&ro, perm).unwrap();
    let o = Options { tickets: ro.to_str().unwrap().into(), mode: Mode::Apply, ..o };
    let mut f3 = Fake::published(&model());
    let err = sync(&o, &mut f3, &mut out).unwrap_err();
    assert!(err.contains("ro.yaml"), "{err}");
}

#[test]
fn story_and_lookup_failures_surface_from_plan_and_apply() {
    let st = stories_map();
    let mut less = st.clone();
    less.remove(&4);
    // Plan: an existing branch whose stories are not in the spec.
    let (m, mut f) = full_published();
    assert!(plan(&m, &BTreeMap::new(), &mut f).unwrap_err().contains("not in the spec"));
    // Apply: the create of a branch whose story is missing.
    let mut m2 = model();
    m2.set_number("B0-02", 13);
    let mut f2 = Fake::published(&m2);
    let p = plan(&m2, &less, &mut f2).unwrap();
    assert!(apply(&p, &mut m2, &less, &mut f2, &mut Applied::default()).unwrap_err().contains("story 4"));
    // Apply: the rebody of a branch whose story is missing.
    let (mut m3, mut f3) = full_published();
    f3.issues.get_mut(&10).unwrap().body = "stale".into();
    let p = plan(&m3, &st, &mut f3).unwrap();
    let mut no1 = st.clone();
    no1.remove(&1);
    assert!(apply(&p, &mut m3, &no1, &mut f3, &mut Applied::default()).unwrap_err().contains("story 1"));
    // Plan: the superseded issue cannot be read.
    let (m4, mut f4) = full_published();
    f4.issues.remove(&50);
    assert!(plan(&m4, &st, &mut f4).unwrap_err().contains("no issue 50"));
    // Apply: an issue disappears between the plan and the apply.
    let (mut m5, mut f5) = full_published();
    let p = plan(&m5, &st, &mut f5).unwrap();
    f5.issues.remove(&13);
    assert!(apply(&p, &mut m5, &st, &mut f5, &mut Applied::default()).unwrap_err().contains("no issue 13"));
}

#[test]
fn the_client_reports_failed_posts() {
    let (mut c, _) = gh(vec![Err("no create".into())]);
    assert_eq!(c.create("t", "b", &[]).unwrap_err(), "no create");
    let (mut c, _) = gh(vec![Err("no comment".into())]);
    assert_eq!(c.close(1, "x", false).unwrap_err(), "no comment");
    let (mut c, _) = gh(vec![Ok("{}".into()), Err("no close".into())]);
    assert_eq!(c.close(1, "x", false).unwrap_err(), "no close");
}

#[test]
fn the_command_reports_a_bad_file_and_a_failed_apply() {
    let held = fake_exe::lock();
    let d = scratch("bad");
    let (t, sp) = (d.join("t.yaml"), d.join("spec.md"));
    std::fs::write(&sp, SPEC).unwrap();
    std::fs::write(&t, ": [").unwrap();
    let (code, _, err) = go(&["--tickets", t.to_str().unwrap(), "--spec", sp.to_str().unwrap()], "gh");
    assert!(code == 1 && err.contains("tickets file"));
    std::fs::write(&t, "spec: 1\nlabels: []\nbranches:\n  B0:\n    title: T\n    what: w\ntickets: []\n").unwrap();
    let gh = script(
        &held,
        &d,
        r#"case "$*" in
*"-X GET repos/o/r/issues/1") echo '{"number":1,"id":1001,"title":"S","body":"b","state":"open"}';;
*"--paginate"*) echo '[]';;
*) cat >/dev/null; echo refused >&2; exit 1;;
esac"#,
    );
    let args = ["--repo", "o/r", "--tickets", t.to_str().unwrap(), "--spec", sp.to_str().unwrap(), "--apply"];
    let (code, _, err) = go(&args, &gh);
    assert!(code == 1 && err.contains("refused"), "{err}");
    // A create that succeeded is recorded even though a later step failed.
    std::fs::write(&t, "spec: 1\nlabels: []\nbranches:\n  B0:\n    title: T\n    what: w\ntickets: []\n").unwrap();
    let half = script(
        &held,
        &d,
        r#"case "$*" in
*"-X GET repos/o/r/issues/1") echo '{"number":1,"id":1001,"title":"S","body":"b","state":"open"}';;
*"-X POST repos/o/r/issues --input -") cat >/dev/null; echo '{"number":9,"id":1009,"title":"T","body":"b","state":"open"}';;
*"--paginate"*) echo '[]';;
*) cat >/dev/null; echo refused >&2; exit 1;;
esac"#,
    );
    let (code, _, err) = go(&args, &half);
    assert!(code == 1 && err.contains("refused"), "{err}");
    assert!(std::fs::read_to_string(&t).unwrap().contains("    issue: 9\n"));
    // Usage that cannot be written.
    let mut err = Vec::new();
    assert_eq!(run(s(&["--help"]), "gh", &mut Broken, &mut err), 1);
}

// ---- the approved close, at the client interface ----

/// A published graph whose B0-01 supersedes #50 (a superseded issue), #51 (a
/// delivered one) and #52 (an absorbed one), with an unrelated open #60.
fn closing_fixture() -> (Model, Fake, BTreeMap<u64, String>) {
    let (mut m, mut f) = full_published();
    m.tickets[0].supersedes = vec![50, 51, 52];
    for n in [51, 52, 60] {
        f.put(n, "Other", "other", true);
    }
    let disp = dispositions(
        "| Issue | Title | Branch | Disposition | Stories |\n| :- | :- | :- | :- | :- |\n\
         | #50 | Old | B0 | Close: umbrella superseded by this spec | 1 |\n\
         | #51 | Shipped | B0 | Close: done (installer from GitHub releases) | 132 |\n\
         | #52 | Review | B0 | Close: absorbed by the review stages | 88 |\n\
         | #60 | Keep | B0 | Rewrite: something else | 1 |\n| #61 | Odd | not a row |\n",
    );
    (m, f, disp)
}

fn open_issues(f: &Fake) -> Vec<u64> {
    f.issues.values().filter(|i| i.open).map(|i| i.number).collect()
}

#[test]
fn the_spec_dispositions_are_read_from_its_table() {
    let (_, _, disp) = closing_fixture();
    assert_eq!(disp.len(), 3);
    assert_eq!(disp[&51], "done (installer from GitHub releases)");
}

#[test]
fn every_closing_comment_links_its_ticket_and_delivered_ones_carry_evidence() {
    let (m, mut f, disp) = closing_fixture();
    let p = plan(&m, &stories_map(), &mut f).unwrap();
    let c = closings(&p, &m, &disp).unwrap();
    assert_eq!(
        c,
        vec![
            Closing { number: 50, comment: "Superseded by #11 (B0-01) of spec #1.".into(), completed: false },
            Closing { number: 51, comment: "Delivered: done (installer from GitHub releases). Tracked by #11 (B0-01) of spec #1.".into(), completed: true },
            Closing { number: 52, comment: "Superseded by #11 (B0-01) of spec #1 (absorbed by the review stages).".into(), completed: false },
        ]
    );
    // Without a table row the plain mapping comment is used; a repeat is closed once.
    let mut m2 = m.clone();
    m2.tickets[1].supersedes = vec![50];
    let p2 = plan(&m2, &stories_map(), &mut f).unwrap();
    let c2 = closings(&p2, &m2, &BTreeMap::new()).unwrap();
    assert_eq!(c2.iter().map(|c| c.number).collect::<Vec<_>>(), vec![50, 51, 52]);
    assert_eq!(c2[0].comment, "Superseded by #11 (B0-01) of spec #1.");
    // Closing is refused for a graph issue, and for a ticket not yet published.
    let mut m3 = m.clone();
    m3.tickets[0].supersedes = vec![13];
    let mut f3 = Fake::published(&m3);
    f3.put(13, "Graph", "g", true);
    let p3 = plan(&m3, &stories_map(), &mut f3).unwrap();
    assert!(closings(&p3, &m3, &disp).unwrap_err().contains("part of the ticket graph"));
    let mut m4 = model();
    m4.tickets[1].supersedes = vec![50];
    let mut f4 = Fake::published(&m4);
    let p4 = plan(&m4, &stories_map(), &mut f4).unwrap();
    assert!(closings(&p4, &m4, &disp).unwrap_err().contains("B0-02 is not published"));
}

#[test]
fn without_an_approval_the_close_closes_nothing() {
    let (m, mut f, disp) = closing_fixture();
    let p = plan(&m, &stories_map(), &mut f).unwrap();
    let c = closings(&p, &m, &disp).unwrap();
    let d = scratch("no-approval");
    let mut done = Vec::new();
    let err = close(&d.join("approval"), "o/r", &c, &mut f, &mut done).unwrap_err();
    assert!(err.contains("no approval recorded") && err.contains("--approve-plan"), "{err}");
    assert!(done.is_empty() && f.log.is_empty());
    assert_eq!(open_issues(&f), vec![1, 10, 11, 12, 13, 14, 50, 51, 52, 60]);
    // An unreadable approval is no approval either.
    std::fs::write(d.join("bad"), "not json").unwrap();
    assert!(close(&d.join("bad"), "o/r", &c, &mut f, &mut done).is_err());
    // Nor is the approval of an empty plan.
    assert!(approve(&d.join("empty"), "o/r", &[]).unwrap_err().contains("nothing to approve"));
    // An approval that cannot be written is an error, not a silent success.
    std::fs::write(d.join("file"), "x").unwrap();
    assert!(approve(&d.join("file/sub/approval"), "o/r", &c).unwrap_err().contains("file"));
    assert!(approve(&d, "o/r", &c).is_err());
    assert!(f.log.is_empty());
}

#[test]
fn an_approval_of_another_plan_closes_nothing() {
    let (m, mut f, disp) = closing_fixture();
    let p = plan(&m, &stories_map(), &mut f).unwrap();
    let c = closings(&p, &m, &disp).unwrap();
    let d = scratch("stale-approval");
    approve(&d.join("approval"), "o/r", &c[..2]).unwrap();
    let err = close(&d.join("approval"), "o/r", &c, &mut f, &mut Vec::new()).unwrap_err();
    assert!(err.contains("not for the current close plan") && err.contains(&fingerprint("o/r", &c)), "{err}");
    assert!(f.log.is_empty());
    // The comment is part of the plan: a changed mapping needs a new approval.
    approve(&d.join("approval"), "o/r", &c).unwrap();
    let mut changed = c.clone();
    changed[0].comment.push('!');
    assert!(close(&d.join("approval"), "o/r", &changed, &mut f, &mut Vec::new()).is_err());
    assert!(f.log.is_empty());
}

#[test]
fn with_an_approval_the_close_closes_exactly_the_plan() {
    let (m, mut f, disp) = closing_fixture();
    let p = plan(&m, &stories_map(), &mut f).unwrap();
    let c = closings(&p, &m, &disp).unwrap();
    let d = scratch("approved");
    approve(&d.join("state/approval"), "o/r", &c).unwrap();
    let mut done = Vec::new();
    close(&d.join("state/approval"), "o/r", &c, &mut f, &mut done).unwrap();
    assert_eq!(done, vec![50, 51, 52]);
    assert_eq!(
        f.log,
        vec![
            "close 50 completed=false: Superseded by #11 (B0-01) of spec #1.".to_string(),
            "close 51 completed=true: Delivered: done (installer from GitHub releases). Tracked by #11 (B0-01) of spec #1.".to_string(),
            "close 52 completed=false: Superseded by #11 (B0-01) of spec #1 (absorbed by the review stages).".to_string(),
        ]
    );
    // Nothing outside the plan was touched.
    assert_eq!(open_issues(&f), vec![1, 10, 11, 12, 13, 14, 60]);
    // A failing close stops there and reports what was closed.
    // The approval is used up: a reopened issue gives a plan that needs a new one.
    assert!(!d.join("state/approval").exists());
    f.issues.get_mut(&50).unwrap().open = true;
    f.log.clear();
    let p = plan(&m, &stories_map(), &mut f).unwrap();
    let again = closings(&p, &m, &disp).unwrap();
    assert!(close(&d.join("state/approval"), "o/r", &again, &mut f, &mut Vec::new()).unwrap_err().contains("no approval"));
    assert!(f.log.is_empty());
    // A failing close stops there and keeps the approval.
    let (_, mut f2, _) = closing_fixture();
    f2.fail_on = Some("close");
    approve(&d.join("state/approval"), "o/r", &c).unwrap();
    let mut done2 = Vec::new();
    assert_eq!(close(&d.join("state/approval"), "o/r", &c, &mut f2, &mut done2).unwrap_err(), "close failed");
    assert!(done2.is_empty() && d.join("state/approval").exists());
}

#[test]
fn the_command_approves_then_closes_and_only_then() {
    let (m, f, _) = closing_fixture();
    let mut f = f;
    let d = scratch("command");
    let (t, sp) = (d.join("t.yaml"), d.join("spec.md"));
    let mut yaml = YAML.replace("  supersedes:\n  - 50\n", "  supersedes:\n  - 50\n  - 51\n");
    yaml = yaml.replace("- key: B0-02\n", "- key: B0-02\n  issue: 13\n").replace("    title: B1 Next\n", "    title: B1 Next\n    issue: 12\n").replace("- key: B1-01\n", "- key: B1-01\n  issue: 14\n");
    std::fs::write(&t, &yaml).unwrap();
    std::fs::write(&sp, format!("{SPEC}\n| #50 | Old | B0 | Close: superseded by this spec | 1 |\n| #51 | Shipped | B0 | Close: done (shipped) | 2 |\n")).unwrap();
    assert_eq!(Model::parse(&yaml).unwrap().number("B1-01"), m.number("B1-01"));
    let opts = |mode| Options { repo: "o/r".into(), tickets: t.to_str().unwrap().into(), spec: sp.to_str().unwrap().into(), approval: d.join("approval").to_str().unwrap().into(), mode };
    // A dry run shows the close plan and its fingerprint, and records nothing.
    let mut out = Vec::new();
    sync(&opts(Mode::DryRun), &mut f, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("close     #51 (completed): Delivered: done (shipped)") && text.contains("close plan for o/r: 2 issues, fingerprint "), "{text}");
    assert!(!d.join("approval").exists());
    // Closing without an approval fails and closes nothing.
    let mut out = Vec::new();
    assert!(sync(&opts(Mode::Close), &mut f, &mut out).unwrap_err().contains("no approval recorded"));
    assert!(out.is_empty() && f.log.is_empty());
    // Approving writes the file only.
    let mut out = Vec::new();
    sync(&opts(Mode::ApprovePlan), &mut f, &mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("approval recorded in "));
    assert!(d.join("approval").exists() && f.log.is_empty());
    // An apply still closes nothing.
    sync(&opts(Mode::Apply), &mut f, &mut Vec::new()).unwrap();
    assert!(!f.log.iter().any(|l| l.starts_with("close")));
    // With the approval in place the close runs, and the output says what closed.
    let mut out = Vec::new();
    sync(&opts(Mode::Close), &mut f, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("closed #50\nclosed #51\nclosed 2 issues\n"), "{text}");
    assert_eq!(open_issues(&f), vec![1, 10, 11, 12, 13, 14, 52, 60]);
    // The plan is now empty, so the approval no longer matches and nothing more closes.
    assert!(sync(&opts(Mode::Close), &mut f, &mut Vec::new()).is_err());
    assert_eq!(f.log.iter().filter(|l| l.starts_with("close")).count(), 2);
    // A failure part-way still reports what was closed.
    let (_, mut f2, _) = closing_fixture();
    f2.fail_on = Some("close");
    sync(&opts(Mode::ApprovePlan), &mut f2, &mut Vec::new()).unwrap();
    let mut out = Vec::new();
    assert_eq!(sync(&opts(Mode::Close), &mut f2, &mut out).unwrap_err(), "close failed");
    assert!(out.is_empty());
}

#[test]
fn an_approval_is_bound_to_its_repository_and_its_exact_plan() {
    let (m, mut f, disp) = closing_fixture();
    let p = plan(&m, &stories_map(), &mut f).unwrap();
    let c = closings(&p, &m, &disp).unwrap();
    let d = scratch("bound");
    let a = d.join("approval");
    // Another repository cannot reuse it, and its fingerprint differs.
    approve(&a, "o/r", &c).unwrap();
    assert_ne!(fingerprint("o/r", &c), fingerprint("other/r", &c));
    assert!(close(&a, "other/r", &c, &mut f, &mut Vec::new()).is_err());
    // The recorded issue list is enforced too: a hand-edited file closes nothing.
    let text = std::fs::read_to_string(&a).unwrap();
    let issue_list = format!("{:?}", c.iter().map(|c| c.number).collect::<Vec<_>>());
    assert!(text.contains(&c[0].number.to_string()));
    let forged = text.replacen(&format!("{}", c[0].number), "999", 1);
    std::fs::write(&a, forged).unwrap();
    assert!(close(&a, "o/r", &c, &mut f, &mut Vec::new()).is_err(), "{issue_list}");
    assert!(f.log.is_empty());
    // Moving text between the comment of one closing and the next one cannot keep the fingerprint.
    let two = vec![
        Closing { number: 50, comment: "S".into(), completed: false },
        Closing { number: 51, comment: "D".into(), completed: true },
    ];
    let one = vec![Closing { number: 50, comment: "S;51;true;1:D;".into(), completed: false }];
    assert_ne!(fingerprint("o/r", &two), fingerprint("o/r", &one));
}

#[test]
fn only_clear_evidence_counts_as_delivered() {
    let (m, mut f, _) = closing_fixture();
    let p = plan(&m, &stories_map(), &mut f).unwrap();
    for (why, completed) in [("done (shipped)", true), ("done", true), ("covered by Hindsight", true), ("doneness check pending", false), ("covered? maybe", false), ("shipped", false)] {
        let disp = BTreeMap::from([(50, why.to_string())]);
        assert_eq!(closings(&p, &m, &disp).unwrap()[0].completed, completed, "{why}");
    }
}

#[test]
fn the_close_command_exits_non_zero_without_an_approval() {
    let held = fake_exe::lock();
    let d = scratch("exit");
    let (t, sp) = (d.join("t.yaml"), d.join("spec.md"));
    std::fs::write(&t, "spec: 1\nlabels: []\nbranches:\n  B0:\n    title: T\n    issue: 9\n    what: w\ntickets: []\n").unwrap();
    std::fs::write(&sp, SPEC).unwrap();
    let gh = script(
        &held,
        &d,
        r#"case "$*" in
*"-X GET repos/o/r/issues/"*) echo '{"number":9,"id":1009,"title":"T (spec #1)","body":"x","state":"open"}';;
*"--paginate"*) echo '[]';;
*) echo "WRITE $*" >> writes.log; echo '{}';;
esac"#,
    );
    let none = d.join("none");
    let args = ["--repo", "o/r", "--tickets", t.to_str().unwrap(), "--spec", sp.to_str().unwrap(), "--approval", none.to_str().unwrap(), "--close"];
    let (code, out, err) = go(&args, &gh);
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(err.contains("no approval recorded"), "{err}");
}
