use super::support::{run_with, Tmp};
use crate::json::{self, Json};
use crate::litellm_tiers::{drift, fetch_upstream, render, run_with as run_tiers, validate};
use crate::skill_sources::Gh;

const ROUTING: &str = r#"{"other": 1, "tier_groups": {
  "tier-free": {"ceiling": "free", "key_models": ["free", "tier-free"],
    "steps": [{"model": "free/a", "cost": "free"}]},
  "tier-vibe": {"ceiling": "go", "key_models": ["tier-vibe", "vibe-tier"],
    "steps": [{"model": "vibe-tier/b", "cost": "vibe"}, {"tier": "tier-free"}]}
}}"#;

fn encode(text: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in text.as_bytes().chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
        // GitHub wraps its lines.
        if out.len() % 61 == 60 {
            out.push('\n');
        }
    }
    out
}

fn listing(routing: &str) -> String {
    format!(r#"{{"sha": "abc123", "content": {:?}}}"#, encode(routing))
}

struct Fake(Result<String, String>);

impl Gh for Fake {
    fn gh(&mut self, args: &[&str]) -> Result<String, String> {
        assert_eq!(args[0], "api");
        assert!(args[1].starts_with("repos/JorisJonkers-dev/litellm/contents/catalog/routing.json?ref="));
        self.0.clone()
    }
}

fn served(routing: &str) -> Fake {
    Fake(Ok(listing(routing)))
}

fn snapshot() -> Json {
    fetch_upstream(&mut served(ROUTING), "main").unwrap()
}

fn problems(text: &str) -> String {
    validate(&json::parse(text).unwrap()).join(" | ")
}

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

#[test]
fn a_good_snapshot_is_valid_and_each_fault_is_named() {
    assert_eq!(validate(&snapshot()), Vec::<String>::new());
    assert_eq!(problems("[]"), "not a JSON object");
    assert_eq!(problems(r#"{"source": {}}"#), "no tier_groups object");
    assert_eq!(problems(r#"{"tier_groups": {}}"#), "no source object | no tier_groups object");
    let broken = r#"{"source": 1, "tier_groups": {
      "nope": {}, "tier-x": 3,
      "tier-a": {"ceiling": "platinum", "key_models": ["tier-b"], "steps": []},
      "tier-b": {"key_models": ["tier-b", 4], "steps": [{"tier": "tier-gone"}, 5, {"tier": "tier-a", "model": "m"},
        {"model": "m", "cost": "dear"}, {"model": 1, "cost": "free"}, {"model": "m", "cost": "free"}, {"cost": "go", "model": "m"}]},
      "tier-c": {"ceiling": "go", "key_models": "tier-c", "steps": "none"}}}"#;
    let said = problems(broken);
    for fragment in [
        "no source object",
        "a tier is named tier-<name>",
        "tier tier-x: not an object",
        "tier tier-a: ceiling must be one of free, vibe, go",
        "tier tier-a: key_models must be a list of names that includes tier-a",
        "tier tier-a: no steps",
        "tier tier-b: ceiling must be",
        "tier tier-b: key_models must be a list",
        "tier tier-c: no steps",
    ] {
        assert!(said.contains(fragment), "{fragment} in {said}");
    }
    assert_eq!(said.matches("is neither a model with a cost class nor a known tier").count(), 5);
}

#[test]
fn a_fetch_reads_the_tier_groups_and_where_they_came_from() {
    let snap = snapshot();
    let source = snap.get("source").unwrap();
    assert_eq!(source.get("ref").and_then(Json::as_str), Some("main"));
    assert_eq!(source.get("blob").and_then(Json::as_str), Some("abc123"));
    assert!(matches!(snap.get("tier_groups"), Some(Json::Obj(groups)) if groups.len() == 2));
}

#[test]
fn a_fetch_that_cannot_be_read_says_why() {
    let said = |gh: &mut Fake| fetch_upstream(gh, "r").unwrap_err();
    assert!(said(&mut Fake(Err("HTTP 404".into()))).contains("gh could not fetch JorisJonkers-dev/litellm/catalog/routing.json at r: HTTP 404"));
    for (reply, why) in [
        ("not json".to_string(), "expected"),
        (r#"{"content": "e30="}"#.to_string(), "no sha"),
        (r#"{"sha": "s"}"#.to_string(), "no content"),
        (r#"{"sha": "s", "content": "!!"}"#.to_string(), "not base64"),
        (listing("{"), "has no readable tier_groups"),
        (listing(r#"{"nothing": 1}"#), "no tier_groups"),
        (listing(r#"{"tier_groups": {"tier-x": {}}}"#), "malformed tier_groups"),
    ] {
        let error = said(&mut Fake(Ok(reply)));
        assert!(error.contains(why), "{why} in {error}");
    }
}

#[test]
fn base64_lines_and_padding_are_read_whatever_their_length() {
    for text in ["", "a", "ab", "abc", "abcd", "~~~~~~", "??????", &"x".repeat(200)] {
        let reply = format!(r#"{{"sha": "s", "content": {:?}}}"#, encode(&format!(r#"{{"tier_groups": {{}}, "pad": {text:?}}}"#)));
        // The tiers are empty, so the fetch is refused as malformed: which proves the body was decoded and read.
        let error = fetch_upstream(&mut Fake(Ok(reply)), "r").unwrap_err();
        assert!(error.contains("malformed tier_groups: no tier_groups object"), "{text}: {error}");
    }
}

#[test]
fn the_snapshot_is_written_with_two_space_indent_in_the_order_it_came() {
    let text = render(&snapshot());
    assert!(text.starts_with("{\n  \"source\": {\n    \"repo\": \"JorisJonkers-dev/litellm\""), "{text}");
    assert!(text.contains("\"key_models\": [\n        \"free\",\n        \"tier-free\"\n      ]"), "{text}");
    assert!(text.ends_with("  }\n}\n"));
    assert_eq!(json::parse(&text).unwrap(), snapshot());
    let odd = json::parse(r#"{"a": [], "b": {}, "c": [null, true, false, 1, 2.5, "q\"\\"]}"#).unwrap();
    assert_eq!(render(&odd), "{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    null,\n    true,\n    false,\n    1,\n    2.5,\n    \"q\\\"\\\\\"\n  ]\n}\n");
}

#[test]
fn the_checked_in_snapshot_is_valid_and_written_the_way_render_writes_it() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../registry/litellm-tiers.json");
    let text = std::fs::read_to_string(path).unwrap();
    let parsed = json::parse(&text).unwrap();
    assert_eq!(validate(&parsed), Vec::<String>::new());
    assert_eq!(render(&parsed), text);
}

#[test]
fn drift_names_the_tiers_that_differ_or_are_missing_on_either_side() {
    let ours = snapshot();
    assert_eq!(drift(&ours, &ours), Vec::<String>::new());
    let changed = ROUTING.replace("free/a", "free/z");
    let theirs = fetch_upstream(&mut served(&changed), "main").unwrap();
    assert_eq!(drift(&ours, &theirs), ["tier-free"]);
    let fewer = fetch_upstream(&mut served(r#"{"tier_groups": {"tier-free": {"ceiling": "free", "key_models": ["tier-free"], "steps": [{"model": "free/a", "cost": "free"}]}}}"#), "main").unwrap();
    assert_eq!(drift(&ours, &fewer), ["tier-free", "tier-vibe"]);
    assert_eq!(drift(&Json::Null, &fewer), ["tier-free"]);
}

fn call(items: &[&str], gh: &mut Fake) -> (i32, String, String) {
    let items = args(items);
    let result = run_with(|io| run_tiers(&items, io, gh), "");
    (result.code, result.out, result.err)
}

#[test]
fn write_then_check_round_trips_and_check_finds_drift_upstream() {
    let tmp = Tmp::new();
    let path = tmp.join("tiers.json");
    let path = path.to_str().unwrap();
    let mut gh = served(ROUTING);
    let (code, out, _) = call(&["--write", "--ref=r1", "--path", path], &mut gh);
    assert_eq!((code, out.contains("written")), (0, true));
    assert!(std::fs::read_to_string(path).unwrap().contains("\"ref\": \"r1\""));
    assert_eq!(call(&["--check", "--path", path], &mut gh), (0, "tier snapshot ok\n".to_string(), String::new()));
    assert_eq!(call(&["--check", "--upstream", "r1", "--path", path], &mut gh).0, 0);
    let mut drifted = served(&ROUTING.replace("free/a", "free/z"));
    let (code, _, err) = call(&["--check", "--upstream=r1", "--path", path], &mut drifted);
    assert_eq!(code, 1);
    assert!(err.contains("differs from litellm r1: tier-free") && err.contains("--write --ref r1"), "{err}");
    let (code, _, err) = call(&["--check", "--upstream", "r1", "--path", path], &mut Fake(Err("HTTP 500".into())));
    assert_eq!((code, err.contains("HTTP 500")), (1, true));
}

#[test]
fn a_snapshot_that_cannot_be_read_or_written_or_is_malformed_fails() {
    let tmp = Tmp::new();
    let mut gh = served(ROUTING);
    let absent = tmp.join("absent.json");
    let (code, _, err) = call(&["--check", "--path", absent.to_str().unwrap()], &mut gh);
    assert_eq!((code, err.contains("cannot be read")), (1, true));
    let text = tmp.write("text.json", "nope");
    let (code, _, err) = call(&["--check", "--path", text.to_str().unwrap()], &mut gh);
    assert_eq!((code, err.contains("is not JSON")), (1, true));
    let bad = tmp.write("bad.json", "{}");
    let (code, _, err) = call(&["--check", "--path", bad.to_str().unwrap()], &mut gh);
    assert_eq!((code, err.contains("is malformed")), (1, true));
    let nowhere = tmp.join("missing-dir/out.json");
    let (code, _, err) = call(&["--write", "--path", nowhere.to_str().unwrap()], &mut gh);
    assert_eq!((code, err.contains("cannot write")), (1, true));
    let (code, _, err) = call(&["--write"], &mut Fake(Err("HTTP 404".into())));
    assert_eq!((code, err.contains("could not fetch")), (1, true));
}

#[test]
fn the_arguments_are_one_mode_and_known_options() {
    let mut gh = served(ROUTING);
    for items in [
        vec!["--bogus"],
        vec![],
        vec!["--write", "--check"],
        vec!["--check", "--ref"],
        vec!["--check", "--upstream"],
        vec!["--check", "--path"],
    ] {
        let (code, _, err) = call(&items, &mut gh);
        assert_eq!((code, err.starts_with("litellm-tiers: ")), (2, true), "{items:?}: {err}");
    }
}

#[test]
fn the_dispatcher_runs_the_subcommand() {
    let result = super::support::run(&["litellm-tiers", "--bogus"], "");
    assert_eq!(result.code, 2);
    assert!(result.err.contains("unknown argument --bogus"));
}
