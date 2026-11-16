//! `converge gold-band-roles` against a temporary critique directory, registry, closing file, keep file and profiles
//! directory, with a stub process table; one test renders the kit's own six roles.

use crate::cli::run_seeing;
use crate::converge::{Ran, Runner};
use crate::gold_band::{self, Proc, Processes};
use crate::Failure;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static NEXT: AtomicUsize = AtomicUsize::new(0);

const ALPHA: &str = "  - {file: alpha-critic, id: pf-alpha, name: Alpha critic, summary: 'Asks about {id} and {{x}}.', source: alpha.md, lenses: [first-lens, second-lens]}\n";
const BETA: &str = "  - {file: beta-critic, id: pf-beta, name: Beta critic, summary: 'Checks ${HOME}.', source: beta.md, lenses: []}\n";

const SOURCES: [(&str, &str); 7] = [
    ("roles/alpha.md", "Alpha role: {\"a\": {1}} {{x}} ${HOME} {id}\n"),
    // No trailing newline, so both branches of the section separator run.
    ("roles/beta.md", "Beta role: {id}"),
    ("prompts/preamble.md", "Preamble {{x}}\n"),
    ("prompts/output-contract.md", "Contract ${HOME}\n"),
    ("schemas/finding.schema.json", "{\n  \"a\": {1}\n}\n\n"),
    ("lenses/first-lens.md", "First lens {id}\n"),
    ("lenses/second-lens.md", "Second lens {{x}}\n"),
];

const EXPECTED_ALPHA: &str = "---\nid: pf-alpha\nname: Alpha critic\nsummary: Asks about {id} and {{x}}.\ncreatedAt: 2025-01-01T00:00:00Z\nupdatedAt: 2025-01-01T00:00:00Z\ndynamicTemplate: false\n---\n\nAlpha role: {\"a\": {1}} {{x}} ${HOME} {id}\n\nPreamble {{x}}\n\nContract ${HOME}\n\n```json\n{\n  \"a\": {1}\n}\n```\n\nFirst lens {id}\n\nSecond lens {{x}}\n\nClosing {id} {{x}}\n\n";
const EXPECTED_BETA: &str = "---\nid: pf-beta\nname: Beta critic\nsummary: Checks ${HOME}.\ncreatedAt: 2025-01-01T00:00:00Z\nupdatedAt: 2025-01-01T00:00:00Z\ndynamicTemplate: false\n---\n\nBeta role: {id}\n\nPreamble {{x}}\n\nContract ${HOME}\n\n```json\n{\n  \"a\": {1}\n}\n```\n\nClosing {id} {{x}}\n\n";

/// A process table: each read answers the next list, and the last one repeats.
struct Table {
    procs: Mutex<Vec<Vec<Proc>>>,
}

impl Table {
    fn none() -> Table {
        Table { procs: Mutex::new(Vec::new()) }
    }

    fn reads(lists: Vec<Vec<Proc>>) -> Table {
        Table { procs: Mutex::new(lists) }
    }
}

impl Processes for Table {
    fn list(&self) -> Vec<Proc> {
        let mut lists = self.procs.lock().unwrap();
        if lists.len() > 1 {
            lists.remove(0)
        } else {
            lists.first().cloned().unwrap_or_default()
        }
    }
}

struct NoRunner;

impl Runner for NoRunner {
    fn run(&mut self, program: &str, _: &[String], _: &[(String, Option<String>)]) -> Result<Ran, Failure> {
        panic!("gold-band-roles ran {program}")
    }
}

struct Machine {
    root: PathBuf,
    profiles: PathBuf,
}

impl Machine {
    fn new(roles: &str, retired: Option<&str>, keep: &str) -> Machine {
        let root = std::env::temp_dir().join(format!("agent-kit-roles-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        for (relative, text) in SOURCES {
            let path = root.join("critique").join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fs::write(root.join("closing.md"), "Closing {id} {{x}}\n").unwrap();
        fs::write(root.join("keep.yaml"), keep).unwrap();
        let retired = retired.map_or(String::new(), |retired| format!("  retired_roles: {retired}\n"));
        fs::write(root.join("registry.yaml"), format!("gold_band:\n  roles_source: critique\n  roles:\n{roles}{retired}")).unwrap();
        let profiles = root.join("home/.gold-band/context/profiles");
        Machine { root, profiles }
    }

    fn standard() -> Machine {
        Machine::new(&format!("{ALPHA}{BETA}"), None, "keep: []\n")
    }

    fn profile(&self, stem: &str) -> PathBuf {
        self.profiles.join(format!("{stem}.md"))
    }

    fn hand_made(&self, stem: &str, text: &str) {
        fs::create_dir_all(&self.profiles).unwrap();
        fs::write(self.profile(stem), text).unwrap();
    }

    fn args(&self, extra: &[&str]) -> Vec<String> {
        let path = |name: &str| self.root.join(name).display().to_string();
        let profiles = self.profiles.display().to_string();
        let mut args: Vec<String> = ["converge", "gold-band-roles", "--keep", &path("keep.yaml"), "--registry", &path("registry.yaml"), "--profiles", &profiles, "--critique", &path("critique"), "--closing", &path("closing.md")]
            .iter()
            .map(|text| text.to_string())
            .collect();
        args.extend(extra.iter().map(|text| text.to_string()));
        args
    }

    fn run(&self, extra: &[&str]) -> (i32, String) {
        self.run_with(&self.args(extra), &Table::none(), &[("AGENT_KIT_GOLD_BAND_RUNNING", "0")])
    }

    fn run_with(&self, args: &[String], table: &dyn Processes, vars: &[(&str, &str)]) -> (i32, String) {
        let home = self.root.join("home").display().to_string();
        let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let env = move |name: &str| if name == "HOME" { Some(home.clone()) } else { vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()) };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let status = {
            let mut stdin: &[u8] = &[];
            let mut io = crate::Io { stdin: &mut stdin, out: &mut out, err: &mut err };
            run_seeing(args, &mut io, &env, &mut NoRunner, table)
        };
        (status, String::from_utf8(out).unwrap() + &String::from_utf8(err).unwrap())
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.profiles).map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        names.sort();
        names
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn the_first_run_adds_every_role_rendered_verbatim_and_a_second_changes_nothing() {
    let machine = Machine::standard();
    assert_eq!(machine.run(&[]), (0, "added alpha-critic\nadded beta-critic\n".to_string()));
    assert_eq!(fs::read_to_string(machine.profile("alpha-critic")).unwrap(), EXPECTED_ALPHA);
    assert_eq!(fs::read_to_string(machine.profile("beta-critic")).unwrap(), EXPECTED_BETA);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(fs::metadata(&machine.profiles).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(machine.run(&[]), (0, "profiles unchanged\n".to_string()));
    fs::write(machine.profile("beta-critic"), "edited by hand").unwrap();
    assert_eq!(machine.run(&[]), (0, "changed beta-critic\n".to_string()));
    assert_eq!(fs::read_to_string(machine.profile("beta-critic")).unwrap(), EXPECTED_BETA);
    assert!(!machine.files().iter().any(|name| name.ends_with(".agent-kit-tmp")));
}

#[test]
fn undeclared_profiles_go_unless_kept_and_retired_ones_go_even_when_kept() {
    let machine = Machine::new(&format!("{ALPHA}{BETA}"), Some("[old-critic, pf-gone]"), "keep: [mine, mine2.md, old-critic, renamed]\n");
    machine.hand_made("stray", "---\nid: pf-stray\n---\n");
    machine.hand_made("mine", "kept by stem");
    machine.hand_made("mine2", "kept with its extension");
    machine.hand_made("old-critic", "---\nid: pf-old\n---\n");
    machine.hand_made("renamed", "---\nname: x\nid: pf-gone\n---\n");
    machine.hand_made("pf-builtin-x", "---\nid: pf-builtin-x\n---\n");
    fs::create_dir_all(machine.profiles.join("dir.md")).unwrap();
    fs::write(machine.profiles.join("notes.txt"), "not markdown").unwrap();
    fs::write(machine.profile("undecodable"), [0xff, 0xfe]).unwrap();
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 0, "{out}");
    assert_eq!(out, "added alpha-critic\nadded beta-critic\nremoved old-critic (retired)\nremoved renamed (retired)\nremoved stray (undeclared)\nremoved undecodable (undeclared)\n");
    assert_eq!(machine.files(), ["alpha-critic.md", "beta-critic.md", "dir.md", "mine.md", "mine2.md", "notes.txt", "pf-builtin-x.md"]);
}

#[test]
fn check_prints_the_changes_and_writes_nothing_even_while_gold_band_runs() {
    let machine = Machine::standard();
    let running = Table::reads(vec![vec![Proc { pid: 7, exe: "/opt/gold-band-desktop".into(), data: None }]]);
    let (status, out) = machine.run_with(&machine.args(&["--check"]), &running, &[]);
    assert_eq!((status, out.as_str()), (0, "added alpha-critic\nadded beta-critic\n"));
    assert!(!machine.profiles.exists());
    machine.hand_made("stray", "x");
    assert_eq!(machine.run(&["--check"]).1, "added alpha-critic\nadded beta-critic\nremoved stray (undeclared)\n");
    assert!(machine.profile("stray").exists());
}

#[test]
fn a_running_instance_refuses_the_write_and_one_that_starts_midway_stops_it() {
    let machine = Machine::standard();
    let (status, out) = machine.run_with(&machine.args(&[]), &Table::none(), &[("AGENT_KIT_GOLD_BAND_RUNNING", "1")]);
    assert_eq!(status, 4);
    assert_eq!(out, "gold-band-roles: Gold Band instance gold-band-personal (set by AGENT_KIT_GOLD_BAND_RUNNING) is running; quit it and re-run setup (nothing was written)\n");
    // Not running at the first check or the first group, running at the second.
    let data = machine.root.join("home/.gold-band");
    let app = Proc { pid: 9, exe: "/usr/lib/Gold Band".into(), data: Some(data.clone()) };
    let midway = Table::reads(vec![vec![], vec![], vec![app]]);
    let (status, out) = machine.run_with(&machine.args(&[]), &midway, &[]);
    assert_eq!(status, 4, "{out}");
    let shown = fs::canonicalize(&data).unwrap_or(data);
    assert!(out.ends_with(&format!(
        "gold-band-roles: Gold Band instance gold-band-personal (process 9, data {}) started during setup; quit it and re-run setup\ngold-band-roles: written: alpha-critic.md\ngold-band-roles: not yet written: beta-critic.md\n",
        shown.display()
    )), "{out}");
    // An app on another data directory, or another program, does not hold ours.
    let elsewhere = Table::reads(vec![vec![Proc { pid: 3, exe: "gold-band-desktop".into(), data: Some("/elsewhere/.gold-band".into()) }, Proc { pid: 4, exe: "vim".into(), data: None }]]);
    assert_eq!(machine.run_with(&machine.args(&[]), &elsewhere, &[]).0, 0);
    // A stop before any group is written says so.
    let machine = Machine::standard();
    let first = Table::reads(vec![vec![], vec![Proc { pid: 5, exe: "gold-band-desktop".into(), data: None }]]);
    let (status, out) = machine.run_with(&machine.args(&[]), &first, &[]);
    assert_eq!(status, 4);
    assert!(out.contains("gold-band-roles: written: nothing\ngold-band-roles: not yet written: alpha-critic.md, beta-critic.md\n"), "{out}");
}

#[test]
fn a_role_that_breaks_a_rule_is_refused_with_every_problem() {
    let roles = "  - {file: Bad_Name, id: x, name: 1, summary: 'line # comment', source: a.md, lenses: oops, extra: 1}\n  - {file: pf-builtin-y, id: pf-builtin-y, name: N, summary: S, source: s, lenses: []}\n  - {file: dup, id: pf-a, name: N, summary: S, source: s, lenses: [], 5: n}\n  - {file: dup, id: pf-a, name: N, summary: S, source: s, lenses: []}\n  - {file: gone, id: pf-gone, name: N, summary: S, source: s, lenses: []}\n  - {name: N, summary: S, source: s, lenses: []}\n";
    let machine = Machine::new(roles, Some("[gone]"), "keep: []\n");
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 1);
    assert_eq!(
        out,
        "gold-band-roles: Bad_Name: file must match ^[a-z0-9]+(-[a-z0-9]+)*$\ngold-band-roles: x: id must start with pf-\ngold-band-roles: x: name must be a string\ngold-band-roles: x: summary must be a plain single-line scalar\ngold-band-roles: x: lenses must be a list of names\ngold-band-roles: x: unknown field extra\ngold-band-roles: pf-builtin-y: file must not name a built-in profile\ngold-band-roles: pf-builtin-y: id must not name a built-in profile\ngold-band-roles: pf-a: unknown field 5\ngold-band-roles: dup: duplicate file\ngold-band-roles: pf-a: duplicate id\ngold-band-roles: pf-gone: declared and retired at once\ngold-band-roles: ?: file must match ^[a-z0-9]+(-[a-z0-9]+)*$\ngold-band-roles: ?: id must start with pf-\n"
    );
    assert!(!machine.profiles.exists());
}

#[test]
fn a_registry_or_source_that_cannot_be_read_exits_3_and_touches_nothing() {
    for (registry, message) in [
        ("[1, 2]", "is not a mapping"),
        ("other: 1", "has no gold_band mapping"),
        ("gold_band: {roles_source: [x], roles: []}", "gold_band.roles_source must be a path"),
        ("gold_band: {roles: [1]}", "gold_band.roles must be a list of mappings"),
        ("gold_band: {roles: [], retired_roles: [1]}", "gold_band.retired_roles must be a list of stems or ids"),
        ("gold_band: [", "cannot be read"),
    ] {
        let machine = Machine::standard();
        fs::write(machine.root.join("registry.yaml"), registry).unwrap();
        let (status, out) = machine.run(&[]);
        assert_eq!(status, 3, "{registry}");
        assert!(out.starts_with("gold-band-roles: cannot converge: ") && out.contains(message), "{out}");
    }
    let machine = Machine::standard();
    fs::remove_file(machine.root.join("registry.yaml")).unwrap();
    assert!(machine.run(&[]).1.contains("registry.yaml cannot be read"));
    // A missing source: nothing is written, the stray profile stays.
    let machine = Machine::standard();
    machine.hand_made("stray", "x");
    fs::remove_file(machine.root.join("critique/lenses/second-lens.md")).unwrap();
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 3);
    assert!(out.contains("second-lens.md: "), "{out}");
    assert_eq!(machine.files(), ["stray.md"]);
    // No roles source and no --critique.
    let machine = Machine::standard();
    fs::write(machine.root.join("registry.yaml"), format!("gold_band:\n  roles:\n{ALPHA}")).unwrap();
    let args: Vec<String> = machine.args(&[]).into_iter().filter(|arg| !arg.ends_with("/critique") && arg != "--critique").collect();
    let (status, out) = machine.run_with(&args, &Table::none(), &[("AGENT_KIT_GOLD_BAND_RUNNING", "0")]);
    assert_eq!(status, 3);
    assert!(out.contains("gold_band.roles_source must be a path"), "{out}");
}

#[test]
fn a_builtin_under_a_declared_stem_a_write_error_and_a_bad_keep_file_are_refused() {
    let machine = Machine::standard();
    machine.hand_made("alpha-critic", "---\nid: pf-builtin-alpha\n---\n");
    assert_eq!(machine.run(&[]), (1, "gold-band-roles: alpha-critic: the existing profile is a built-in; nothing was written\n".to_string()));
    // A file where the directory goes: the directory cannot be made.
    let machine = Machine::standard();
    fs::create_dir_all(machine.profiles.parent().unwrap()).unwrap();
    fs::write(&machine.profiles, "not a directory").unwrap();
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 3);
    assert!(out.ends_with("\n") && out.contains("\ngold-band-roles: cannot converge: "), "{out}");
    // A directory where a profile goes: the rename fails and leaves no staged file.
    let machine = Machine::standard();
    fs::create_dir_all(machine.profile("beta-critic")).unwrap();
    fs::write(machine.profile("beta-critic").join("inside"), "x").unwrap();
    assert_eq!(machine.run(&[]).0, 3);
    assert!(!machine.files().iter().any(|name| name.ends_with(".agent-kit-tmp")), "{:?}", machine.files());
    // A removal that fails.
    let machine = Machine::standard();
    fs::create_dir_all(machine.profile("stray")).unwrap();
    fs::write(machine.profile("stray").join("inside"), "x").unwrap();
    machine.hand_made("stray2", "x");
    let machine2 = Machine::standard();
    machine2.hand_made("gone", "x");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&machine2.profiles, fs::Permissions::from_mode(0o500)).unwrap();
    let (status, _) = machine2.run(&[]);
    fs::set_permissions(&machine2.profiles, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(status, 3);
    let machine = Machine::standard();
    fs::write(machine.root.join("keep.yaml"), "keep: 5\n").unwrap();
    assert_eq!(machine.run(&[]).0, 1);
}

#[test]
fn a_profile_id_is_the_first_front_matter_blocks() {
    assert_eq!(super::profile_id(Some("---\nname: x\nid: pf-a\n---\n")), Some("pf-a".to_string()));
    for text in ["no front matter", "", "---\nplain line\n---\n", "---\nname: x\n---\nid: pf-late\n", "---\nid:\n---\n", "---\nname: x\n"] {
        assert_eq!(super::profile_id(Some(text)), None, "{text:?}");
    }
    assert_eq!(super::profile_id(None), None);
}

#[test]
fn the_profiles_path_defaults_to_gold_band_home_and_the_kits_own_files() {
    let machine = Machine::standard();
    let kit = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let registry = kit.join("registry/estate-tooling.yaml").display().to_string();
    let band = machine.root.join("band").display().to_string();
    let keep = machine.root.join("keep.yaml").display().to_string();
    let args: Vec<String> = ["converge", "gold-band-roles", "--keep", &keep, "--registry", &registry].iter().map(|text| text.to_string()).collect();
    let vars = [("GOLD_BAND_HOME", band.as_str()), ("AGENT_KIT_GOLD_BAND_RUNNING", "0")];
    let (status, out) = machine.run_with(&args, &Table::none(), &vars);
    assert_eq!(status, 0, "{out}");
    let profiles = machine.root.join("band/.gold-band/context/profiles");
    let written: Vec<_> = fs::read_dir(&profiles).unwrap().flatten().collect();
    assert_eq!(written.len(), 6, "the kit declares six roles");
    for entry in written {
        let text = fs::read_to_string(entry.path()).unwrap();
        assert!(text.contains("dynamicTemplate: false\n") && text.contains("```json\n{"), "{}", entry.path().display());
    }
    assert_eq!(machine.run_with(&args, &Table::none(), &vars), (0, "profiles unchanged\n".to_string()));
    let (status, out) = machine.run_with(&["converge".to_string(), "gold-band-roles".to_string()], &Table::none(), &vars);
    assert_eq!((status, out.as_str()), (2, "agent-kit-merge: the following arguments are required: --keep, --registry\n"));
}

#[test]
fn the_data_directory_and_the_system_process_tables() {
    assert_eq!(gold_band::data_dir(Some(" "), "/h"), PathBuf::from("/h/.gold-band"));
    assert_eq!(gold_band::data_dir(Some("/g"), "/h"), PathBuf::from("/g/.gold-band"));
    assert_eq!(gold_band::parse_ps("  12 /Applications/Gold Band\nnot-a-pid x\n 13\n"), [Proc { pid: 12, exe: "/Applications/Gold Band".into(), data: None }, Proc { pid: 13, exe: String::new(), data: None }]);
    // This test's own process is in both system tables, with its data directory where /proc shows it.
    let me = std::process::id();
    let listed = gold_band::SystemProcesses.list();
    assert!(listed.iter().any(|proc| proc.pid == me && proc.data.is_some()), "{listed:?}");
    assert!(gold_band::ps_table().iter().any(|proc| proc.pid == me));
    assert!(gold_band::system_table(Path::new("/nonexistent-proc")).iter().any(|proc| proc.pid == me));
    assert!(gold_band::proc_table(Path::new("/nonexistent-proc"), 0).is_empty());
    assert!(gold_band::proc_table(Path::new("/proc"), u32::MAX - 7).is_empty());
}

#[test]
fn a_removal_is_a_write_group_too_and_a_second_run_finishes() {
    let machine = Machine::standard();
    assert_eq!(machine.run(&[]).0, 0);
    machine.hand_made("stale", "old\n");
    // Clear at the first check, running before the removal group.
    let midway = Table::reads(vec![vec![], vec![Proc { pid: 7, exe: "gold-band-desktop".into(), data: None }]]);
    let (status, out) = machine.run_with(&machine.args(&[]), &midway, &[]);
    assert_eq!(status, 4, "{out}");
    assert!(out.ends_with("gold-band-roles: written: nothing\ngold-band-roles: not yet written: stale.md\n"), "{out}");
    assert!(machine.profile("stale").exists());
    assert_eq!(machine.run(&[]), (0, "removed stale (undeclared)\n".to_string()));
    assert!(!machine.profile("stale").exists());
}
