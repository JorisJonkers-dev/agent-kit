//! `converge mcp|plugins` against a fake machine: a temporary home with two Claude profiles, Codex, both Vibe homes,
//! OpenCode and a keep file, and a fake runner that edits those files (or a plugin state) the way the real CLIs do.

use super::{Ran, Runner, SystemRunner};
use crate::cli::run_with;
use crate::Failure;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use toml_edit::DocumentMut;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// The plugin state the fake `claude plugin` commands serve: `(name)` markets and `(id, scope)` plugins.
#[derive(Default)]
struct Plugins {
    markets: Vec<String>,
    plugins: Vec<(String, String)>,
}

/// Edits the fake machine's files the way `claude` and `codex` do, and logs each call.
struct Fake {
    home: PathBuf,
    calls: Vec<String>,
    /// Names whose removal the CLI accepts and ignores.
    stuck: Vec<String>,
    plugins: Plugins,
    /// When set, every `claude ... --json` listing fails or prints this.
    listing: Option<Ran>,
}

impl Fake {
    fn new(home: &Path) -> Fake {
        Fake { home: home.to_path_buf(), calls: Vec::new(), stuck: Vec::new(), plugins: Plugins::default(), listing: None }
    }

    fn removals(&self) -> Vec<&String> {
        self.calls.iter().filter(|call| call.contains("remove") || call.contains("uninstall")).collect()
    }
}

impl Runner for Fake {
    fn run(&mut self, program: &str, args: &[String], env: &[(String, Option<String>)]) -> Result<Ran, Failure> {
        let config_dir = env.iter().find(|(name, _)| name == "CLAUDE_CONFIG_DIR").and_then(|(_, value)| value.clone());
        self.calls.push(format!("{program} {} @ {}", args.join(" "), config_dir.as_deref().unwrap_or("-")));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let ok = |stdout: String| Ok(Ran { stdout, success: true });
        let quoted = |items: Vec<String>| format!("[{}]", items.join(","));
        match (program, args.as_slice()) {
            ("claude", ["plugin", "marketplace", "list", "--json"]) | ("claude", ["plugin", "list", "--json"]) if self.listing.is_some() => {
                let listing = self.listing.as_ref().expect("listing");
                Ok(Ran { stdout: listing.stdout.clone(), success: listing.success })
            }
            ("claude", ["plugin", "marketplace", "list", "--json"]) => ok(quoted(self.plugins.markets.iter().map(|name| format!(r#"{{"name":"{name}"}}"#)).collect())),
            ("claude", ["plugin", "list", "--json"]) => ok(quoted(self.plugins.plugins.iter().map(|(id, scope)| format!(r#"{{"id":"{id}","scope":"{scope}"}}"#)).collect())),
            (_, [.., name]) if self.stuck.iter().any(|stuck| stuck == name) => ok(String::new()),
            ("claude", ["plugin", "marketplace", "remove", name]) => {
                self.plugins.markets.retain(|market| market != name);
                self.plugins.plugins.retain(|(id, _)| !id.ends_with(&format!("@{name}")));
                ok(String::new())
            }
            ("claude", ["plugin", "uninstall", name, "--scope", "user"]) => {
                self.plugins.plugins.retain(|(id, scope)| !(id == name && scope == "user"));
                ok(String::new())
            }
            ("claude", ["mcp", "remove", "--scope", "user", name]) => {
                let path = config_dir.map_or_else(|| self.home.join(".claude.json"), |dir| PathBuf::from(dir).join(".claude.json"));
                let text = fs::read_to_string(&path).expect("claude json").replace(&format!(r#""{name}": {{"type": "http"}}"#), "");
                fs::write(&path, text.replace("{, ", "{").replace(", }", "}").replace(", ,", ",")).expect("write");
                ok(String::new())
            }
            ("codex", ["mcp", "remove", name]) => {
                let path = self.home.join(".codex/config.toml");
                let mut doc: DocumentMut = fs::read_to_string(&path).expect("codex").parse().expect("toml");
                doc["mcp_servers"].as_table_mut().expect("table").remove(name);
                fs::write(&path, doc.to_string()).expect("write");
                ok(String::new())
            }
            other => panic!("unexpected call {other:?}"),
        }
    }
}

/// A fake workstation whose every surface holds the given servers.
struct Machine {
    root: PathBuf,
    home: PathBuf,
}

impl Machine {
    fn new(servers: &[&str], keep: Option<&str>) -> Machine {
        let root = std::env::temp_dir().join(format!("agent-kit-converge-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        let home = root.join("home");
        for dir in [".claude", ".claude-personal", ".codex", ".vibe", ".vibe-private", ".config/opencode", ".config/agent-kit"] {
            fs::create_dir_all(home.join(dir)).expect("dir");
        }
        let entries: Vec<String> = servers.iter().map(|name| format!(r#""{name}": {{"type": "http"}}"#)).collect();
        let claude = format!(r#"{{"mcpServers": {{{}}}, "other": 1}}"#, entries.join(", "));
        fs::write(home.join(".claude.json"), &claude).expect("write");
        fs::write(home.join(".claude-personal/.claude.json"), &claude).expect("write");
        let codex: String = servers.iter().map(|name| format!("[mcp_servers.\"{name}\"]\nurl = \"https://x/mcp\"\n\n[mcp_servers.\"{name}\".env]\nA = \"b\"\n\n")).collect();
        fs::write(home.join(".codex/config.toml"), format!("model = \"gpt\"\n\n{codex}")).expect("write");
        let vibe: String = servers.iter().map(|name| format!("[[mcp_servers]]\nname = \"{name}\"\ntransport = \"stdio\"\ncommand = \"x\"\n\n")).collect();
        for config in [".vibe/config.toml", ".vibe-private/config.toml"] {
            fs::write(home.join(config), format!("active_model = \"mistral\"\n\n{vibe}")).expect("write");
        }
        let opencode: Vec<String> = servers.iter().map(|name| format!("\"{name}\": {{}}")).collect();
        fs::write(home.join(".config/opencode/opencode.json"), format!("{{\"mcp\": {{\"servers\": {{{}}}}}, \"theme\": \"x\"}}", opencode.join(", "))).expect("write");
        if let Some(keep) = keep {
            fs::write(home.join(".config/agent-kit/keep.yaml"), keep).expect("write");
        }
        Machine { root, home }
    }

    fn path(&self, relative: &str) -> String {
        self.home.join(relative).display().to_string()
    }

    /// The names each surface holds now.
    fn surfaces(&self) -> Vec<(&'static str, BTreeSet<String>)> {
        let json_keys = |relative: &str, pointer: &str| -> BTreeSet<String> {
            let value = crate::json::parse(&fs::read_to_string(self.home.join(relative)).expect("read")).expect("json");
            let mut at = &value;
            for key in pointer.split('/') {
                at = at.as_object().and_then(|object| object.get(key)).expect("key");
            }
            at.as_object().expect("object").keys().into_iter().collect()
        };
        let toml = |relative: &str| -> DocumentMut { fs::read_to_string(self.home.join(relative)).expect("read").parse().expect("toml") };
        let codex = toml(".codex/config.toml");
        let codex: BTreeSet<String> = codex.get("mcp_servers").and_then(|item| item.as_table()).map(|table| table.iter().map(|(name, _)| name.to_string()).collect()).unwrap_or_default();
        let vibe = |relative: &str| super::vibe_entries(&toml(relative)).into_iter().collect::<BTreeSet<String>>();
        vec![
            ("claude work", json_keys(".claude.json", "mcpServers")),
            ("claude personal", json_keys(".claude-personal/.claude.json", "mcpServers")),
            ("codex", codex),
            ("vibe", vibe(".vibe/config.toml")),
            ("vibe-private", vibe(".vibe-private/config.toml")),
            ("opencode", json_keys(".config/opencode/opencode.json", "mcp/servers")),
        ]
    }

    /// The text of every file under the home.
    fn files(&self) -> Vec<(PathBuf, String)> {
        let mut files = Vec::new();
        let mut dirs = vec![self.home.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).expect("dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    files.push((path.clone(), fs::read_to_string(&path).expect("read")));
                }
            }
        }
        files.sort();
        files
    }

    /// `converge mcp` over every surface, as setup runs it, plus `extra` flags.
    fn mcp(&self, fake: &mut Fake, declared: &str, extra: &[&str]) -> (i32, String, String) {
        let mut args: Vec<String> = ["converge", "mcp", "--keep", &self.path(".config/agent-kit/keep.yaml"), "--claude-home", &self.path(".claude")]
            .into_iter()
            .map(str::to_string)
            .collect();
        for flag in [["--claude", &self.path(".claude")], ["--claude", &self.path(".claude-personal")], ["--codex", &self.path(".codex/config.toml")], ["--vibe", &self.path(".vibe/config.toml")], ["--vibe-private", &self.path(".vibe-private/config.toml")], ["--opencode", &self.path(".config/opencode/opencode.json")]] {
            args.extend(flag.iter().map(|text| text.to_string()));
        }
        for surface in ["claude", "codex", "vibe", "opencode"] {
            args.push(format!("--declared={surface}={declared}"));
        }
        args.push(format!("--declared=vibe-private={declared},memory-private"));
        args.extend(["--retired", "memory-mcp,vuetify", "--private", "memory-private"].iter().map(|text| text.to_string()));
        args.extend(extra.iter().map(|text| text.to_string()));
        self.run(fake, &args)
    }

    fn run(&self, fake: &mut Fake, args: &[String]) -> (i32, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let home = self.home.display().to_string();
        let env = move |name: &str| (name == "HOME").then(|| home.clone());
        let status = {
            let mut stdin: &[u8] = &[];
            let mut io = crate::Io { stdin: &mut stdin, out: &mut out, err: &mut err };
            run_with(args, &mut io, &env, fake)
        };
        (status, String::from_utf8(out).expect("utf8"), String::from_utf8(err).expect("utf8"))
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const DECLARED: &str = "memory-api,n8n,hindsight";

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[test]
fn an_undeclared_server_is_removed_from_every_surface() {
    let machine = Machine::new(&["memory-api", "pycharm"], Some("keep: []\n"));
    let mut fake = Fake::new(&machine.home);
    let (status, out, err) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!((status, err.as_str()), (0, ""), "{out}");
    for (surface, names) in machine.surfaces() {
        assert_eq!(names, set(&["memory-api"]), "{surface}");
    }
    assert!(out.contains("removed pycharm (codex, undeclared)\n"), "{out}");
    assert!(out.contains("removed pycharm (claude ~/.claude, undeclared)\n"), "{out}");
    assert!(out.contains("removed pycharm (claude ~/.claude-personal, undeclared)\n"), "{out}");
    assert!(out.contains("removed pycharm (opencode, undeclared)\n"), "{out}");
    // Each Claude profile is changed through its own CLAUDE_CONFIG_DIR; the primary one through the home's file.
    assert!(fake.calls.contains(&"claude mcp remove --scope user pycharm @ -".to_string()), "{:?}", fake.calls);
    assert!(fake.calls.contains(&format!("claude mcp remove --scope user pycharm @ {}", machine.path(".claude-personal"))), "{:?}", fake.calls);
    // Vibe keeps its other settings, and OpenCode its other keys, written as Python's json.dumps writes them.
    assert!(fs::read_to_string(machine.home.join(".vibe/config.toml")).expect("vibe").starts_with("active_model = \"mistral\"\n"));
    assert_eq!(fs::read_to_string(machine.home.join(".config/opencode/opencode.json")).expect("opencode"), "{\n  \"mcp\": {\n    \"servers\": {\n      \"memory-api\": {}\n    }\n  },\n  \"theme\": \"x\"\n}\n");
}

#[test]
fn the_keep_file_spares_a_server_and_a_retired_one_goes_anyway() {
    let machine = Machine::new(&["memory-api", "context7", "vuetify", "pycharm", "plugin:github:github"], Some("keep:\n  - context7\n  - vuetify\n"));
    let mut fake = Fake::new(&machine.home);
    let (status, out, _) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!(status, 0, "{out}");
    for (surface, names) in machine.surfaces() {
        assert_eq!(names, set(&["context7", "memory-api", "plugin:github:github"]), "{surface}");
    }
    assert!(out.contains("removed vuetify (vibe, retired)\n"), "{out}");
}

#[test]
fn a_missing_or_empty_keep_file_keeps_nothing() {
    for keep in [None, Some(""), Some("other: 1\n"), Some("keep:\n"), Some("- a list\n")] {
        let machine = Machine::new(&["memory-api", "context7"], keep);
        let mut fake = Fake::new(&machine.home);
        assert_eq!(machine.mcp(&mut fake, DECLARED, &[]).0, 0);
        assert!(machine.surfaces().iter().all(|(_, names)| !names.contains("context7")), "{keep:?}");
    }
}

#[test]
fn private_memory_survives_only_where_it_is_declared_whatever_the_keep_file_says() {
    let machine = Machine::new(&["memory-api", "memory-private"], Some("keep: [memory-private]\n"));
    let mut fake = Fake::new(&machine.home);
    let (status, out, _) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!(status, 0, "{out}");
    for (surface, names) in machine.surfaces() {
        let wanted = if surface == "vibe-private" { set(&["memory-api", "memory-private"]) } else { set(&["memory-api"]) };
        assert_eq!(names, wanted, "{surface}");
    }
    assert!(out.contains("removed memory-private (codex, private)\n"), "{out}");
}

#[test]
fn check_previews_each_removal_and_changes_nothing() {
    let machine = Machine::new(&["memory-api", "pycharm"], None);
    let before = machine.files();
    let mut fake = Fake::new(&machine.home);
    let (status, out, _) = machine.mcp(&mut fake, DECLARED, &["--check"]);
    assert_eq!(status, 0);
    assert_eq!(machine.files(), before);
    assert!(fake.removals().is_empty(), "{:?}", fake.calls);
    let mut lines: Vec<&str> = out.lines().collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            "would remove pycharm (claude ~/.claude, undeclared)",
            "would remove pycharm (claude ~/.claude-personal, undeclared)",
            "would remove pycharm (codex, undeclared)",
            "would remove pycharm (opencode, undeclared)",
            "would remove pycharm (vibe, undeclared)",
            "would remove pycharm (vibe-private, undeclared)",
        ]
    );
}

#[test]
fn a_run_fails_when_a_removed_server_is_still_listed() {
    let machine = Machine::new(&["memory-api", "pycharm"], None);
    let mut fake = Fake::new(&machine.home);
    fake.stuck.push("pycharm".to_string());
    let (status, out, _) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!(status, 1);
    assert!(out.contains("pycharm is still present (codex)\n"), "{out}");
    assert!(!out.contains("pycharm is still present (vibe)"), "{out}");
}

#[test]
fn a_malformed_keep_file_removes_nothing() {
    let machine = Machine::new(&["memory-api", "pycharm"], Some("keep: context7\n"));
    let before = machine.files();
    let mut fake = Fake::new(&machine.home);
    let (status, out, err) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!((status, out.as_str()), (1, ""));
    assert_eq!(err, format!("converge: {} must hold a top-level `keep:` list of names; nothing removed\n", machine.path(".config/agent-kit/keep.yaml")));
    assert_eq!(machine.files(), before);
    let list_of_numbers = Machine::new(&["pycharm"], Some("keep: [1]\n"));
    assert_eq!(list_of_numbers.mcp(&mut Fake::new(&list_of_numbers.home), DECLARED, &[]).0, 1);
    let not_yaml = Machine::new(&["pycharm"], Some("keep: [\n"));
    let (status, _, err) = not_yaml.mcp(&mut Fake::new(&not_yaml.home), DECLARED, &[]);
    assert_eq!(status, 2);
    assert!(err.starts_with(&format!("agent-kit-merge: {}: ", not_yaml.path(".config/agent-kit/keep.yaml"))), "{err}");
}

#[test]
fn opencode_that_is_not_plain_json_is_left_alone_and_fails_the_run() {
    let machine = Machine::new(&["memory-api", "pycharm"], None);
    fs::write(machine.home.join(".config/opencode/opencode.json"), "// comment\n{}").expect("write");
    let mut fake = Fake::new(&machine.home);
    let (status, out, _) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!(status, 1);
    assert!(out.starts_with("~/.config/opencode/opencode.json is not plain JSON; opencode not converged\n"), "{out}");
    assert!(out.contains("removed pycharm (codex, undeclared)"), "{out}");
}

#[test]
fn a_missing_surface_file_holds_nothing_and_an_opencode_without_servers_too() {
    let machine = Machine::new(&["pycharm"], None);
    for file in [".claude.json", ".codex/config.toml", ".vibe/config.toml"] {
        fs::remove_file(machine.home.join(file)).expect("remove");
    }
    fs::remove_file(machine.home.join(".config/opencode/opencode.json")).expect("remove");
    fs::write(machine.home.join(".vibe-private/config.toml"), "mcp_servers = 1\n").expect("write");
    fs::write(machine.home.join(".claude-personal/.claude.json"), "{\"mcpServers\": null}").expect("write");
    let mut fake = Fake::new(&machine.home);
    let (status, out, _) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!((status, out.as_str()), (0, ""));
    fs::write(machine.home.join(".config/opencode/opencode.json"), "{\"mcp\": {\"servers\": []}}").expect("write");
    fs::write(machine.home.join(".codex/config.toml"), "model = 1\n").expect("write");
    assert_eq!(machine.mcp(&mut fake, DECLARED, &[]).1, "");
}

#[test]
fn a_surface_that_cannot_be_read_stops_the_run() {
    let machine = Machine::new(&["pycharm"], None);
    let mut fake = Fake::new(&machine.home);
    fs::write(machine.home.join(".claude.json"), "[]").expect("write");
    let (status, _, err) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!((status, err), (2, format!("agent-kit-merge: {}: not a JSON object\n", machine.path(".claude.json"))));
    fs::write(machine.home.join(".claude.json"), "{}").expect("write");
    fs::write(machine.home.join(".codex/config.toml"), "= x").expect("write");
    let (status, _, err) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!(status, 2);
    assert!(err.starts_with(&format!("agent-kit-merge: {}: ", machine.path(".codex/config.toml"))), "{err}");
}

#[test]
fn a_vibe_entry_without_a_name_or_with_a_number_is_named_as_python_prints_it() {
    let machine = Machine::new(&[], None);
    fs::write(machine.home.join(".vibe/config.toml"), "[[mcp_servers]]\ncommand = \"x\"\n\n[[mcp_servers]]\nname = 7\n").expect("write");
    let mut fake = Fake::new(&machine.home);
    let (status, out, _) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!(status, 0, "{out}");
    assert!(out.contains("removed None (vibe, undeclared)\nremoved 7 (vibe, undeclared)\n") || out.contains("removed 7 (vibe, undeclared)\nremoved None (vibe, undeclared)\n"), "{out}");
    assert_eq!(fs::read_to_string(machine.home.join(".vibe/config.toml")).expect("vibe"), "");
}

#[test]
fn the_primary_profile_is_read_from_its_own_directory_when_claude_config_dir_is_set() {
    let machine = Machine::new(&["pycharm"], None);
    fs::write(machine.home.join(".claude/.claude.json"), "{\"mcpServers\": {\"memory-api\": {}}}").expect("write");
    let home = machine.home.display().to_string();
    let profile = machine.path(".claude");
    let env = move |name: &str| match name {
        "HOME" => Some(home.clone()),
        "CLAUDE_CONFIG_DIR" => Some(profile.clone()),
        _ => None,
    };
    let args: Vec<String> = ["converge", "mcp", "--keep", "/nowhere", "--claude-home", &machine.path(".claude"), "--claude", &machine.path(".claude"), "--claude", "/elsewhere", "--declared", "claude=memory-api"]
        .iter()
        .map(|text| text.to_string())
        .collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut stdin: &[u8] = &[];
    let mut io = crate::Io { stdin: &mut stdin, out: &mut out, err: &mut err };
    let mut fake = Fake::new(&machine.home);
    assert_eq!(run_with(&args, &mut io, &env, &mut fake), 0);
    assert!(fake.calls.is_empty(), "{:?}", fake.calls);
    assert_eq!(super::tilde(&machine.home, &env), "~/.");
    assert_eq!(super::tilde(Path::new("/elsewhere"), &env), "/elsewhere");
}

fn plugin_machine(markets: &[&str], plugins: &[(&str, &str)]) -> (Machine, Fake) {
    let machine = Machine::new(&[], None);
    let mut fake = Fake::new(&machine.home);
    fake.plugins.markets = markets.iter().map(|name| name.to_string()).collect();
    fake.plugins.plugins = plugins.iter().map(|(id, scope)| (id.to_string(), scope.to_string())).collect();
    (machine, fake)
}

fn plugins(machine: &Machine, fake: &mut Fake, keep: &str, extra: &[&str]) -> (i32, String, String) {
    fs::write(machine.home.join(".config/agent-kit/keep.yaml"), keep).expect("write");
    let mut args: Vec<String> = ["converge", "plugins", "--keep", &machine.path(".config/agent-kit/keep.yaml"), "--declared-markets", "agent-kit-official,caveman", "--declared-plugins", "caveman@caveman,github@agent-kit-official", "--retired-markets", "hindsight", "--retired-plugins", "hindsight-memory@hindsight"]
        .iter()
        .map(|text| text.to_string())
        .collect();
    args.extend(extra.iter().map(|text| text.to_string()));
    machine.run(fake, &args)
}

#[test]
fn plugins_go_before_their_marketplaces_and_a_kept_one_stays() {
    let (machine, mut fake) = plugin_machine(
        &["agent-kit-official", "caveman", "claude-plugins-official", "hindsight", "nedap-ai-skills"],
        &[("caveman@caveman", "user"), ("github@agent-kit-official", "user"), ("plugin-dev@agent-kit-official", "user"), ("hindsight-memory@hindsight", "user"), ("review@nedap-ai-skills", "user"), ("local@agent-kit-official", "project")],
    );
    let (status, out, _) = plugins(&machine, &mut fake, "keep: [nedap-ai-skills]\n", &[]);
    assert_eq!(status, 0, "{out}");
    assert_eq!(
        out,
        "removed hindsight-memory@hindsight (claude plugin, retired)\nremoved plugin-dev@agent-kit-official (claude plugin, undeclared)\nremoved claude-plugins-official (claude marketplace, undeclared)\nremoved hindsight (claude marketplace, retired)\n"
    );
    assert_eq!(fake.plugins.markets, ["agent-kit-official", "caveman", "nedap-ai-skills"]);
    assert_eq!(fake.plugins.plugins.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["caveman@caveman", "github@agent-kit-official", "review@nedap-ai-skills", "local@agent-kit-official"]);
    assert!(fake.removals().iter().all(|call| call.ends_with("@ -")), "{:?}", fake.calls);
}

#[test]
fn plugin_check_changes_nothing_and_a_stuck_marketplace_fails_the_run() {
    let (machine, mut fake) = plugin_machine(&["agent-kit-official", "claude-plugins-official"], &[]);
    let (status, out, _) = plugins(&machine, &mut fake, "", &["--check"]);
    assert_eq!((status, out.as_str()), (0, "would remove claude-plugins-official (claude marketplace, undeclared)\n"));
    assert!(fake.removals().is_empty());
    fake.stuck.push("claude-plugins-official".to_string());
    let (status, out, _) = plugins(&machine, &mut fake, "", &[]);
    assert_eq!(status, 1);
    assert!(out.ends_with("claude-plugins-official is still present (claude marketplace)\n"), "{out}");
    let (status, _, err) = plugins(&machine, &mut fake, "keep: 5\n", &[]);
    assert_eq!(status, 1);
    assert!(err.contains("must hold a top-level `keep:` list of names"), "{err}");
}

#[test]
fn a_listing_that_fails_or_is_not_a_list_stops_the_run() {
    let (machine, mut fake) = plugin_machine(&[], &[]);
    fake.listing = Some(Ran { stdout: String::new(), success: false });
    assert_eq!(plugins(&machine, &mut fake, "", &[]).2, "agent-kit-merge: claude plugin list --json failed\n");
    fake.listing = Some(Ran { stdout: "{}".to_string(), success: true });
    assert_eq!(plugins(&machine, &mut fake, "", &[]).2, "agent-kit-merge: claude plugin list --json did not print a JSON list\n");
    fake.listing = Some(Ran { stdout: r#"[{"id": 5, "scope": "user"}, {"scope": "user"}]"#.to_string(), success: true });
    let (_, out, _) = plugins(&machine, &mut fake, "", &["--check"]);
    // The fake serves the same listing to both surfaces: the marketplace one has no name, so it is None too.
    assert_eq!(out, "would remove 5 (claude plugin, undeclared)\nwould remove None (claude plugin, undeclared)\nwould remove None (claude marketplace, undeclared)\n");
}

#[test]
fn the_command_line_refuses_what_argparse_would() {
    let machine = Machine::new(&[], None);
    let mut fake = Fake::new(&machine.home);
    let run = |args: &[&str], fake: &mut Fake| machine.run(fake, &args.iter().map(|text| text.to_string()).collect::<Vec<_>>());
    assert_eq!(run(&["converge"], &mut fake).2, "agent-kit-merge: converge: expected mcp, plugins, gold-band-roles or gold-band-workflows\n");
    assert_eq!(run(&["converge", "gold"], &mut fake).2, "agent-kit-merge: converge: unknown surface \"gold\"; expected mcp, plugins, gold-band-roles or gold-band-workflows\n");
    assert_eq!(run(&["converge", "mcp"], &mut fake).2, "agent-kit-merge: the following arguments are required: --claude-home, --keep\n");
    assert_eq!(run(&["converge", "plugins"], &mut fake).2, "agent-kit-merge: the following arguments are required: --keep\n");
    assert_eq!(run(&["converge", "plugins", "--keep", "/nowhere", "--bogus"], &mut fake).0, 2);
    assert!(fake.calls.len() == 0);
}

#[test]
fn the_system_runner_starts_a_process_with_the_environment_it_is_given() {
    let mut runner = SystemRunner;
    let script = |text: &str| vec!["-c".to_string(), text.to_string()];
    let ran = runner.run("sh", &script("printf '%s|%s' \"$AK_SET\" \"${HOME:-unset}\""), &[("AK_SET".to_string(), Some("v".to_string())), ("HOME".to_string(), None)]).expect("ran");
    assert_eq!(ran, Ran { stdout: "v|unset".to_string(), success: true });
    assert!(!runner.run("sh", &script("exit 3"), &[]).expect("ran").success);
    assert!(runner.run("/nonexistent/agent-kit-tool", &[], &[]).expect_err("spawn").0.starts_with("/nonexistent/agent-kit-tool: "));
}

#[test]
fn a_removal_from_a_file_that_changed_under_it_is_refused() {
    // Between the read and the removal another writer emptied the file: nothing is there to remove from.
    let machine = Machine::new(&[], None);
    let mut fake = Fake::new(&machine.home);
    for (kind, file, text) in [(super::Kind::OpenCode, ".config/opencode/opencode.json", "[]"), (super::Kind::Vibe, ".vibe/config.toml", "a = 1\n"), (super::Kind::Vibe, ".vibe/config.toml", "mcp_servers = 1\n")] {
        let path = machine.home.join(file);
        fs::write(&path, text).expect("write");
        let surface = super::Surface::new("s", kind, &path);
        assert_eq!(surface.remove("x", &mut fake), Err(Failure(format!("{}: its MCP servers changed while they were converged", path.display()))));
    }
    assert_eq!((super::Kind::Plugin.name(), super::Kind::Marketplace.name()), ("plugin", "marketplace"));
}

#[test]
fn a_vibe_home_with_an_inline_server_array_is_converged_too() {
    let machine = Machine::new(&[], None);
    let path = machine.home.join(".vibe/config.toml");
    fs::write(&path, "a = 1\nmcp_servers = [{ name = \"stale\", command = \"x\" }, { name = \"memory-api\", command = \"y\" }]\n").expect("write");
    let mut fake = Fake::new(&machine.home);
    let (status, out, _) = machine.mcp(&mut fake, DECLARED, &[]);
    assert_eq!(status, 0, "{out}");
    assert!(out.contains("removed stale (vibe, undeclared)\n"), "{out}");
    let doc: DocumentMut = fs::read_to_string(&path).expect("read").parse().expect("toml");
    assert_eq!(super::vibe_entries(&doc), ["memory-api"]);
}
