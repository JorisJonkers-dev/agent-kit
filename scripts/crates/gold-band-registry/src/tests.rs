use super::*;
use std::path::PathBuf;

const WORK_OK: &str = "instance\tgold-band\n\
    field\tagents.0\tclaude-work\nfield\tagents.1\tcodex\n\
    field\trepository_rule.remotes\tnedap-only\nfield\trepository_rule.no_remote\trefuse\n\
    field\thosts.0\tmac\nfield\tbanks.0\tnedap\n";
const PERSONAL_OK: &str = "instance\tgold-band-personal\n\
    field\tagents.0\tvibe\nfield\tagents.1\topencode\n\
    field\trepository_rule.remotes\tnot-nedap\nfield\trepository_rule.no_remote\tallow\n\
    field\thosts.0\tlinux\nfield\thosts.1\tmac\nfield\tbanks.0\tshared\n";

fn registry() -> String {
    format!("source\tregistry\n{WORK_OK}{PERSONAL_OK}")
}

fn found(text: &str) -> Vec<String> {
    problems(&parse(text).expect("parses"))
}

fn has(text: &str, needle: &str) {
    let all = found(text);
    assert!(all.iter().any(|p| p.contains(needle)), "{needle} in {all:?}");
}

fn overlay(body: &str) -> String {
    format!("{}source\toverlay\n{body}", registry())
}

#[test]
fn a_correct_registry_is_clean() {
    assert_eq!(found(&registry()), Vec::<String>::new());
}

#[test]
fn missing_and_duplicate_and_unknown_instances_are_refused() {
    has("source\tregistry\n", "registry gold-band: instance is not declared");
    has("source\tregistry\n", "registry gold-band-personal: instance is not declared");
    has(&format!("{}{WORK_OK}", registry()), "declared more than once");
    has(&format!("{}instance\tother\n", registry()), "unknown instance");
}

#[test]
fn work_instance_agents_are_exactly_claude_work_and_codex() {
    let text = registry().replace("claude-work", "vibe");
    has(&text, "agent vibe is not allowed on the work instance");
    has(&registry().replace("claude-work", "Bad_Id"), "bad agent id");
    has(&format!("{}field\tagents.2\tcodex\n", registry().replace(PERSONAL_OK, "")), "declared more than once");
}

#[test]
fn personal_instance_refuses_codex_and_the_work_claude_agent() {
    for bad in ["codex", "codex-review", "claude-work", "claude-work-2"] {
        let text = registry().replace("agents.1\topencode", &format!("agents.1\t{bad}"));
        has(&text, &format!("agent {bad} is not allowed on the personal instance"));
    }
    assert!(found(&registry().replace("opencode", "claude-personal")).is_empty());
}

#[test]
fn repository_rules_are_required_and_exact() {
    let no_rule = registry().replace("field\trepository_rule.remotes\tnedap-only\n", "");
    has(&no_rule, "repository_rule.remotes is missing (must be nedap-only)");
    let no_remote = registry().replace("field\trepository_rule.no_remote\tallow\n", "");
    has(&no_remote, "repository_rule.no_remote is missing (must be allow)");
    has(&registry().replace("nedap-only", "any"), "remotes is any, must be nedap-only");
    has(&registry().replace("not-nedap", "nedap-only"), "remotes is nedap-only, must be not-nedap");
    has(&registry().replace("\trefuse", "\tallow"), "no_remote is allow, must be refuse");
}

#[test]
fn hosts_and_banks_are_checked() {
    has(&registry().replace("hosts.0\tmac", "hosts.0\tlinux"), "work machine (mac) only");
    has(&registry().replace("hosts.0\tlinux", "hosts.0\twindows"), "unknown host windows");
    has(&registry().replace("banks.0\tnedap\n", "banks.0\tshared\n"), "bank shared is not allowed (this instance reaches the nedap bank only)");
    has(&registry().replace("banks.0\tnedap\n", "banks.0\tnedap\nfield\tbanks.1\tshared\n"), "bank shared is not allowed");
    has(&registry().replace("banks.0\tshared\n", "banks.0\tnedap\n"), "bank nedap is not allowed (this instance reaches the shared bank only)");
    has(&registry().replace("banks.0\tshared\n", "banks.0\tprivate\n"), "bank private is not allowed");
}

#[test]
fn repeated_fields_are_refused() {
    let text = format!("{}field\trepository_rule.remotes\tnedap-only\n", registry());
    has(&text, "field repository_rule.remotes is set more than once");
    has(&overlay("instance\tgold-band\nfield\tsettings.a\tb\nfield\tsettings.a\tc\n"), "settings.a is set more than once");
}

#[test]
fn secrets_in_names_and_ids_are_refused_and_never_printed() {
    let secret = "ghp_aaaaaaaaaaaaaaaaaaaaaaaa";
    let body = format!("instance\t{secret}\ninstance\tgold-band\nfield\tsettings.{secret}\tx\nfield\tmcp_servers.{secret}.url\tx\n");
    let all = found(&overlay(&body));
    assert!(all.iter().any(|p| p.contains("a field name holds a literal secret (github token)")), "{all:?}");
    assert!(all.iter().any(|p| p.contains("unknown instance")));
    assert!(all.iter().all(|p| !p.contains(secret)), "{all:?}");
    let upper = "AKIAAAAAAAAAAAAAAAAA";
    let body = format!("instance\tgold-band\nfield\tmcp_servers.{upper}.command\tx\nfield\tmcp_servers.a.env.{upper}\tx\nfield\tmcp_servers.b.url\tx\nfield\tmcp_servers.b.command\tx\n");
    let all = found(&overlay(&body));
    assert!(all.iter().all(|p| !p.contains(upper)), "{all:?}");
}

#[test]
fn repeated_hosts_and_banks_are_refused() {
    has(&registry().replace("hosts.0\tmac\n", "hosts.0\tmac\nfield\thosts.1\tmac\n"), "host mac is listed more than once");
    has(&registry().replace("banks.0\tnedap\n", "banks.0\tnedap\nfield\tbanks.1\tnedap\n"), "bank nedap is listed more than once");
}

#[test]
fn empty_lists_are_refused() {
    let bare = "source\tregistry\ninstance\tgold-band\ninstance\tgold-band-personal\n";
    for what in ["declares no agents", "declares no hosts", "declares no banks"] {
        has(bare, what);
    }
}

#[test]
fn unknown_fields_and_literal_secrets_are_refused() {
    has(&format!("{}field\tmodel\tx\n", registry()), "unknown field model");
    has(&format!("{}field\tagents\tx\n", registry()), "unknown field agents");
    has(&format!("{}field\tagents.x\tx\n", registry()), "unknown field agents.x");
    has(&format!("{}field\tbanks.0.x\tshared\n", registry()), "unknown field banks.0.x");
    let text = registry().replace("hosts.0\tmac", "hosts.0\tsk-aaaaaaaaaaaaaaaaaaaa");
    let all = found(&text);
    assert!(all.iter().any(|p| p.contains("literal secret (openai key)")));
    assert!(all.iter().all(|p| !p.contains("sk-aaaa")));
}

#[test]
fn overlay_adds_servers_and_settings_to_the_work_instance() {
    let body = "instance\tgold-band\n\
        field\tmcp_servers.jira.command\tnpx\nfield\tmcp_servers.jira.args.0\t-y\n\
        field\tmcp_servers.jira.env.JIRA_TOKEN\t$JIRA_TOKEN\n\
        field\tmcp_servers.wiki.url\thttps://wiki.example/mcp\n\
        field\tsettings.editor.theme\tdark\nfield\tagents.0\tcodex\n";
    assert_eq!(found(&overlay(body)), Vec::<String>::new());
}

#[test]
fn overlay_may_not_touch_the_personal_instance_except_for_agents() {
    let body = "instance\tgold-band-personal\nfield\tmcp_servers.x.command\tnpx\nfield\tsettings.a\tb\n";
    has(&overlay(body), "overlay may add mcp_servers to the work instance gold-band only");
    has(&overlay(body), "overlay may add settings to the work instance gold-band only");
    let ok = "instance\tgold-band-personal\nfield\tagents.0\tvibe\n";
    assert!(found(&overlay(ok)).is_empty());
}

#[test]
fn overlay_refuses_cross_instance_agents() {
    for bad in ["codex", "claude-work"] {
        let body = format!("instance\tgold-band-personal\nfield\tagents.0\t{bad}\n");
        has(&overlay(&body), &format!("overlay gold-band-personal: agent {bad} is not allowed"));
    }
    let body = "instance\tgold-band\nfield\tagents.0\tvibe\n";
    has(&overlay(body), "overlay gold-band: agent vibe is not allowed on the work instance");
}

#[test]
fn overlay_shape_is_checked() {
    for (body, needle) in [
        ("field\tmcp_servers.jira\tx\n", "unknown field mcp_servers.jira"),
        ("field\tmcp_servers.Bad.command\tx\n", "bad mcp server name Bad"),
        ("field\tmcp_servers.a.command\tx\nfield\tmcp_servers.a.url\ty\n", "exactly one of command or url"),
        ("field\tmcp_servers.a.args.0\tx\n", "exactly one of command or url"),
        ("field\tmcp_servers.a.what\tx\n", "unknown field mcp_servers.a.what"),
        ("field\tmcp_servers.a.args.x\tx\n", "unknown field mcp_servers.a.args.x"),
        ("field\tmcp_servers.a.env.lower\tx\n", "unknown field mcp_servers.a.env.lower"),
        ("field\tsettings\tx\n", "bad settings path settings"),
        ("field\tsettings.a b\tx\n", "bad settings path settings.a b"),
        ("field\thosts.0\tlinux\n", "unknown field hosts.0"),
        ("field\trepository_rule.remotes\tx\n", "unknown field repository_rule.remotes"),
        ("field\tagents\tx\n", "unknown field agents"),
    ] {
        has(&overlay(&format!("instance\tgold-band\n{body}")), needle);
    }
    has(&overlay("instance\tgold-band\ninstance\tgold-band\n"), "overlay gold-band: declared more than once");
    has(&overlay("instance\tnope\n"), "overlay nope: unknown instance");
}

#[test]
fn overlay_literal_secrets_are_refused() {
    let body = "instance\tgold-band\nfield\tmcp_servers.a.command\tx\nfield\tmcp_servers.a.env.API_TOKEN\thunter2\n\
        field\tsettings.endpoint\tBearer abcdefghijklmnopqrstuv\n";
    has(&overlay(body), "secret-named");
    has(&overlay(body), "literal secret (bearer token)");
}

#[test]
fn parse_rejects_malformed_lines() {
    for bad in [
        "field\ta\tb\n",
        "source\tregistry\nfield\ta\tb\n",
        "instance\tgold-band\n",
        "source\tother\n",
        "source\tregistry\ninstance\ta\nfield\tonly-path\n",
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    assert_eq!(parse("# note\n\n").expect("empty").len(), 0);
}

fn file(name: &str, text: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("gbr-{}-{name}", std::process::id()));
    fs::write(&path, text).expect("writes");
    path
}

#[test]
fn the_command_line_answers_with_the_documented_exit_codes() {
    let run_with = |args: &[&OsStr]| run(args.iter().map(|a| (*a).to_os_string()));
    let ok = file("ok", &registry());
    let bad = file("bad", "source\tregistry\n");
    let malformed = file("malformed", "nonsense\n");
    let validate = OsStr::new("validate");
    assert_eq!(run_with(&[validate, ok.as_os_str()]), 0);
    assert_eq!(run_with(&[validate, bad.as_os_str()]), 1);
    assert_eq!(run_with(&[validate, malformed.as_os_str()]), 2);
    assert_eq!(run_with(&[validate, OsStr::new("/nonexistent/gbr")]), 2);
    assert_eq!(run_with(&[]), 2);
    assert_eq!(run_with(&[OsStr::new("launch"), ok.as_os_str()]), 2);
    assert!(usage().contains(NAME));
}
