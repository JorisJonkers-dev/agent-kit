//! `grant reserve` against the real binary, many processes at once: the decision
//! and the reservation line are one locked ledger update, so parallel nodes
//! never both pass on a grant's last start.

use model_gateway::json::{parse, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const WORKFLOW: &str = "gold-band-race";

/// A scratch machine: its own home and state, gone when it drops.
struct Machine {
    root: PathBuf,
}

impl Machine {
    fn new() -> Machine {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("model-gateway-race-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(root.join("home")).expect("home");
        Machine { root: std::fs::canonicalize(&root).expect("canonical root") }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_model-gateway"));
        command
            .args(args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.root.join("home"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .current_dir(&self.root);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("the gateway runs")
    }

    /// A grant of `max_starts` node starts for the race's workflow.
    fn grant(&self, max_starts: u32) {
        let input = format!(r#"{{"workflow":"{WORKFLOW}","client":"codex","cap_usd":5,"question":"q","answer":"a","max_starts":{max_starts}}}"#);
        let written = self.run(&["grant", "write", "--json", &input]);
        assert!(written.status.success(), "{}", String::from_utf8_lossy(&written.stderr));
    }

    /// One `grant reserve` per node name, all started before any is waited for.
    fn reserve_together(&self, nodes: &[String]) -> Vec<(String, Value)> {
        let children: Vec<(String, std::process::Child)> = nodes
            .iter()
            .map(|node| {
                let input = format!(r#"{{"workflow":"{WORKFLOW}","client":"codex","node":"{node}"}}"#);
                let child = self
                    .command(&["grant", "reserve", "--json", &input])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .expect("the gateway starts");
                (node.clone(), child)
            })
            .collect();
        children
            .into_iter()
            .map(|(node, child)| {
                let output = child.wait_with_output().expect("the gateway ends");
                let value = parse(String::from_utf8_lossy(&output.stdout).trim()).expect("one JSON object");
                (node, value)
            })
            .collect()
    }

    /// The reservation lines of node starts in the ledger.
    fn reservation_lines(&self) -> usize {
        let ledger = self.root.join("state").join("agent-kit").join("model-gateway").join("ledger.jsonl");
        std::fs::read_to_string(ledger).unwrap_or_default().lines().filter(|line| line.contains("\"kind\":\"reservation\"") && line.contains("\"model\":\"node-start\"")).count()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(Path::new(&self.root));
    }
}

fn member<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|object| object.get(key))
}

fn status_of(value: &Value) -> &str {
    member(value, "status").and_then(Value::as_str).unwrap_or_default()
}

fn code_of(value: &Value) -> &str {
    member(value, "code").and_then(Value::as_str).unwrap_or_default()
}

fn repeated(value: &Value) -> bool {
    member(member(value, "reservation").expect("a reservation"), "repeated") == Some(&Value::Bool(true))
}

fn nodes(count: usize) -> Vec<String> {
    (1..=count).map(|index| format!("codex-{index}")).collect()
}

#[test]
fn exactly_one_of_eight_parallel_starts_passes_on_a_grant_of_one() {
    for _ in 0..5 {
        let machine = Machine::new();
        machine.grant(1);
        let answers = machine.reserve_together(&nodes(8));
        assert_eq!(answers.iter().filter(|(_, answer)| status_of(answer) == "ok").count(), 1, "{answers:?}");
        assert_eq!(answers.iter().filter(|(_, answer)| code_of(answer) == "cap-exceeded").count(), 7, "{answers:?}");
        assert_eq!(machine.reservation_lines(), 1);
    }
}

#[test]
fn exactly_three_of_eight_parallel_starts_pass_on_a_grant_of_three() {
    for _ in 0..5 {
        let machine = Machine::new();
        machine.grant(3);
        let answers = machine.reserve_together(&nodes(8));
        assert_eq!(answers.iter().filter(|(_, answer)| status_of(answer) == "ok").count(), 3, "{answers:?}");
        assert_eq!(answers.iter().filter(|(_, answer)| code_of(answer) == "cap-exceeded").count(), 5, "{answers:?}");
        assert_eq!(machine.reservation_lines(), 3);
    }
}

#[test]
fn eight_parallel_asks_for_one_node_and_attempt_hold_one_start() {
    let machine = Machine::new();
    machine.grant(1);
    let answers = machine.reserve_together(&vec!["codex-1".to_string(); 8]);
    assert!(answers.iter().all(|(_, answer)| status_of(answer) == "ok"), "{answers:?}");
    assert_eq!(answers.iter().filter(|(_, answer)| !repeated(answer)).count(), 1, "{answers:?}");
    let run_ids: std::collections::BTreeSet<&str> = answers
        .iter()
        .filter_map(|(_, answer)| member(member(answer, "reservation")?, "run_id")?.as_str())
        .collect();
    assert_eq!(run_ids.len(), 1, "every repeat names the one start: {answers:?}");
    assert_eq!(machine.reservation_lines(), 1);
}

#[test]
fn parallel_repeats_never_take_a_start_from_another_node() {
    // Four nodes ask three times each for two starts: a node that won a start is answered every time, one that lost never is.
    for _ in 0..5 {
        let machine = Machine::new();
        machine.grant(2);
        let asked: Vec<String> = nodes(4).into_iter().flat_map(|node| vec![node; 3]).collect();
        let answers = machine.reserve_together(&asked);
        let mut by_node: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (node, answer) in &answers {
            by_node.entry(node.as_str()).or_default().push(status_of(answer));
        }
        let winners = by_node.values().filter(|statuses| statuses.iter().all(|status| *status == "ok")).count();
        let losers = by_node.values().filter(|statuses| statuses.iter().all(|status| *status == "error")).count();
        assert_eq!((winners, losers), (2, 2), "{by_node:?}");
        assert_eq!(machine.reservation_lines(), 2);
    }
}
