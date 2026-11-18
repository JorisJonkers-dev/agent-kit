//! `litellm-tiers`: the snapshot of litellm's tier policy.
//!
//! The tier policy has one source, `catalog/routing.json` key `tier_groups` in
//! JorisJonkers-dev/litellm: each tier (`tier-free`, `tier-vibe`, ...) is a callable model
//! name on the proxy with a `ceiling`, `key_models` and ordered `steps`. agent-kit defines
//! no tier of its own: `registry/litellm-tiers.json` is that key copied verbatim, with where
//! it came from, and `render_registry.py` renders the delegation profiles, key lists and Pi
//! providers from it.
//!
//! ```text
//! ci-helpers litellm-tiers --write [--ref REF] [--path FILE]
//! ci-helpers litellm-tiers --check [--upstream REF] [--path FILE]
//! ```
//!
//! `--write` re-snapshots from litellm at REF (default `main`). `--check` exits 1 when the
//! snapshot is malformed, and with `--upstream REF` also when litellm's `tier_groups` at REF
//! differ from it. The fetch goes through `gh api`.

use crate::json::{self, Json};
use crate::skill_sources::{Gh, Subprocess};
use crate::{option_value, Io};

const UPSTREAM_REPO: &str = "JorisJonkers-dev/litellm";
const UPSTREAM_PATH: &str = "catalog/routing.json";
const DEFAULT_PATH: &str = "registry/litellm-tiers.json";
const COST_CLASSES: [&str; 3] = ["free", "vibe", "go"];

/// What is wrong with a snapshot; empty when it is one.
pub fn validate(data: &Json) -> Vec<String> {
    let mut problems = Vec::new();
    if !matches!(data, Json::Obj(_)) {
        return vec!["not a JSON object".to_string()];
    }
    if !matches!(data.get("source"), Some(Json::Obj(_))) {
        problems.push("no source object".to_string());
    }
    let Some(Json::Obj(groups)) = data.get("tier_groups").filter(|g| g.truthy()) else {
        problems.push("no tier_groups object".to_string());
        return problems;
    };
    for (name, tier) in groups {
        if !name.starts_with("tier-") {
            problems.push(format!("tier {name:?}: a tier is named tier-<name>"));
            continue;
        }
        if !matches!(tier, Json::Obj(_)) {
            problems.push(format!("tier {name}: not an object"));
            continue;
        }
        let ceiling = tier.get("ceiling").and_then(Json::as_str);
        if !ceiling.is_some_and(|ceiling| COST_CLASSES.contains(&ceiling)) {
            problems.push(format!(
                "tier {name}: ceiling must be one of {}",
                COST_CLASSES.join(", ")
            ));
        }
        let names_itself = match tier.get("key_models") {
            Some(Json::Arr(keys)) => {
                keys.iter().all(|key| key.as_str().is_some())
                    && keys.iter().any(|key| key.as_str() == Some(name))
            }
            _ => false,
        };
        if !names_itself {
            problems.push(format!(
                "tier {name}: key_models must be a list of names that includes {name}"
            ));
        }
        match tier.get("steps") {
            Some(Json::Arr(steps)) if !steps.is_empty() => {
                for step in steps {
                    if !step_is_valid(step, groups) {
                        problems.push(format!(
                            "tier {name}: step {} is neither a model with a cost class nor a known tier",
                            step.canonical()
                        ));
                    }
                }
            }
            _ => problems.push(format!("tier {name}: no steps")),
        }
    }
    problems
}

/// A step is `{"tier": <a tier of the snapshot>}` or `{"model": <name>, "cost": <a cost class>}`.
fn step_is_valid(step: &Json, groups: &[(String, Json)]) -> bool {
    let Json::Obj(members) = step else {
        return false;
    };
    let keys: Vec<&str> = members.iter().map(|(key, _)| key.as_str()).collect();
    match keys.as_slice() {
        ["tier"] => step.get("tier").and_then(Json::as_str).is_some_and(|tier| {
            groups.iter().any(|(name, _)| name == tier)
        }),
        ["model", "cost"] | ["cost", "model"] => {
            step.get("model").and_then(Json::as_str).is_some()
                && step
                    .get("cost")
                    .and_then(Json::as_str)
                    .is_some_and(|cost| COST_CLASSES.contains(&cost))
        }
        _ => false,
    }
}

/// Decodes standard base64, ignoring whitespace (GitHub wraps its lines).
fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let (mut bits, mut held) = (0u32, 0u32);
    for byte in text.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return Err(format!("not base64: byte {byte}")),
        };
        bits = (bits << 6) | u32::from(value);
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
            bits &= (1 << held) - 1;
        }
    }
    Ok(out)
}

/// litellm's `tier_groups` at `reference`, as a snapshot.
pub fn fetch_upstream(gh: &mut dyn Gh, reference: &str) -> Result<Json, String> {
    let path = format!("repos/{UPSTREAM_REPO}/contents/{UPSTREAM_PATH}?ref={reference}");
    let reply = gh
        .gh(&["api", &path])
        .map_err(|error| format!("gh could not fetch {UPSTREAM_REPO}/{UPSTREAM_PATH} at {reference}: {error}"))?;
    let unreadable = |why: String| {
        format!("{UPSTREAM_REPO}/{UPSTREAM_PATH} at {reference} has no readable tier_groups: {why}")
    };
    let listing = json::parse(&reply).map_err(&unreadable)?;
    let blob = listing
        .get("sha")
        .and_then(Json::as_str)
        .ok_or_else(|| unreadable("no sha".to_string()))?;
    let content = listing
        .get("content")
        .and_then(Json::as_str)
        .ok_or_else(|| unreadable("no content".to_string()))?;
    let decoded = base64_decode(content).map_err(&unreadable)?;
    let routing = json::parse(&String::from_utf8_lossy(&decoded)).map_err(&unreadable)?;
    let groups = routing
        .get("tier_groups")
        .ok_or_else(|| unreadable("no tier_groups".to_string()))?;
    let text = |value: &str| Json::Str(value.to_string());
    let snapshot = Json::Obj(vec![
        (
            "source".to_string(),
            Json::Obj(vec![
                ("repo".to_string(), text(UPSTREAM_REPO)),
                ("path".to_string(), text(UPSTREAM_PATH)),
                ("key".to_string(), text("tier_groups")),
                ("ref".to_string(), text(reference)),
                ("blob".to_string(), text(blob)),
            ]),
        ),
        ("tier_groups".to_string(), groups.clone()),
    ]);
    let problems = validate(&snapshot);
    if problems.is_empty() {
        Ok(snapshot)
    } else {
        Err(format!(
            "{UPSTREAM_REPO}/{UPSTREAM_PATH} at {reference} has malformed tier_groups: {}",
            problems.join("; ")
        ))
    }
}

/// The snapshot as the file holds it: two-space indent, keys in the order they came, a final newline.
pub fn render(value: &Json) -> String {
    let mut out = String::new();
    write_value(value, 0, &mut out);
    out.push('\n');
    out
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

fn write_value(value: &Json, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth + 1);
    let close = "  ".repeat(depth);
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(flag) => out.push_str(&flag.to_string()),
        Json::Num(text) => out.push_str(text),
        Json::Str(text) => out.push_str(&quote(text)),
        Json::Arr(items) if items.is_empty() => out.push_str("[]"),
        Json::Obj(entries) if entries.is_empty() => out.push_str("{}"),
        Json::Arr(items) => {
            out.push_str("[\n");
            for (at, item) in items.iter().enumerate() {
                out.push_str(&pad);
                write_value(item, depth + 1, out);
                out.push_str(if at + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&close);
            out.push(']');
        }
        Json::Obj(entries) => {
            out.push_str("{\n");
            for (at, (key, item)) in entries.iter().enumerate() {
                out.push_str(&format!("{pad}{}: ", quote(key)));
                write_value(item, depth + 1, out);
                out.push_str(if at + 1 < entries.len() { ",\n" } else { "\n" });
            }
            out.push_str(&close);
            out.push('}');
        }
    }
}

/// The tiers that differ between the snapshot and what upstream says now, sorted.
pub fn drift(ours: &Json, theirs: &Json) -> Vec<String> {
    let groups = |snapshot: &Json| match snapshot.get("tier_groups") {
        Some(Json::Obj(entries)) => entries.clone(),
        _ => Vec::new(),
    };
    let (ours, theirs) = (groups(ours), groups(theirs));
    let find = |entries: &[(String, Json)], name: &str| {
        entries.iter().find(|(key, _)| key == name).map(|(_, value)| value.canonical())
    };
    let mut names: Vec<&String> = ours.iter().chain(&theirs).map(|(name, _)| name).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|name| find(&ours, name) != find(&theirs, name))
        .cloned()
        .collect()
}

struct Args {
    write: bool,
    check: bool,
    reference: String,
    upstream: Option<String>,
    path: String,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        write: false,
        check: false,
        reference: "main".to_string(),
        upstream: None,
        path: DEFAULT_PATH.to_string(),
    };
    let mut at = 0;
    while at < args.len() {
        at += if args[at] == "--write" {
            parsed.write = true;
            1
        } else if args[at] == "--check" {
            parsed.check = true;
            1
        } else if let Some((value, used)) = option_value(args, at, "--ref") {
            parsed.reference = value.ok_or("--ref needs a value")?.to_string();
            used
        } else if let Some((value, used)) = option_value(args, at, "--upstream") {
            parsed.upstream = Some(value.ok_or("--upstream needs a value")?.to_string());
            used
        } else if let Some((value, used)) = option_value(args, at, "--path") {
            parsed.path = value.ok_or("--path needs a value")?.to_string();
            used
        } else {
            return Err(format!("unknown argument {}", args[at]));
        };
    }
    if parsed.write == parsed.check {
        return Err("name exactly one of --write and --check".to_string());
    }
    Ok(parsed)
}

pub fn run(args: &[String], io: &mut Io) -> i32 {
    run_with(args, io, &mut Subprocess::default())
}

pub fn run_with(args: &[String], io: &mut Io, gh: &mut dyn Gh) -> i32 {
    let parsed = match parse_args(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            io.complain(&format!("litellm-tiers: {message}"));
            return 2;
        }
    };
    match drive(&parsed, io, gh) {
        Ok(code) => code,
        Err(error) => {
            io.complain(&error);
            1
        }
    }
}

fn drive(args: &Args, io: &mut Io, gh: &mut dyn Gh) -> Result<i32, String> {
    if args.write {
        let snapshot = fetch_upstream(gh, &args.reference)?;
        std::fs::write(&args.path, render(&snapshot))
            .map_err(|error| format!("cannot write {}: {error}", args.path))?;
        io.say(&format!("tier snapshot written to {}", args.path));
        return Ok(0);
    }
    let text = std::fs::read_to_string(&args.path)
        .map_err(|error| format!("the tier snapshot {} cannot be read: {error}", args.path))?;
    let snapshot = json::parse(&text)
        .map_err(|error| format!("the tier snapshot {} is not JSON: {error}", args.path))?;
    let problems = validate(&snapshot);
    if !problems.is_empty() {
        return Err(format!(
            "the tier snapshot {} is malformed: {}",
            args.path,
            problems.join("; ")
        ));
    }
    if let Some(reference) = &args.upstream {
        let stale = drift(&snapshot, &fetch_upstream(gh, reference)?);
        if !stale.is_empty() {
            io.complain(&format!(
                "tier snapshot differs from litellm {reference}: {}; run --write --ref {reference}",
                stale.join(", ")
            ));
            return Ok(1);
        }
    }
    io.say("tier snapshot ok");
    Ok(0)
}
