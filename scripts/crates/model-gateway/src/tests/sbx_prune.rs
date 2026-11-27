//! The space the gateway's sandboxes leave in sbx: `sandbox prune`, and the removal of superseded images after a refresh.

use super::catalog::{cli, sandbox_runtime};
use super::scenarios::{FixedClock, Sandbox};
use crate::json::{self, Value};
use crate::ports::{Process, ProcessCommand, ProcessResult, Runtime};
use crate::sbx_images::{self, Record};
use crate::sbx_prune;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

const NOW: i64 = 1_795_780_800_000;
const PINNED: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OLD: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const BUSY: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const SHARED: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
const SHELL: &str = "sha256:4444444444444444444444444444444444444444444444444444444444444444";

/// What `sbx template ls --all --digests --json` prints: the pinned pi image under its two names, a superseded one
/// (also under its assembled name), one a sandbox still runs from, one a user also tagged, and two that are not the gateway's.
fn listing() -> String {
    let row = |id: &str, repository: &str, tag: &str, size: u64| format!(r#"{{"id":"{id}","repository":"{repository}","tag":"{tag}","digest":"{id}","size":{size}}}"#);
    let rows = [
        row(PINNED, "docker.io/sbx/pi-image", "", 800),
        row(PINNED, "docker.io/library/sandbox-kit-assembled", "aaaa", 800),
        row(OLD, "docker.io/sbx/pi-image", "", 700),
        row(OLD, "docker.io/library/sandbox-kit-assembled", "bbbb", 700),
        row(BUSY, "docker.io/sbx/opencode-image", "", 600),
        row(SHARED, "docker.io/sbx/vibe-image", "", 500),
        row(SHARED, "docker.io/me/mine", "keep", 500),
        row(SHELL, "docker.io/sbx/shell", "", 400),
        r#"{"repository":"docker.io/library/debian","tag":"stable-slim","size":-1}"#.to_string(),
        r#"{"id":"sha256:5555555555555555555555555555555555555555555555555555555555555555","repository":"docker.io/library/debian","tag":"stable-slim"}"#.to_string(),
    ];
    format!("[{}]", rows.join(","))
}

fn df(bytes: u64) -> String {
    format!(r#"{{"summaries":[{{"type":"Containers","size":1}},{{"type":"Images","size":{bytes}}}]}}"#)
}

type Handler = Box<dyn Fn(&str, &[&str]) -> Result<ProcessResult, String> + Send + Sync>;

/// A process whose answers a closure gives, which records every call.
struct Fake {
    handler: Handler,
    calls: Mutex<Vec<String>>,
    alive: Vec<u32>,
}

impl Fake {
    fn new(handler: impl Fn(&str, &[&str]) -> Result<ProcessResult, String> + Send + Sync + 'static) -> Fake {
        Fake { handler: Box::new(handler), calls: Mutex::new(Vec::new()), alive: Vec::new() }
    }
}

impl Process for Fake {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn is_alive(&self, pid: u32) -> bool {
        self.alive.contains(&pid)
    }
    fn exec(&self, command: &str, args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.calls.lock().expect("calls").push(format!("{command} {}", args.join(" ")));
        (self.handler)(command, args)
    }
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessResult, String> {
        Err("no client runs here".to_string())
    }
}

fn out(stdout: &str) -> Result<ProcessResult, String> {
    Ok(ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out: false })
}

fn fail(code: i32) -> Result<ProcessResult, String> {
    Ok(ProcessResult { stdout: String::new(), stderr: "nope\n".to_string(), exit_code: code, timed_out: false })
}

/// A machine whose records pin the pi image, with a gateway that is gone (pid 100) and one that is alive (pid 200).
fn machine(sandbox: &Sandbox, process: Fake) -> (Runtime, std::sync::Arc<Fake>) {
    let process = std::sync::Arc::new(process);
    let dir = format!("{}/state/agent-kit/model-gateway", sandbox.text());
    std::fs::create_dir_all(&dir).expect("state dir");
    let records = BTreeMap::from([("pi".to_string(), Record { tag: "20261125-x".to_string(), digest: PINNED.to_string(), checked_at: NOW })]);
    struct Shared(std::sync::Arc<Fake>);
    impl Process for Shared {
        fn resolve(&self, binary: &str) -> Option<String> {
            self.0.resolve(binary)
        }
        fn is_alive(&self, pid: u32) -> bool {
            self.0.is_alive(pid)
        }
        fn exec(&self, command: &str, args: &[&str], timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
            self.0.exec(command, args, timeout_ms)
        }
        fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
            self.0.run(command)
        }
    }
    let runtime = Runtime { clock: Box::new(FixedClock(NOW)), process: Box::new(Shared(process.clone())), ..sandbox_runtime(sandbox, None) };
    sbx_images::write_records(runtime.files.as_ref(), &format!("{dir}/sbx-images.json"), &records).expect("records");
    (runtime, process)
}

fn calls(process: &Fake) -> Vec<String> {
    process.calls.lock().expect("calls").clone()
}

fn sbx_world(removed: &'static Mutex<bool>) -> impl Fn(&str, &[&str]) -> Result<ProcessResult, String> + Send + Sync {
    move |_, args| match args {
        ["template", "ls", ..] => out(&listing()),
        ["ls", "-q"] => out("mg-100-aa\nmg-200-bb\nother\n"),
        ["ls", "--format", "json"] => out(&format!(r#"[{{"name":"mg-200-bb","template":"docker.io/sbx/opencode-image@{BUSY}"}}]"#)),
        ["df", "--json"] => out(&df(if *removed.lock().expect("removed") { 5_000 } else { 9_000 })),
        ["rm", ..] | ["template", "rm", ..] => {
            *removed.lock().expect("removed") = true;
            out("")
        }
        _ => fail(2),
    }
}

#[test]
fn a_prune_removes_the_gateways_leftovers_and_only_those_and_reports_what_sbx_says_it_freed() {
    static REMOVED: Mutex<bool> = Mutex::new(false);
    let sandbox = Sandbox::new();
    let (runtime, process) = machine(&sandbox, Fake { alive: vec![200], ..Fake::new(sbx_world(&REMOVED)) });

    let dry = cli(&runtime, &["sandbox", "prune", "--dry-run", "--json"]);
    assert_eq!(dry.exit_code, 0, "{}", dry.stderr);
    let answer = json::parse(&dry.stdout).expect("JSON");
    let object = answer.as_object().expect("object");
    assert_eq!(object.get("dry_run"), Some(&Value::Bool(true)));
    assert_eq!(object.get("sandboxes"), Some(&Value::Arr(vec![Value::str("mg-100-aa")])));
    let Some(Value::Arr(templates)) = object.get("templates") else { panic!("templates") };
    let ids: Vec<&str> = templates.iter().filter_map(|template| template.as_object()?.get("id")?.as_str()).collect();
    // The pinned image, the one a sandbox runs from, the one a user also tagged and the foreign ones all stay.
    assert_eq!(ids, [OLD], "{}", dry.stdout);
    assert_eq!(object.get("freed_bytes"), Some(&Value::Num(700.0)));
    assert!(calls(&process).iter().all(|call| !call.contains(" rm ")), "{:?}", calls(&process));

    let real = cli(&runtime, &["sandbox", "prune"]);
    assert_eq!(real.exit_code, 0, "{}", real.stderr);
    assert_eq!(real.stdout, format!("removed sandbox mg-100-aa\nremoved image {OLD} docker.io/sbx/pi-image (0.0 MB)\nremoved 0.0 MB (a dry run adds up the listed image sizes, which count shared layers once each)\n"), "{}", real.stdout);
    let rms: Vec<String> = calls(&process).into_iter().filter(|call| call.contains(" rm ")).collect();
    assert_eq!(rms, ["sbx rm --force mg-100-aa".to_string(), format!("sbx template rm --force {OLD}")]);
}

#[test]
fn the_freed_bytes_are_what_sbx_df_fell_by_and_with_all_every_unpinned_idle_image_goes() {
    static REMOVED: Mutex<bool> = Mutex::new(false);
    let sandbox = Sandbox::new();
    let (runtime, _) = machine(&sandbox, Fake { alive: vec![200], ..Fake::new(sbx_world(&REMOVED)) });
    let all = cli(&runtime, &["sandbox", "prune", "--all", "--json"]);
    assert_eq!(all.exit_code, 0, "{}", all.stderr);
    let answer = json::parse(&all.stdout).expect("JSON");
    let object = answer.as_object().expect("object");
    assert_eq!(object.get("freed_bytes"), Some(&Value::Num(4000.0)), "{}", all.stdout);
    let Some(Value::Arr(templates)) = object.get("templates") else { panic!("templates") };
    let mut ids: Vec<&str> = templates.iter().filter_map(|template| template.as_object()?.get("id")?.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, [OLD, SHARED, SHELL, "sha256:5555555555555555555555555555555555555555555555555555555555555555"], "{}", all.stdout);
}

#[test]
fn what_cannot_be_removed_is_a_warning_and_a_prune_without_sbx_or_records_removes_nothing() {
    let sandbox = Sandbox::new();
    let broken = Fake::new(|_, args| match args {
        ["template", "ls", ..] => out(&listing()),
        ["ls", "-q"] => out("mg-100-aa\nmg-300-cc\n"),
        ["rm", "--force", "mg-100-aa"] => Err("no daemon".to_string()),
        ["template", "rm", ..] => Err("no daemon".to_string()),
        ["df", "--json"] => out("{}"),
        _ => fail(1),
    });
    let (runtime, _) = machine(&sandbox, broken);
    let stopped = cli(&runtime, &["sandbox", "prune"]);
    assert_eq!(stopped.exit_code, 0, "{}", stopped.stderr);
    assert!(stopped.stdout.contains("warning: the sandbox mg-100-aa of an ended run was not removed; remove it with sbx rm --force mg-100-aa"), "{}", stopped.stdout);
    assert!(stopped.stdout.contains("warning: the sandbox mg-300-cc of an ended run was not removed"), "{}", stopped.stdout);
    assert!(stopped.stdout.contains(&format!("warning: the image {OLD} (docker.io/sbx/pi-image) was not removed (no daemon)")), "{}", stopped.stdout);

    let refused = Fake::new(|_, args| match args {
        ["template", "ls", ..] => out(&listing()),
        ["template", "rm", ..] => fail(1),
        _ => fail(1),
    });
    let (runtime, _) = machine(&sandbox, refused);
    let answer = cli(&runtime, &["sandbox", "prune"]);
    assert!(answer.stdout.contains(&format!("warning: the image {OLD} (docker.io/sbx/pi-image) was not removed (sbx template rm exited 1: nope)")), "{}", answer.stdout);

    // Without sbx, or with a listing that is not a list, there is nothing to remove.
    for listing in [Err("sbx missing".to_string()), fail(1), out("not json"), out("{}")] {
        let (runtime, process) = machine(&sandbox, Fake::new(move |_, _| listing.clone()));
        let answer = cli(&runtime, &["sandbox", "prune", "--dry-run"]);
        assert_eq!(answer.stdout, "would remove 0.0 MB (a dry run adds up the listed image sizes, which count shared layers once each)\n", "{}", answer.stdout);
        assert!(calls(&process).iter().all(|call| !call.contains(" rm ")));
    }

    // Records that cannot be read pin nothing, so no image is removed on a guess.
    let (runtime, process) = machine(&sandbox, Fake::new(|_, _| out(&listing())));
    std::fs::write(format!("{}/state/agent-kit/model-gateway/sbx-images.json", sandbox.text()), "[]").expect("records");
    let answer = cli(&runtime, &["sandbox", "prune"]);
    assert!(answer.stdout.contains("warning: sbx-images.json cannot be read (not a JSON object); no image is removed"), "{}", answer.stdout);
    assert!(calls(&process).iter().all(|call| !call.contains("template rm")), "{:?}", calls(&process));

    // The state home must be known.
    let homeless = Runtime { env: HashMap::new(), ..sandbox_runtime(&sandbox, None) };
    assert_eq!(cli(&homeless, &["sandbox", "prune"]).exit_code, 1);
}

#[test]
fn prune_takes_its_own_flags_and_the_other_sandbox_commands_do_not() {
    let sandbox = Sandbox::new();
    let (runtime, _) = machine(&sandbox, Fake::new(|_, _| fail(1)));
    let refused = cli(&runtime, &["sandbox", "prune", "--force"]);
    assert_eq!(refused.exit_code, 1);
    assert!(refused.stderr.contains("sandbox prune takes [--all] [--dry-run] [--json]"), "{}", refused.stderr);
    for flag in ["--all", "--dry-run"] {
        let refused = cli(&runtime, &["sandbox", "status", flag]);
        assert!(refused.stderr.contains("sandbox status takes [--json]"), "{}", refused.stderr);
    }
}

#[test]
fn a_df_that_cannot_be_read_gives_no_size() {
    for answer in [fail(1), Err("gone".to_string()), out("[]"), out(r#"{"summaries":{}}"#), out(r#"{"summaries":[{"type":"Images"}]}"#), out(r#"{"summaries":[{"type":"Images","size":-1}]}"#)] {
        assert_eq!(sbx_prune::images_bytes(&Fake::new(move |_, _| answer.clone())), None);
    }
    assert_eq!(sbx_prune::images_bytes(&Fake::new(|_, _| out(&df(7)))), Some(7));
}

#[test]
fn a_refresh_removes_the_images_it_superseded_and_a_failure_to_is_a_warning() {
    let hub = |_: &str, args: &[&str]| match args {
        ["-fsS", ..] => out(super::sandbox::HUB_TAGS),
        ["template", "ls", ..] => out(&listing()),
        ["template", "rm", ..] => fail(1),
        _ => fail(1),
    };
    let sandbox = Sandbox::new();
    let (runtime, process) = machine(&sandbox, Fake::new(hub));
    let refreshed = cli(&runtime, &["sandbox", "refresh"]);
    assert_eq!(refreshed.exit_code, 0, "{}", refreshed.stdout);
    // Every client now pins the same digest, so the gateway's other images are superseded.
    let removals: Vec<String> = calls(&process).into_iter().filter(|call| call.starts_with("sbx template rm")).collect();
    assert!(removals.contains(&format!("sbx template rm --force {OLD}")), "{removals:?}");
    assert!(refreshed.stdout.contains("warning: the image "), "{}", refreshed.stdout);
}

#[test]
fn a_launch_that_refreshes_a_stale_record_removes_the_superseded_images() {
    let hub = |_: &str, args: &[&str]| match args {
        ["-fsS", ..] => out(super::sandbox::HUB_TAGS),
        ["template", "ls", ..] => out(&listing()),
        _ => out(""),
    };
    let sandbox = Sandbox::new();
    let (runtime, process) = machine(&sandbox, Fake::new(hub));
    let (reference, warnings) = sbx_images::ensure(runtime.process.as_ref(), runtime.files.as_ref(), &FixedClock(NOW + 3 * 24 * 3_600_000), &runtime.env, "pi").expect("image");
    assert!(reference.starts_with("docker.io/sbx/pi-image@sha256:"));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(calls(&process).iter().any(|call| call.starts_with("sbx template rm --force ")), "{:?}", calls(&process));
}
