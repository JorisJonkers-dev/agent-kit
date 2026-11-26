//! The spec critique on the tiers, end to end: `critique tier-review` through the command line, with stubbed harnesses. The
//! stubs are real programs and real sockets: a `pi` that calls the proxy the way Pi would, an `agent-launcher` that runs what
//! it confines, and a LiteLLM on 127.0.0.1 that answers with the headers a real one would, so a reviewer that LiteLLM
//! served from another model shows up exactly as it would live. The forwarding is the real `curl`.

use super::scenarios::{run_git, shipped_folder, FixedClock, Sandbox, EXEC_LOCK, FIXED_NOW};
use crate::cli::{run_cli, CliResult};
use crate::git::SystemGit;
use crate::Runtime;
use model_gateway::adapters::fs::SystemFiles;
use model_gateway::json::{self, parse, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

const SPEC: &str = "# Spec\n\nThe retry budget is unbounded.\n";

/// The `pi` the harness stubs: signed in, and a reviewer that calls the proxy for its model and answers with one finding.
const PI: &str = r#"#!/bin/sh
case "$*" in
  auth*) echo '{"status":"ready"}'; exit 0;;
esac
cat > /dev/null
model=""
while [ $# -gt 0 ]; do
  if [ "$1" = "--model" ]; then model=$2; fi
  shift
done
base=$(sed -n 's/.*"baseUrl": "\([^"]*\)".*/\1/p' "$PI_CODING_AGENT_DIR/models.json")
key=$(sed -n 's/.*"apiKey": "\([^"]*\)".*/\1/p' "$PI_CODING_AGENT_DIR/models.json")
printf '%s\n' "$model" >> "@SANDBOX@/asked.txt"
curl -s -o /dev/null -X POST "$base/chat/completions" -H "Authorization: Bearer $key" -H "Content-Type: application/json" -d "{\"model\":\"$model\"}"
if [ -f "@SANDBOX@/limited" ] && grep -qx "$model" "@SANDBOX@/limited"; then
  printf '%s\n' '{"message":{"content":[],"role":"assistant","stopReason":"error","errorMessage":"429 rate limit exceeded"},"type":"message_end"}'
  exit 0
fi
printf '%s\n' '{"cwd":"/p","id":"s","type":"session","version":3}'
printf '%s\n' '{"type":"agent_start"}'
printf '{"message":{"content":[{"text":"Review.\\n```json\\n{\\"findings\\":[{\\"category\\":\\"failure-mode\\",\\"check\\":\\"dependency-fails\\",\\"claim\\":\\"finding from %s\\",\\"evidence\\":[{\\"kind\\":\\"quote\\",\\"value\\":\\"The retry budget is unbounded.\\"}],\\"id\\":\\"a\\",\\"location\\":\\"artifact/spec.md\\",\\"severity\\":\\"major\\"}]}\\n```","type":"text"}],"role":"assistant","stopReason":"stop","usage":{"cacheRead":0,"cost":{"total":0},"input":10,"output":5}},"type":"message_end"}\n' "$model"
"#;

const LAUNCHER: &str = r#"#!/bin/sh
case " $* " in *" --check "*) exit 0;; esac
while [ "$1" != "--" ]; do shift; done
shift
exec "$@"
"#;

/// A LiteLLM on 127.0.0.1: it answers a request for a model with LiteLLM's headers, and a request for one of `falling`
/// with a fallback to another model.
fn litellm(falling: &[&str]) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    let falling: Vec<String> = falling.iter().map(|model| model.to_string()).collect();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let falling = falling.clone();
            std::thread::spawn(move || answer(stream, &falling));
        }
    });
    port
}

fn answer(mut stream: TcpStream, falling: &[String]) {
    let mut bytes: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|value| value.trim().parse::<usize>().ok())).unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
        }
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let asked = text.split("\"model\":\"").nth(1).and_then(|rest| rest.split('"').next()).unwrap_or("").to_string();
    let fell = falling.contains(&asked);
    let (served, fallbacks) = if fell { ("free/another-model".to_string(), 1) } else { (asked.clone(), 0) };
    let body = format!("{{\"model\":\"{served}\"}}");
    let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nx-litellm-model-id: deployment-{asked}\r\nx-litellm-attempted-fallbacks: {fallbacks}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(reply.as_bytes());
}

/// One sandboxed machine: a repository on GitHub, the family allowlist, the tier's Pi provider and the stubbed programs.
struct Machine {
    sandbox: Sandbox,
}

struct Setup<'a> {
    families: &'a [&'a str],
    public: bool,
    remote: &'a str,
    falling: &'a [&'a str],
    /// The `pi` program.
    pi: &'a str,
}

impl Setup<'_> {
    const ALL: [&'static str; 8] = ["nvidia", "google", "zhipu", "openai", "deepseek", "alibaba", "minimax", "xiaomi"];
}

fn setup() -> Setup<'static> {
    Setup { families: &Setup::ALL, public: true, remote: "git@github.com:JorisJonkers-dev/example.git", falling: &[], pi: PI }
}

impl Machine {
    fn new(setup: &Setup<'_>) -> Machine {
        let sandbox = Sandbox::new();
        let root = sandbox.path.clone();
        let write = |path: &str, text: &str| {
            let target = root.join(path);
            std::fs::create_dir_all(target.parent().expect("parent")).expect("parent dir");
            std::fs::write(target, text).expect("write");
        };
        let _held = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        write("work/.keep", "");
        write("repos/k1/docs/agents/issue-tracker.md", "# Issue tracker: Local Markdown\n\nTickets live in .scratch.\n");
        write("repos/k1/docs/spec.md", SPEC);
        let git = |args: &[&str]| run_git(&root, &args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
        git(&["init", "-q", "--template=", "repos/k1"]);
        git(&["-C", "repos/k1", "remote", "add", "origin", setup.remote]);
        git(&["-C", "repos/k1", "add", "-A"]);
        git(&["-C", "repos/k1", "commit", "-q", "-m", "base #12"]);
        let families: Vec<String> = setup.families.iter().map(|family| format!("\"{family}\"")).collect();
        write(
            "home/.config/agent-kit/critique-allowlist.json",
            &format!("{{\"repositories\":{{\"github.com/jorisjonkers-dev/example\":{{\"code\":false,\"families\":[{}],\"history\":false,\"updated_at\":\"2026-11-01T00:00:00.000Z\"}}}},\"version\":1}}\n", families.join(",")),
        );
        // What GitHub says about the repository's visibility, through the gh the runner asks.
        write("bin/gh", &format!("#!/bin/sh\necho {}\n", if setup.public { "public" } else { "private" }));
        let port = litellm(setup.falling);
        write(
            "home/.pi/agent/models.json",
            &format!("{{\"providers\":{{\"litellm-tier-review\":{{\"api\":\"openai-completions\",\"apiKey\":\"sk-tier-review\",\"baseUrl\":\"http://127.0.0.1:{port}/v1\",\"models\":[{{\"id\":\"tier-review\"}}]}}}}}}\n"),
        );
        write("bin/pi", &setup.pi.replace("@SANDBOX@", &sandbox.text()));
        write("bin/agent-launcher", LAUNCHER);
        for program in ["pi", "agent-launcher", "gh"] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.join("bin").join(program), std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        Machine { sandbox }
    }

    /// The step models the stubbed `pi` was asked for, in order.
    fn asked(&self) -> Vec<String> {
        std::fs::read_to_string(self.sandbox.path.join("asked.txt")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    fn put(&self, path: &str, text: &str) {
        let target = self.sandbox.path.join(path);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("parent dir");
        std::fs::write(target, text).expect("write");
    }

    fn runtime(&self) -> Runtime {
        let sandbox = &self.sandbox.path;
        let path = format!("{}:{}", sandbox.join("bin").to_string_lossy(), std::env::var("PATH").unwrap_or_default());
        let mut env: HashMap<String, String> = HashMap::new();
        env.insert("PATH".into(), path);
        env.insert("HOME".into(), sandbox.join("home").to_string_lossy().into_owned());
        env.insert("XDG_STATE_HOME".into(), sandbox.join("state").to_string_lossy().into_owned());
        let cwd = sandbox.join("work").to_string_lossy().into_owned();
        Runtime {
            gateway: model_gateway::ports::Runtime {
                clock: Box::new(FixedClock(FIXED_NOW)),
                files: Box::new(SystemFiles { cwd: cwd.clone() }),
                lock: model_gateway::ports::LockTimings::default(),
                process: Box::new(model_gateway::adapters::process::SystemProcess { env: env.clone() }),
                git: Box::new(model_gateway::adapters::git::SystemGit { env: env.clone(), timeout_ms: model_gateway::adapters::git::TIMEOUT_MS }),
                env: env.clone(),
                defaults: model_gateway::SHIPPED_DEFAULTS.to_string(),
                defaults_path: "defaults.json".to_string(),
                routing: model_gateway::SHIPPED_ROUTING.to_string(),
            },
            clock: Box::new(FixedClock(FIXED_NOW)),
            files: Box::new(SystemFiles { cwd }),
            dirs: Box::new(crate::dirs::SystemDirs),
            git: Box::new(SystemGit { env: env.clone() }),
            gh: Box::new(crate::gh::SystemGh { env: env.clone() }),
            env,
            assets_root: shipped_folder(),
            stdin: Box::new(String::new),
            search: None,
        }
    }

    fn tier_review(&self, extra: &str) -> (CliResult, Value) {
        let input = format!("{{\"repo\":\"{}/repos/k1\",\"spec\":\"docs/spec.md\"{extra}}}", self.sandbox.text());
        let _held = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = run_cli(&["tier-review".to_string(), "--json".to_string(), input], &self.runtime());
        let value = parse(&result.stdout).expect("the command prints JSON");
        (result, value)
    }

    fn ledger(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.sandbox.path.join("state/agent-kit/model-gateway/ledger.jsonl")).unwrap_or_default();
        text.lines().filter_map(|line| parse(line).ok()).collect()
    }
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("no {key} in {value:?}"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    field(value, key).as_str().unwrap_or_else(|| panic!("{key} is not text"))
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Value::Arr(items) => items,
        other => panic!("{key} is {other:?}"),
    }
}

fn families(value: &Value) -> Vec<&str> {
    list(value, "families").iter().filter_map(Value::as_str).collect()
}

#[test]
fn one_command_resolves_three_families_without_the_writers_and_writes_the_synthesis_and_a_routing_record_per_reviewer() {
    let machine = Machine::new(&setup());
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (0, "reviewed"), "{}", result.stdout);
    // Free first, in ladder order: Nemotron, Gemma, GLM-4.7 Flash; never the writer's own family.
    assert_eq!(families(&value), ["nvidia", "google", "zhipu"]);
    let reviewers = list(&value, "reviewers");
    let asked: Vec<&str> = reviewers.iter().map(|reviewer| text(field(reviewer, "routing"), "requested_model")).collect();
    assert_eq!(asked, ["free/nemotron-3-ultra-550b-a55b", "free/gemma-4-31b-it", "free/glm-4.7-flash"]);
    for reviewer in reviewers {
        let routing = field(reviewer, "routing");
        assert_eq!((text(routing, "tier"), text(routing, "routing_capture"), text(routing, "served_model")), ("tier-review", "recorded", text(routing, "requested_model")));
        assert_eq!(field(routing, "fallback_fired"), &Value::Bool(false));
        assert!(text(routing, "litellm_model_id").starts_with("deployment-free/"), "{routing:?}");
    }
    // The ledger holds a routing record for each reviewer, under the tier's gateway client and with the step model it asked for.
    let lines: Vec<Value> = machine.ledger().into_iter().filter(|line| line.as_object().and_then(|line| line.get("run_id")).is_some() && line.as_object().and_then(|line| line.get("tier")).is_some()).collect();
    assert_eq!(lines.len(), 3);
    for line in &lines {
        assert_eq!((text(line, "client"), text(line, "harness"), text(line, "tier"), text(line, "routing_capture")), ("pi-tier-review", "pi", "tier-review", "recorded"));
        assert_eq!(field(line, "attempted_fallbacks"), &Value::Num(0.0));
        assert!(text(line, "model").starts_with("free/"), "{line:?}");
    }
    let synthesis = std::fs::read_to_string(text(&value, "synthesis_path")).expect("the synthesis");
    assert!(synthesis.contains("| c1-free-nemotron-3-ultra-550b-a55b | nvidia |") && synthesis.contains("finding from free/nemotron-3-ultra-550b-a55b"), "{synthesis}");
    // The three reviewers raised the one finding, and the synthesis says who from which family.
    assert!(synthesis.contains("Raised by c1-free-nemotron-3-ultra-550b-a55b (nvidia), c2-free-gemma-4-31b-it (google), c3-free-glm-4.7-flash (zhipu).") && synthesis.contains("## For the coordinator"), "{synthesis}");
}

#[test]
fn a_reviewer_litellm_served_from_another_model_is_retried_through_a_family_not_yet_in_the_review() {
    let machine = Machine::new(&Setup { falling: &["free/gemma-4-31b-it"], ..setup() });
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (0, "reviewed"), "{}", result.stdout);
    // Gemma fell back inside LiteLLM, so it is no review; the retry is the next family of the ladder, GPT-OSS, not a second zhipu.
    assert_eq!(families(&value), ["nvidia", "zhipu", "openai"]);
    let reviewers = list(&value, "reviewers");
    let collapsed = reviewers.iter().find(|reviewer| text(reviewer, "critic_id") == "c2-free-gemma-4-31b-it").expect("gemma's reviewer");
    assert_eq!((text(collapsed, "status"), text(collapsed, "failure")), ("failed", "routing-collapsed"));
    assert_eq!(field(field(collapsed, "routing"), "fallback_fired"), &Value::Bool(true));
    let retry = reviewers.iter().find(|reviewer| reviewer.as_object().and_then(|object| object.get("retry_of")).and_then(Value::as_str) == Some("c2-free-gemma-4-31b-it")).expect("the retry");
    assert_eq!(text(retry, "family"), "openai");
}

#[test]
fn a_review_whose_reviewers_span_fewer_than_three_families_fails_and_says_so() {
    // A private repository allows three families and nothing is left to retry through, so a collapsed reviewer
    // leaves two. A public one is not held to its entry, so only a private one can be short of families.
    let machine = Machine::new(&Setup { families: &["deepseek", "zhipu", "alibaba"], public: false, falling: &["vibe-tier/zai-glm-5-3-flash"], ..setup() });
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (3, "failed"), "{}", result.stdout);
    assert_eq!(families(&value), ["deepseek", "alibaba"]);
    assert!(text(&value, "message").contains("only 2 of 3 families reviewed"), "{value:?}");
    assert!(std::path::Path::new(text(&value, "synthesis_path")).exists());
}

#[test]
fn a_private_repository_never_sees_a_free_step_and_the_ladder_goes_on_to_vibe_and_go() {
    let machine = Machine::new(&Setup { public: false, ..setup() });
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (0, "reviewed"), "{}", result.stdout);
    assert_eq!(families(&value), ["deepseek", "zhipu", "alibaba"]);
    let asked: Vec<&str> = list(&value, "reviewers").iter().map(|reviewer| text(field(reviewer, "routing"), "requested_model")).collect();
    assert_eq!(asked, ["vibe-tier/deepseek-v4-1-flash-1017", "vibe-tier/zai-glm-5-3-flash", "opencode-go-tier/qwen3.8-flash"]);
}

#[test]
fn a_step_the_ledger_holds_as_rate_limited_is_left_for_the_next_family() {
    let machine = Machine::new(&setup());
    let limited = "{\"at\":\"2026-11-03T11:00:00.000Z\",\"client\":\"pi-tier-review\",\"cost_class\":\"subsidised\",\"cost_usd\":0,\"model\":\"free/nemotron-3-ultra-550b-a55b\",\"outcome\":\"failed\",\"reason\":\"rate-limited\",\"run_id\":\"earlier\",\"workflow\":\"w\"}\n";
    machine.put("state/agent-kit/model-gateway/ledger.jsonl", limited);
    let (_, value) = machine.tier_review("");
    assert_eq!(families(&value), ["google", "zhipu", "openai"]);
}

#[test]
fn a_run_that_cannot_have_three_families_is_refused_with_what_was_passed_over() {
    let machine = Machine::new(&Setup { families: &["deepseek", "zhipu"], public: false, ..setup() });
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (0, "refused"), "{}", result.stdout);
    let message = text(&value, "message");
    assert!(message.starts_with("tier-review resolved 2 of 3 families that are neither anthropic nor barred; passed over: "), "{message}");
    // A free step is barred because the repository is not public, and a paid one because its family is not allowed.
    assert!(message.contains("free-glm-4.7-flash (data-scope)") && message.contains("opencode-go-tier-qwen3.8-flash (family-not-allowed)"), "{message}");
}

#[test]
fn a_tier_whose_key_is_not_ready_is_refused_before_anything_is_frozen() {
    let machine = Machine::new(&Setup { pi: "#!/bin/sh\necho '{\"status\":\"not_ready\",\"reason\":\"no key\"}'\n", ..setup() });
    let (_, value) = machine.tier_review("");
    assert_eq!(text(&value, "status"), "refused");
    assert!(text(&value, "message").contains("pi: the litellm-tier-review provider is not_ready (no key)"), "{value:?}");
}

#[test]
fn the_roster_is_the_tiers_to_pick_and_a_nedap_repository_is_refused() {
    let machine = Machine::new(&setup());
    let (result, value) = machine.tier_review(",\"roster\":\"free\"");
    assert_eq!((result.exit_code, text(&value, "code")), (1, "invalid-input"));
    let (_, value) = machine.tier_review(",\"stage\":\"tickets\"");
    assert_eq!(text(&value, "code"), "invalid-input");
    // GitHub says it is public, and it is still refused: a nedap repository sends nothing, whatever its visibility.
    let work = Machine::new(&Setup { remote: "git@github.com:nedap/example.git", ..setup() });
    let (_, value) = work.tier_review("");
    assert_eq!(text(&value, "status"), "refused");
    assert_eq!(text(&value, "reason"), "nedap", "{value:?}");
    assert!(text(&value, "message").contains("nothing is sent to any critic"), "{value:?}");
}

#[test]
fn a_public_repository_is_reviewed_without_an_allowlist_entry() {
    let machine = Machine::new(&setup());
    std::fs::remove_file(machine.sandbox.path.join("home/.config/agent-kit/critique-allowlist.json")).expect("no allowlist");
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (0, "reviewed"), "{}", result.stdout);
}

#[test]
fn a_repository_whose_visibility_cannot_be_read_is_private_and_needs_its_entry() {
    let machine = Machine::new(&setup());
    machine.put("bin/gh", "#!/bin/sh\necho 'gh: Not Found (HTTP 404)' >&2\nexit 1\n");
    // With its entry it is held to it like a private repository: no free step reviews it.
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (0, "reviewed"), "{}", result.stdout);
    let asked: Vec<&str> = list(&value, "reviewers").iter().map(|reviewer| text(field(reviewer, "routing"), "requested_model")).collect();
    assert!(asked.iter().all(|model| !model.starts_with("free/")), "{asked:?}");
    // Without one it is refused.
    std::fs::remove_file(machine.sandbox.path.join("home/.config/agent-kit/critique-allowlist.json")).expect("no allowlist");
    let (_, value) = machine.tier_review("");
    assert_eq!((text(&value, "status"), text(&value, "reason")), ("refused", "not-opted-in"), "{value:?}");
}

#[test]
fn a_stage_that_routes_no_critics_gives_the_reviewers_no_roles() {
    let machine = Machine::new(&setup());
    machine.put("home/.config/agent-kit/critique-routing.json", "{\"stages\":{\"spec\":{\"base\":[],\"deep\":[]}}}\n");
    let (_, value) = machine.tier_review("");
    assert_eq!(text(&value, "status"), "refused");
    assert!(text(&value, "message").contains("routing names no critics"), "{value:?}");
}

#[test]
fn a_rate_limited_reviewer_is_retried_through_another_family_and_its_step_is_cooled_for_the_next_run() {
    let machine = Machine::new(&setup());
    machine.put("limited", "free/gemma-4-31b-it\n");
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (0, "reviewed"), "{}", result.stdout);
    assert_eq!(families(&value), ["nvidia", "zhipu", "openai"]);
    let limited = list(&value, "reviewers").iter().find(|reviewer| text(reviewer, "critic_id") == "c2-free-gemma-4-31b-it").expect("gemma's reviewer");
    assert_eq!(text(limited, "failure"), "rate-limited");
    let line = machine.ledger().into_iter().find(|line| line.as_object().and_then(|line| line.get("reason")).and_then(Value::as_str) == Some("rate-limited")).expect("the refusal is in the ledger");
    assert_eq!((text(&line, "client"), text(&line, "model")), ("pi-tier-review", "free/gemma-4-31b-it"));
    // The next run does not ask gemma at all: its refusal stands, and the roster says so.
    let asked_before = machine.asked().len();
    let (_, again) = machine.tier_review("");
    assert_eq!(text(&again, "status"), "reviewed");
    assert!(!machine.asked()[asked_before..].contains(&"free/gemma-4-31b-it".to_string()));
    let dropped: Vec<(&str, &str)> = list(&again, "dropped").iter().map(|dropped| (text(dropped, "client"), text(dropped, "reason"))).collect();
    assert_eq!(dropped, [("free-gemma-4-31b-it", "unavailable"), ("vibe-tier-zai-glm-5-3-flash", "same-family")]);
}

#[test]
fn a_reviewer_whose_proxy_cannot_be_set_up_is_no_review_because_its_family_cannot_be_proven() {
    // The key is a secret command nobody answers: the proxy stands aside, and the confined launch then cannot read it either.
    let machine = Machine::new(&setup());
    machine.put("home/.pi/agent/models.json", "{\"providers\":{\"litellm-tier-review\":{\"api\":\"openai-completions\",\"apiKey\":\"!agent-kit-secret get NOPE\",\"baseUrl\":\"http://127.0.0.1:9/v1\"}}}\n");
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (3, "failed"), "{}", result.stdout);
    let first = &list(&value, "reviewers")[0];
    assert_eq!((text(first, "status"), text(first, "failure")), ("failed", "unconfined"));
    assert_eq!(text(field(first, "routing"), "routing_capture"), "hidden");
    assert!(families(&value).is_empty());
}

#[test]
fn a_reviewer_that_cannot_be_confined_is_no_review_and_its_routing_says_nothing_was_asked() {
    use crate::clients::{review_with, CriticLaunch};
    use crate::critic_tools::CriticToolset;
    use crate::roster::RosterCritic;
    use crate::routing::{CriticClient, PiCriticClient};
    let machine = Machine::new(&setup());
    std::fs::remove_file(machine.sandbox.path.join("bin/agent-launcher")).expect("remove the launcher");
    let config = PiCriticClient { binary: "pi".into(), provider: "litellm-tier-review".into(), model: "free/gemma-4-31b-it".into(), family: "google".into(), data_scope: "public".into(), free: true, tier: Some("tier-review".into()) };
    let critic = RosterCritic { client: "c".into(), role: "spec-redteam".into(), lenses: Vec::new(), max_price_usd: 1.0, time_limit_s: 30.0, critic_id: "c1".into(), client_config: CriticClient::Pi(config), fallbacks: Vec::new() };
    machine.put("critic/prompt.md", "review");
    let root = machine.sandbox.text();
    let launch = CriticLaunch { critic: &critic, packet_dir: format!("{root}/critic/packet"), prompt_path: format!("{root}/critic/prompt.md"), critic_dir: format!("{root}/critic"), schema_path: String::new(), toolset: CriticToolset::Pi };
    let _held = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // The launcher is gone from the machine, and the host's own (which setup installs in ~/.local/bin) must not be
    // found in its place: this launch looks for it on the sandbox's PATH alone, and nothing ever starts anyway.
    let mut runtime = machine.runtime();
    runtime.env.insert("PATH".to_string(), format!("{}/bin", machine.sandbox.text()));
    let report = review_with(&runtime, &launch).expect("a report");
    match report.outcome {
        crate::clients::CriticOutcome::Failed { failure, .. } => assert_eq!(failure, "unconfined"),
        other => panic!("{other:?}"),
    }
    assert_eq!(report.routing.expect("a routing").capture, "no-request");
}

#[test]
fn a_finding_names_each_reviewers_family_and_says_its_suggestion_and_pre_verdict_when_it_has_them() {
    use crate::tier_review_command::finding_lines;
    let critics = vec![json::object(vec![("critic_id", Value::str("c1")), ("family", Value::str("nvidia"))])];
    let with = json::object(vec![
        ("id", Value::str("f1")),
        ("severity", Value::str("major")),
        ("location", Value::str("spec.md")),
        ("claim", Value::str("the claim")),
        ("raised_by", Value::Arr(vec![Value::str("c1"), Value::str("gone")])),
        ("suggestion", Value::str("fix it")),
        ("pre_verdict", json::object(vec![("verdict", Value::str("kept")), ("reason_code", Value::str("rc")), ("reason", Value::str("because"))])),
    ]);
    let lines = finding_lines(with.as_object().expect("finding"), &critics);
    assert!(lines.contains(&"Raised by c1 (nvidia), gone (?).".to_string()), "{lines:?}");
    assert!(lines.contains(&"Pre-verdict: kept (rc): because".to_string()), "{lines:?}");
    assert!(lines.contains(&"Suggestion: fix it".to_string()), "{lines:?}");
    // Without raised_by, suggestion or pre_verdict the finding still reads, with no reviewers named.
    let bare = json::object(vec![("id", Value::str("f2")), ("claim", Value::str("bare"))]);
    let lines = finding_lines(bare.as_object().expect("finding"), &critics);
    assert!(lines.contains(&"Raised by .".to_string()), "{lines:?}");
    assert!(!lines.iter().any(|line| line.starts_with("Suggestion:") || line.starts_with("Pre-verdict:")), "{lines:?}");
}

#[test]
fn a_family_that_reviewed_twice_is_named_once_and_a_failed_reviewer_is_not_a_family_that_reviewed() {
    use crate::tier_review_command::families_that_reviewed;
    let critic = |family: &str, status: &str| json::object(vec![("family", Value::str(family)), ("status", Value::str(status))]);
    let critics = vec![critic("nvidia", "ok"), critic("zhipu", "failed"), critic("nvidia", "ok"), critic("openai", "ok")];
    assert_eq!(families_that_reviewed(&critics), ["nvidia", "openai"]);
}

#[test]
fn a_ledger_refusal_of_another_client_does_not_cool_a_step_for_the_tier() {
    let machine = Machine::new(&setup());
    let other = "{\"at\":\"2026-11-03T11:00:00.000Z\",\"client\":\"pi-other\",\"cost_class\":\"subsidised\",\"cost_usd\":0,\"model\":\"free/gemma-4-31b-it\",\"outcome\":\"failed\",\"reason\":\"rate-limited\",\"run_id\":\"earlier\",\"workflow\":\"w\"}\n";
    machine.put("state/agent-kit/model-gateway/ledger.jsonl", other);
    let (result, value) = machine.tier_review("");
    assert_eq!((result.exit_code, text(&value, "status")), (0, "reviewed"), "{}", result.stdout);
    assert!(machine.asked().contains(&"free/gemma-4-31b-it".to_string()), "gemma was asked although another client refused it");
}

#[test]
fn an_orchestrator_family_the_caller_names_is_kept_and_the_review_still_runs() {
    let machine = Machine::new(&setup());
    let (result, value) = machine.tier_review(",\"orchestrator_family\":\"anthropic\"");
    assert_eq!((result.exit_code, text(&value, "status")), (0, "reviewed"), "{}", result.stdout);
}

#[test]
fn a_ledger_that_cannot_be_read_stops_the_review_before_any_run_is_frozen() {
    let machine = Machine::new(&setup());
    // A directory where the ledger file should be: the ledger cannot be read, so the cooled steps cannot be known.
    std::fs::create_dir_all(machine.sandbox.path.join("state/agent-kit/model-gateway/ledger.jsonl")).expect("the directory");
    let (result, value) = machine.tier_review("");
    assert_ne!(result.exit_code, 0, "{}", result.stdout);
    assert_eq!(text(&value, "code"), "unexpected", "{value:?}");
}

#[test]
fn a_string_field_given_as_a_number_is_an_invalid_input_whichever_field_it_is() {
    let machine = Machine::new(&setup());
    for field in ["head", "base", "decision_summary", "feature", "flow", "spec", "plan", "ticket"] {
        let (result, value) = machine.tier_review(&format!(",\"{field}\":5"));
        assert_eq!((result.exit_code, text(&value, "code")), (1, "invalid-input"), "{field}: {}", result.stdout);
    }
}
