use super::*;
use std::path::PathBuf;

const GOOD: &str = "
agents:
  fake:
    command: [sh, -c, 'printf \"%s:%s:%s\" \"$AGENT_KIT_REACH\" \"$AGENT_KIT_OUTPUT_DIR\" \"$0\" > \"$1\"']
    levels:
      read:
        args: [--tools, none]
        support: {linux: verified, mac: unverified}
      research:
        args: [--out]
        support: {linux: verified, mac: verified}
      write:
        support: {linux: unsupported, mac: verified}
ids:
  fake-read:
    agent: fake
    reach: read
    hosts: [linux]
  fake-research:
    agent: fake
    reach: research
    output_dir: /tmp/research
    hosts: [linux, mac]
";

fn with_command(command: &str) -> String {
    GOOD.lines()
        .map(|l| if l.starts_with("    command:") { format!("    command: {command}") } else { l.to_string() })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A command that records the skill set variables it was started with.
const SKILL_COMMAND: &str =
    "[sh, -c, 'printf \"%s:%s\" \"$AGENT_KIT_SKILL_SET\" \"$AGENT_KIT_SKILLS_DIR\" > \"$1\"']";

fn with_skill_set(text: &str, name: &str) -> String {
    text.replace(
        "    output_dir: /tmp/research\n",
        &format!("    output_dir: /tmp/research\n    skill_set: {name}\n"),
    )
}

fn record(text: &str) -> Record {
    parse(text).expect("parses")
}

fn found(text: &str) -> Vec<String> {
    problems(&record(text))
}

fn has(text: &str, needle: &str) {
    let all = found(text);
    assert!(all.iter().any(|p| p.contains(needle)), "{needle} in {all:?}");
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agent-launcher-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn args(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}

#[test]
fn a_correct_record_is_clean() {
    assert_eq!(found(GOOD), Vec::<String>::new());
}

#[test]
fn a_malformed_record_is_not_parsed() {
    assert!(parse("ids: [").is_err());
    assert!(parse("surprise: 1").is_err());
}

#[test]
fn an_id_without_a_reach_level_is_refused() {
    has("agents: {a: {command: [x]}}\nids: {a-one: {agent: a, hosts: [linux]}}", "no reach level");
}

#[test]
fn an_id_without_an_agent_or_with_an_unknown_one_is_refused() {
    has("ids: {a-one: {reach: read, hosts: [linux]}}", "no agent");
    has("ids: {a-one: {agent: ghost, reach: read, hosts: [linux]}}", "unknown agent");
}

#[test]
fn research_and_write_scoped_need_an_output_directory() {
    for level in ["research", "write-scoped"] {
        let text = GOOD.replace("    output_dir: /tmp/research\n", "").replace("research", level);
        has(&text, &format!("reach level {level} needs an output_dir"));
    }
}

#[test]
fn other_levels_take_no_output_directory_and_it_stays_inside() {
    has(&GOOD.replace("    reach: read\n", "    reach: read\n    output_dir: /tmp/x\n"), "takes no output_dir");
    has(&GOOD.replace("/tmp/research", "/tmp/../etc"), "leaves its parent");
    has(&GOOD.replace("/tmp/research", "''"), "is empty");
}

#[test]
fn an_unknown_level_is_refused_everywhere() {
    has(&GOOD.replace("reach: read", "reach: root"), "unknown reach level \"root\"");
    has(&GOOD.replace("      write:\n", "      root:\n"), "unknown reach level \"root\"");
}

#[test]
fn an_unverified_or_unsupported_cell_cannot_back_an_id() {
    has(&GOOD.replace("hosts: [linux]", "hosts: [linux, mac]"), "is unverified on mac");
    has(&GOOD.replace("reach: read", "reach: write"), "is unsupported on linux");
    has(&GOOD.replace("reach: read", "reach: full"), "is unverified on linux");
}

#[test]
fn every_cell_needs_both_oses_and_a_known_state() {
    has(&GOOD.replace("{linux: verified, mac: unverified}", "{linux: verified}"), "no support state for mac");
    has(&GOOD.replace("mac: unverified", "mac: sure"), "bad support state \"sure\"");
    has(&GOOD.replace("mac: unverified}", "mac: verified, win: verified}"), "unknown OS \"win\"");
}

#[test]
fn agents_need_a_command_and_a_good_name() {
    has(&with_command("['']"), "no command");
    has("agents: {A_b: {}}", "bad agent name");
    has("agents: {a: {}}", "no command");
}

#[test]
fn ids_need_hosts_a_good_name_and_known_hosts() {
    has(&GOOD.replace("    hosts: [linux]\n", ""), "declares no hosts");
    has(&GOOD.replace("fake-read:", "Fake_Read:"), "bad agent id");
    has(&GOOD.replace("hosts: [linux]", "hosts: [beos]"), "unknown host");
}

#[test]
fn one_id_for_each_agent_level_and_skill_set() {
    let twin = format!("{GOOD}  fake-read-again:\n    agent: fake\n    reach: read\n    hosts: [linux]\n");
    has(&twin, "same agent, level and skill set as fake-read");
    let other = twin.replace(
        "fake-read-again:\n    agent: fake\n    reach: read\n",
        "fake-read-again:\n    agent: fake\n    reach: read\n    skill_set: review\n",
    );
    assert_eq!(found(&other), Vec::<String>::new());
}

#[test]
fn a_level_an_agent_does_not_declare_is_unverified() {
    let r = record(GOOD);
    assert_eq!(cell(&r, "fake", "full", "linux"), "unverified");
    assert_eq!(cell(&r, "ghost", "read", "linux"), "unverified");
    assert_eq!(cell(&r, "fake", "read", "linux"), "verified");
}

#[test]
fn plan_builds_the_command_with_the_level_arguments() {
    let r = record(GOOD);
    let plan = plan(&r, "fake-research", "mac").unwrap();
    assert_eq!(plan.level, "research");
    assert_eq!(plan.output_dir.as_deref(), Some("/tmp/research"));
    assert_eq!(plan.argv.last().map(String::as_str), Some("--out"));
}

#[test]
fn plan_carries_the_ids_skill_set() {
    let r = record(&with_skill_set(GOOD, "review"));
    assert_eq!(plan(&r, "fake-research", "linux").unwrap().skill_set.as_deref(), Some("review"));
    assert_eq!(plan(&record(GOOD), "fake-research", "linux").unwrap().skill_set, None);
}

#[test]
fn skills_dir_prefers_gold_band_home_and_falls_back_to_home() {
    let id = "fake-research";
    assert_eq!(
        skills_dir(Some("/gb"), Some("/home/u"), id),
        Some(PathBuf::from("/gb/.gold-band/skill-sets/fake-research"))
    );
    assert_eq!(
        skills_dir(None, Some("/home/u"), id),
        Some(PathBuf::from("/home/u/.gold-band/skill-sets/fake-research"))
    );
    assert_eq!(
        skills_dir(Some(""), Some("/home/u"), id),
        Some(PathBuf::from("/home/u/.gold-band/skill-sets/fake-research"))
    );
    assert_eq!(
        skills_dir(Some("  "), Some("/home/u"), id),
        Some(PathBuf::from("/home/u/.gold-band/skill-sets/fake-research"))
    );
    assert_eq!(skills_dir(Some(""), Some(""), id), None);
    assert_eq!(skills_dir(None, None, id), None);
}

#[test]
fn plan_refuses_what_the_agent_cannot_hold_on_this_os() {
    let r = record(GOOD);
    assert!(plan(&r, "ghost", "linux").unwrap_err().contains("unknown agent id"));
    assert!(plan(&r, "fake-read", "mac").unwrap_err().contains("not declared for mac"));
    let widened = record(&GOOD.replace("hosts: [linux]", "hosts: [linux, mac]"));
    assert!(plan(&widened, "fake-read", "mac").unwrap_err().contains("unverified on mac"));
    assert!(plan(&r, "fake-read", "linux").is_ok());
}

#[test]
fn plan_refuses_an_os_the_id_is_not_declared_for() {
    let text = GOOD.replace("hosts: [linux, mac]", "hosts: [linux]");
    assert!(plan(&record(&text), "fake-research", "mac").unwrap_err().contains("not declared for mac"));
}

#[test]
fn plan_refuses_a_broken_id() {
    let no_level = "agents: {a: {command: [x]}}\nids: {a-one: {agent: a}}";
    assert!(plan(&record(no_level), "a-one", "linux").unwrap_err().contains("no reach level"));
    let no_agent = "ids: {a-one: {agent: ghost, reach: read}}";
    assert!(plan(&record(no_agent), "a-one", "linux").unwrap_err().contains("unknown agent"));
    let no_command = "agents: {a: {levels: {read: {support: {linux: verified, mac: verified}}}}}\nids: {a-one: {agent: a, reach: read, hosts: [linux]}}";
    assert!(plan(&record(no_command), "a-one", "linux").unwrap_err().contains("no command"));
    let no_dir = GOOD.replace("    output_dir: /tmp/research\n", "");
    assert!(plan(&record(&no_dir), "fake-research", "linux").unwrap_err().contains("needs an output_dir"));
}

fn write_record(dir: &std::path::Path) -> OsString {
    let file = dir.join("reach.yaml");
    fs::write(&file, GOOD).unwrap();
    file.into_os_string()
}

#[test]
fn validate_reports_through_the_exit_code() {
    let dir = scratch("validate");
    let file = write_record(&dir);
    assert_eq!(run_on([OsString::from("validate"), file.clone()], "linux"), 0);
    fs::write(&file, GOOD.replace("reach: read", "reach: root")).unwrap();
    assert_eq!(run_on([OsString::from("validate"), file.clone()], "linux"), 1);
    fs::write(&file, "ids: [").unwrap();
    assert_eq!(run_on([OsString::from("validate"), file], "linux"), 2);
    assert_eq!(run(args(&["validate", "/nonexistent/reach.yaml"])), 2);
}

#[test]
fn bad_usage_exits_2() {
    assert_eq!(run(args(&[])), 2);
    assert_eq!(run(args(&["run", "f", "id", "extra"])), 2);
}

#[cfg(target_os = "linux")]
#[test]
fn a_permitted_id_starts_its_agent_with_its_reach_and_arguments() {
    let dir = scratch("start");
    // The research level may write only its output directory, so the marker goes there.
    let out = dir.join("out");
    let record = dir.join("reach.yaml");
    fs::write(&record, GOOD.replace("/tmp/research", &out.to_string_lossy())).unwrap();
    let marker = out.join("marker");
    let code = run_on(
        [OsString::from("run"), record.into_os_string(), "fake-research".into(), "--".into(), marker.clone().into_os_string()],
        "linux",
    );
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        format!("research:{}:--out", out.display())
    );
}

#[test]
fn an_id_with_a_skill_set_exports_it_and_its_directory() {
    let dir = scratch("skills");
    let gold_band_home = dir.join("gold-band");
    let out = dir.join("out");
    let file = dir.join("reach.yaml");
    let text = with_skill_set(&with_command(SKILL_COMMAND), "review").replace("/tmp/research", &out.to_string_lossy());
    fs::write(&file, text).unwrap();
    let marker = out.join("marker");
    std::env::set_var("GOLD_BAND_HOME", &gold_band_home);
    let code = run_on(
        [OsString::from("run"), file.into_os_string(), "fake-research".into(), "--".into(), marker.clone().into_os_string()],
        "linux",
    );
    std::env::remove_var("GOLD_BAND_HOME");
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        format!("review:{}", gold_band_home.join(".gold-band").join("skill-sets").join("fake-research").display())
    );
}

#[test]
fn an_id_without_a_skill_set_exports_neither() {
    let dir = scratch("no-skills");
    let out = dir.join("out");
    let file = dir.join("reach.yaml");
    fs::write(&file, with_command(SKILL_COMMAND).replace("/tmp/research", &out.to_string_lossy())).unwrap();
    let marker = out.join("marker");
    std::env::set_var("AGENT_KIT_SKILL_SET", "stale");
    std::env::set_var("AGENT_KIT_SKILLS_DIR", "/stale/skill-sets/fake-research");
    let code = run_on(
        [OsString::from("run"), file.into_os_string(), "fake-research".into(), "--".into(), marker.clone().into_os_string()],
        "linux",
    );
    std::env::remove_var("AGENT_KIT_SKILL_SET");
    std::env::remove_var("AGENT_KIT_SKILLS_DIR");
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(&marker).unwrap(), ":");
}

#[test]
fn a_refused_id_starts_nothing() {
    let dir = scratch("refuse");
    let record = write_record(&dir);
    let marker = dir.join("marker");
    // fake-read is verified on linux only; this is a mac launch.
    let code = run_on(
        [OsString::from("run"), record, "fake-read".into(), "--".into(), marker.clone().into_os_string()],
        "mac",
    );
    assert_eq!(code, 1);
    assert!(!marker.exists(), "the agent must not have started");
}

#[test]
fn run_without_extra_arguments_and_with_a_missing_or_broken_record() {
    let dir = scratch("plain");
    let quiet = dir.join("quiet.yaml");
    fs::write(&quiet, with_command("[true]")).unwrap();
    assert_eq!(run_on([OsString::from("run"), quiet.into_os_string(), "fake-read".into()], "linux"), 0);
    let failing = dir.join("failing.yaml");
    fs::write(&failing, with_command("[false]")).unwrap();
    assert_eq!(run_on([OsString::from("run"), failing.into_os_string(), "fake-read".into()], "linux"), 1);
    assert_eq!(run(args(&["run", "/nonexistent/reach.yaml", "x"])), 2);
}

#[test]
fn an_agent_that_cannot_be_started_exits_1() {
    let dir = scratch("nostart");
    let file = dir.join("reach.yaml");
    fs::write(
        &file,
        with_command("[/nonexistent/agent]"),
    )
    .unwrap();
    assert_eq!(run_on([OsString::from("run"), file.into_os_string(), "fake-read".into()], "linux"), 1);
}

#[test]
fn an_agent_killed_by_a_signal_exits_1() {
    let dir = scratch("signal");
    let file = dir.join("reach.yaml");
    fs::write(&file, with_command("[sh, -c, 'kill -9 $$']")).unwrap();
    assert_eq!(run_on([OsString::from("run"), file.into_os_string(), "fake-read".into()], "linux"), 1);
}

#[test]
fn this_os_is_one_the_record_knows() {
    assert!(OSES.contains(&this_os()));
    assert_eq!(os_name("macos"), "mac");
    assert_eq!(os_name("linux"), "linux");
    assert_eq!(os_name("windows"), "unsupported");
    assert_eq!(LEVELS.len(), 7);
}

#[test]
fn run_refuses_an_id_the_validation_would_refuse() {
    let dir = scratch("strict");
    let marker = dir.join("marker");
    let file = dir.join("reach.yaml");
    let cases = [
        (GOOD.replace("/tmp/research", "/tmp/../etc"), "fake-research"),
        (GOOD.replace("    reach: read\n", "    reach: read\n    output_dir: /tmp/x\n"), "fake-read"),
        (with_command("['']"), "fake-read"),
    ];
    for (text, id) in cases {
        fs::write(&file, text).unwrap();
        let code = run_on(
            [OsString::from("run"), file.clone().into_os_string(), id.into(), "--".into(), marker.clone().into_os_string()],
            "linux",
        );
        assert_eq!(code, 1);
        assert!(!marker.exists(), "the agent must not have started");
    }
}

#[test]
fn an_os_with_no_cells_is_refused() {
    assert!(plan(&record(GOOD), "fake-read", "unsupported").is_err());
}
