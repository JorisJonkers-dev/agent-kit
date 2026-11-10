//! Tests at the two seams: the `Client` interface (GitHub faked in memory)
//! for the plan and the apply, and the `Runner` for the real client's
//! request shapes.

use crate::body::{body, labels, normalize, ranges, title};
use crate::client::*;
use crate::model::{stories, Model, Result, SPEC_KEY};
use crate::plan::{apply, plan, Action};
use crate::{parse_args, record_numbers, run, sync, Options};
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
    fn close(&mut self, number: u64, comment: &str) -> Result<()> {
        self.check("close")?;
        self.issues.get_mut(&number).unwrap().open = false;
        self.log.push(format!("close {number}: {comment}"));
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
    assert!(p.render(&m, false).contains("nothing to do"));
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
    let text = p.render(&m, false);
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
        "close     #50 superseded by B0-01 #11  (needs --approve-close)",
        "plan: 2 create, 1 retitle, 1 rebody, 1 unlink, 3 link, 1 unblock, 2 block, 1 close",
    ] {
        assert!(text.contains(expect), "missing {expect:?} in\n{text}");
    }
    assert!(p.render(&m, true).contains("superseded by B0-01 #11\n"));
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
    let done = apply(&p, &mut m, &st, &mut f, true).unwrap();
    assert_eq!(done.created, vec![("B1".to_string(), 101), ("B1-01".to_string(), 102)]);
    assert_eq!(done.closed, vec![50]);
    assert_eq!(done.skipped_closes, 0);
    assert!(f.log.iter().any(|l| l == "close 50: Superseded by #11 (B0-01) of spec #1."));
    assert!(f.log.contains(&"create 102 [\"ready-for-agent\", \"area: agents\", \"type: feature\"]".to_string()));
    // The child body names the branch number that did not exist at plan time.
    assert!(f.issues[&102].body.starts_with("## Parent\n\n#101 (B1 Next), spec #1"));
    let again = plan(&m, &st, &mut f).unwrap();
    assert!(again.is_empty(), "second run planned {:?}", again.actions);
}

#[test]
fn a_close_waits_for_approval() {
    let (m, mut f) = full_published();
    let mut m = m;
    let st = stories_map();
    let p = plan(&m, &st, &mut f).unwrap();
    let done = apply(&p, &mut m, &st, &mut f, false).unwrap();
    assert_eq!((done.closed.len(), done.skipped_closes), (0, 1));
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
    for op in ["create", "edit", "add_sub_issue", "remove_sub_issue", "add_blocked_by", "remove_blocked_by", "close", "issue"] {
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
        assert_eq!(apply(&p, &mut m, &st, &mut f, true).unwrap_err(), format!("{op} failed"), "{op}");
    }
    // The spec issue itself must exist.
    let (mut m, mut f) = full_published();
    f.issues.remove(&1);
    assert!(plan(&m, &stories_map(), &mut f).is_err());
    assert!(apply(&Default::default(), &mut m, &st, &mut f, false).is_err());
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
    assert_eq!(ranges(&[5, 1, 2, 3, 7, 8]), "1-3, 5, 7-8");
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
    let out = record_numbers(YAML, &[("B1".into(), 12), ("B0-02".into(), 13), ("ghost".into(), 99)]);
    let m = Model::parse(&out).unwrap();
    assert_eq!((m.number("B1"), m.number("B0-02")), (Some(12), Some(13)));
    assert_eq!(out.lines().count(), YAML.lines().count() + 2);
}

fn s(a: &[&str]) -> Vec<OsString> {
    a.iter().map(OsString::from).collect()
}

#[test]
fn arguments() {
    let o = parse_args(s(&["--repo", "o/r", "--tickets", "t", "--spec", "s", "--apply", "--approve-close"])).unwrap().unwrap();
    assert_eq!(o, Options { repo: "o/r".into(), tickets: "t".into(), spec: "s".into(), apply: true, approve_close: true });
    let d = parse_args(s(&[])).unwrap().unwrap();
    assert!(!d.apply && d.tickets.ends_with("tickets.yaml") && d.spec.ends_with("spec.md") && d.repo.ends_with("agent-kit"));
    assert!(parse_args(s(&["-h"])).unwrap().is_none());
    assert!(parse_args(s(&["--help"])).unwrap().is_none());
    for flag in ["--repo", "--tickets", "--spec"] {
        assert!(parse_args(s(&[flag])).unwrap_err().contains("needs a value"));
    }
    assert!(parse_args(s(&["--wat"])).unwrap_err().contains("unknown argument"));
    assert!(parse_args(s(&["--approve-close"])).unwrap_err().contains("--apply"));
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
    c.close(1, "bye").unwrap();
    assert_eq!(calls.borrow().len(), 7 + 1);
    assert_eq!(last(&calls), "api -X PATCH repos/o/r/issues/1 --input - | {\"state\":\"closed\",\"state_reason\":\"not_planned\"}");
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

fn script(dir: &std::path::Path, text: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("gh");
    std::fs::write(&p, format!("#!/bin/sh\n{text}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn the_process_runner_runs_a_program() {
    let d = scratch("proc");
    let ok = script(&d, "echo \"$@\"; if [ \"$1\" = in ]; then cat; fi");
    assert_eq!(ProcessRunner::new(&ok).run(&["a".into(), "b".into()], None).unwrap(), "a b\n");
    assert_eq!(ProcessRunner::new(&ok).run(&["in".into()], Some("piped")).unwrap(), "in\npiped");
    let bad = script(&d, "echo nope >&2; exit 3");
    let e = ProcessRunner::new(&bad).run(&["x".into()], None).unwrap_err();
    assert!(e.ends_with("x: nope"), "{e}");
    let e = ProcessRunner::new("/nonexistent/gh").run(&[], None).unwrap_err();
    assert!(e.starts_with("/nonexistent/gh:"));
    // A program that exits without reading its input: the write fails.
    let quick = script(&d, "exec 0<&-");
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
    let d = scratch("cmd");
    let tickets = d.join("tickets.yaml");
    std::fs::write(&tickets, "spec: 1\nlabels:\n- l\nbranches:\n  B0:\n    title: T\n    what: w\ntickets: []\n").unwrap();
    let spec = d.join("spec.md");
    std::fs::write(&spec, SPEC).unwrap();
    let gh = script(
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
    assert!(out.contains("applied: 2 actions, 1 created, 0 closed, 0 closes held back"), "{out}");
    assert!(std::fs::read_to_string(&tickets).unwrap().contains("  B0:\n    title: T\n    issue: 9\n"));

    // Now published: an apply that creates nothing leaves the file alone.
    let (code, out, _) = go(&["--repo", "o/r", "--tickets", t, "--spec", sp, "--apply"], &gh);
    assert_eq!(code, 0);
    assert!(out.contains("applied: 2 actions, 0 created"), "{out}");
}

#[test]
fn the_command_reports_what_went_wrong() {
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
    let bad = script(&d, "echo denied >&2; exit 1");
    let sp = d.join("spec.md");
    std::fs::write(&sp, SPEC).unwrap();
    let (code, _, err) = go(&["--tickets", t.to_str().unwrap(), "--spec", sp.to_str().unwrap()], &bad);
    assert!(code == 1 && err.contains("denied"), "{err}");
    // Output that cannot be written, and a file that cannot be rewritten.
    let mut f = Fake::published(&model());
    let o = Options { repo: "o/r".into(), tickets: t.to_str().unwrap().into(), spec: sp.to_str().unwrap().into(), apply: false, approve_close: false };
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
    let o = Options { tickets: ro.to_str().unwrap().into(), apply: true, ..o };
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
    assert!(apply(&p, &mut m2, &less, &mut f2, true).unwrap_err().contains("story 4"));
    // Apply: the rebody of a branch whose story is missing.
    let (mut m3, mut f3) = full_published();
    f3.issues.get_mut(&10).unwrap().body = "stale".into();
    let p = plan(&m3, &st, &mut f3).unwrap();
    let mut no1 = st.clone();
    no1.remove(&1);
    assert!(apply(&p, &mut m3, &no1, &mut f3, true).unwrap_err().contains("story 1"));
    // Plan: the superseded issue cannot be read.
    let (m4, mut f4) = full_published();
    f4.issues.remove(&50);
    assert!(plan(&m4, &st, &mut f4).unwrap_err().contains("no issue 50"));
    // Apply: an issue disappears between the plan and the apply.
    let (mut m5, mut f5) = full_published();
    let p = plan(&m5, &st, &mut f5).unwrap();
    f5.issues.remove(&13);
    assert!(apply(&p, &mut m5, &st, &mut f5, true).unwrap_err().contains("no issue 13"));
}

#[test]
fn the_client_reports_failed_posts() {
    let (mut c, _) = gh(vec![Err("no create".into())]);
    assert_eq!(c.create("t", "b", &[]).unwrap_err(), "no create");
    let (mut c, _) = gh(vec![Err("no comment".into())]);
    assert_eq!(c.close(1, "x").unwrap_err(), "no comment");
}

#[test]
fn the_command_reports_a_bad_file_and_a_failed_apply() {
    let d = scratch("bad");
    let (t, sp) = (d.join("t.yaml"), d.join("spec.md"));
    std::fs::write(&sp, SPEC).unwrap();
    std::fs::write(&t, ": [").unwrap();
    let (code, _, err) = go(&["--tickets", t.to_str().unwrap(), "--spec", sp.to_str().unwrap()], "gh");
    assert!(code == 1 && err.contains("tickets file"));
    std::fs::write(&t, "spec: 1\nlabels: []\nbranches:\n  B0:\n    title: T\n    what: w\ntickets: []\n").unwrap();
    let gh = script(
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
    // Usage that cannot be written.
    let mut err = Vec::new();
    assert_eq!(run(s(&["--help"]), "gh", &mut Broken, &mut err), 1);
}
