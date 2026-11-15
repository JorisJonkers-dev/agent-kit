use super::*;
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plg-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn ctx(home: &Path) -> Ctx {
    let home = home.to_string_lossy().into_owned();
    Ctx {
        projects: format!("{home}/Projects"),
        home,
        git: "git".to_string(),
        timeout: Duration::from_secs(10),
    }
}

fn bash(cwd: &str, command: &str) -> String {
    json!({"hook_event_name": "PreToolUse", "tool_name": "Bash", "cwd": cwd, "tool_input": {"command": command}}).to_string()
}

fn reason(out: Option<String>) -> Option<String> {
    let out = out?;
    let v: Value = serde_json::from_str(&out).unwrap();
    let d = &v["hookSpecificOutput"];
    assert_eq!(d["hookEventName"], "PreToolUse");
    assert_eq!(d["permissionDecision"], "deny");
    Some(d["permissionDecisionReason"].as_str().unwrap().to_string())
}

fn layout(name: &str) -> (PathBuf, Ctx) {
    let home = scratch(name);
    std::fs::create_dir_all(home.join("Projects")).unwrap();
    let c = ctx(&home);
    (home, c)
}

fn git_init(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    assert!(Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(path)
        .status()
        .unwrap()
        .success());
}

fn denied(c: &Ctx, cwd: &str, command: &str) -> Option<String> {
    reason(run(&bash(cwd, command), c))
}

#[test]
fn a_clone_into_the_owner_folder_is_allowed() {
    let (home, c) = layout("clone-ok");
    let cwd = format!("{}/Projects/JorisJonkers-dev", home.display());
    assert_eq!(
        denied(
            &c,
            &cwd,
            "git clone git@github.com:JorisJonkers-dev/agent-kit.git"
        ),
        None
    );
}

#[test]
fn a_clone_into_tmp_is_denied_with_the_path_it_must_use() {
    let (home, c) = layout("clone-tmp");
    let r = denied(
        &c,
        "/tmp",
        "git clone https://github.com/JorisJonkers-dev/agent-kit",
    )
    .unwrap();
    assert!(r.starts_with("projects-layout-guard: clone JorisJonkers-dev/agent-kit into "));
    assert!(r.contains(&format!(
        "{}/Projects/JorisJonkers-dev/agent-kit, not /tmp/agent-kit.",
        home.display()
    )));
    assert!(
        r.ends_with("If ".to_string().as_str())
            || r.ends_with("use that clone. See the projects-layout skill.")
    );
}

#[test]
fn a_gh_clone_after_cd_into_the_owner_folder_is_allowed() {
    let (_home, c) = layout("gh-cd");
    assert_eq!(
        denied(
            &c,
            "/tmp",
            "cd ~/Projects/ESA-Blueshell && gh repo clone ESA-Blueshell/website"
        ),
        None
    );
}

#[test]
fn a_gh_clone_to_the_projects_root_is_denied() {
    let (_home, c) = layout("gh-root");
    assert!(denied(&c, "/tmp", "gh repo clone acme/widget ~/Projects/widget").is_some());
}

#[test]
fn gh_clone_forms() {
    let (_home, c) = layout("gh-forms");
    // A bare repo name needs the network to resolve: let through.
    assert_eq!(denied(&c, "/tmp", "gh repo clone widget"), None);
    assert_eq!(denied(&c, "/tmp", "gh repo clone"), None);
    // The `.git` suffix of a short spec is dropped; a URL works too.
    assert!(denied(&c, "/tmp", "gh repo clone acme/widget.git")
        .unwrap()
        .contains("clone acme/widget into "));
    assert!(denied(
        &c,
        "/tmp",
        "gh repo clone https://github.com/acme/w.git.git"
    )
    .unwrap()
    .contains("clone acme/w into "));
    // Everything after `--` belongs to git, not to the target.
    assert!(denied(&c, "/tmp", "gh repo clone acme/widget -- --depth 1").is_some());
    assert!(denied(&c, "/tmp", "gh repo clone -u up acme/widget").is_some());
}

#[test]
fn a_clone_from_another_host_is_left_alone() {
    let (_home, c) = layout("other-host");
    assert_eq!(denied(&c, "/tmp", "git clone https://gitlab.com/a/b"), None);
    assert_eq!(denied(&c, "/tmp", "git clone"), None);
    assert_eq!(denied(&c, "/tmp", "git clone --depth 1"), None);
}

#[test]
fn a_command_that_only_mentions_clone_is_left_alone() {
    let (_home, c) = layout("mention");
    assert_eq!(denied(&c, "/tmp", "echo 'git clone x' | grep clone"), None);
    assert_eq!(denied(&c, "/tmp", "git status"), None);
    assert_eq!(denied(&c, "/tmp", "git worktree list"), None);
    assert_eq!(denied(&c, "/tmp", "git"), None);
    assert_eq!(denied(&c, "/tmp", "gh repo view acme/widget"), None);
    assert_eq!(denied(&c, "/tmp", "gh"), None);
}

#[test]
fn clone_option_values_are_skipped_and_targets_resolve() {
    let (home, c) = layout("clone-opts");
    let p = home.display();
    let cwd = format!("{p}/Projects/acme");
    assert_eq!(
        denied(
            &c,
            &cwd,
            "git clone --depth 1 -b main https://github.com/acme/widget"
        ),
        None
    );
    assert_eq!(
        denied(
            &c,
            &cwd,
            "git clone --depth=1 https://github.com/acme/widget widget"
        ),
        None
    );
    assert_eq!(
        denied(
            &c,
            "/tmp",
            "git clone -- https://github.com/acme/widget ~/Projects/acme/widget"
        ),
        None
    );
    assert_eq!(
        denied(
            &c,
            "/tmp",
            &format!("git clone https://github.com/acme/widget {p}/Projects/acme/../acme/widget")
        ),
        None
    );
    assert!(denied(
        &c,
        &cwd,
        "git clone https://github.com/acme/widget elsewhere"
    )
    .unwrap()
    .contains("not "));
    // git's own options before the subcommand.
    assert_eq!(
        denied(
            &c,
            "/tmp",
            "git -C ~/Projects/acme clone https://github.com/acme/widget"
        ),
        None
    );
    assert!(denied(
        &c,
        "/tmp",
        "git -c a=b --no-pager clone https://github.com/acme/widget"
    )
    .is_some());
    assert!(denied(&c, "/tmp", "git -C").is_none());
}

#[test]
fn wrappers_environment_and_nested_shells_are_looked_through() {
    let (_home, c) = layout("wrappers");
    let url = "https://github.com/acme/widget";
    assert!(denied(&c, "/tmp", &format!("FOO=1 BAR=2 git clone {url}")).is_some());
    assert!(denied(&c, "/tmp", &format!("env time nice git clone {url}")).is_some());
    assert!(denied(&c, "/tmp", &format!("/usr/bin/git clone {url}")).is_some());
    assert!(denied(&c, "/tmp", &format!("bash -c 'git clone {url}'")).is_some());
    assert!(denied(&c, "/tmp", &format!("sh -lc \"git clone {url}\"")).is_some());
    assert_eq!(denied(&c, "/tmp", "bash -c"), None);
    assert_eq!(denied(&c, "/tmp", "bash script.sh arg"), None);
    assert_eq!(denied(&c, "/tmp", "FOO=1"), None);
    assert_eq!(denied(&c, "/tmp", ""), None);
    assert!(denied(&c, "/tmp", &format!("true; (git clone {url})")).is_some());
    assert!(denied(&c, "/tmp", &format!("false || git clone {url}")).is_some());
}

#[test]
fn cd_tracks_the_working_directory() {
    let (home, c) = layout("cd");
    let p = home.display();
    let url = "https://github.com/acme/widget";
    assert_eq!(
        denied(
            &c,
            "/tmp",
            &format!("cd {p}/Projects/acme; git clone {url}")
        ),
        None
    );
    assert_eq!(
        denied(
            &c,
            "/tmp",
            &format!("cd {p}/Projects/acme; cd -; git clone {url}")
        ),
        None
    );
    assert!(denied(&c, "/tmp", &format!("cd; git clone {url}"))
        .unwrap()
        .contains(&format!("not {p}/widget.")));
}

#[test]
fn shell_syntax_that_cannot_be_parsed_is_let_through() {
    let (_home, c) = layout("parse");
    assert_eq!(
        denied(&c, "/tmp", "git clone 'https://github.com/acme/widget"),
        None
    );
    assert_eq!(
        denied(&c, "/tmp", "git clone https://github.com/acme/widget \\"),
        None
    );
    assert_eq!(
        denied(&c, "/tmp", "git clone \"https://github.com/acme/widget\\"),
        None
    );
}

#[test]
fn a_comment_ends_the_command() {
    let (_home, c) = layout("comment");
    assert_eq!(
        denied(
            &c,
            "/tmp",
            "echo hi # git clone https://github.com/acme/widget"
        ),
        None
    );
    assert_eq!(
        denied(
            &c,
            "/tmp",
            "echo hi;# git clone https://github.com/acme/widget"
        ),
        None
    );
    assert!(denied(
        &c,
        "/tmp",
        "# note\ngit clone https://github.com/acme/widget"
    )
    .is_some());
    // A newline is plain whitespace, so the next line joins the same command.
    assert_eq!(
        denied(
            &c,
            "/tmp",
            "echo a#b\ngit clone https://github.com/acme/widget"
        ),
        None
    );
    assert_eq!(denied(&c, "/tmp", "# only a comment"), None);
}

#[test]
fn a_worktree_next_to_the_repository_is_denied() {
    let (home, c) = layout("wt-bad");
    let repo = home.join("Projects/acme/widget");
    git_init(&repo);
    let r = denied(
        &c,
        repo.to_str().unwrap(),
        "git worktree add -b feat/x ../widget-x",
    )
    .unwrap();
    assert!(r.contains(&format!(
        "{}/Projects/acme/worktrees/widget/widget-x",
        home.display()
    )));
    assert!(r.contains(&format!("not {}/Projects/acme/widget-x.", home.display())));
}

#[test]
fn a_worktree_in_the_owners_worktrees_folder_is_allowed() {
    let (home, c) = layout("wt-ok");
    let repo = home.join("Projects/acme/widget");
    git_init(&repo);
    let cwd = repo.to_str().unwrap();
    assert_eq!(
        denied(
            &c,
            cwd,
            "git worktree add ~/Projects/acme/worktrees/widget/feat-x -b feat/x"
        ),
        None
    );
    assert_eq!(denied(&c, cwd, "git worktree add"), None);
    assert!(denied(&c, cwd, "git worktree add /")
        .unwrap()
        .contains("(for example"));
    assert!(denied(&c, cwd, "git worktree add /")
        .unwrap()
        .contains("/<name>)"));
}

#[test]
fn a_codex_command_array_through_bash_c_is_checked() {
    let (home, c) = layout("codex");
    let repo = home.join("Projects/acme/widget");
    git_init(&repo);
    let payload = json!({"tool_name": "Bash", "cwd": "/tmp", "tool_input": {"command": ["bash", "-lc", format!("git -C {} worktree add .worktrees/z", repo.display())]}});
    assert!(reason(run(&payload.to_string(), &c)).is_some());
    let empty = json!({"tool_name": "Bash", "cwd": "/tmp", "tool_input": {"command": []}});
    assert_eq!(run(&empty.to_string(), &c), None);
    // A command array holding a non-string is a payload the guard cannot read.
    let odd = json!({"tool_name": "Bash", "cwd": "/tmp", "tool_input": {"command": ["git", 1]}});
    assert_eq!(run(&odd.to_string(), &c), None);
}

#[test]
fn a_bare_repository_is_identified_by_its_own_path() {
    let (home, c) = layout("bare");
    let repo = home.join("Projects/acme/widget");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(&repo)
        .status()
        .unwrap()
        .success());
    let r = denied(&c, repo.to_str().unwrap(), "git worktree add ../x").unwrap();
    assert!(r.contains("worktrees of acme/widget go in "));
}

#[test]
fn a_repository_outside_the_layout_is_identified_by_its_origin() {
    let (home, c) = layout("origin");
    let repo = home.join("elsewhere/widget");
    git_init(&repo);
    let cwd = repo.to_str().unwrap();
    // No origin: unknown repository, allowed.
    assert_eq!(denied(&c, cwd, "git worktree add ../x"), None);
    assert!(Command::new("git")
        .args([
            "-C",
            cwd,
            "remote",
            "add",
            "origin",
            "git@github.com:acme/widget.git"
        ])
        .status()
        .unwrap()
        .success());
    assert!(denied(&c, cwd, "git worktree add ../x")
        .unwrap()
        .contains("worktrees of acme/widget go in "));
    // A repository directly under Projects is not <owner>/<repo>.
    let shallow = home.join("Projects/loose");
    git_init(&shallow);
    assert_eq!(
        denied(&c, shallow.to_str().unwrap(), "git worktree add ../x"),
        None
    );
    // Not a repository at all.
    assert_eq!(
        denied(&c, home.to_str().unwrap(), "git worktree add ../x"),
        None
    );
}

#[test]
fn a_git_that_cannot_run_or_is_too_slow_allows_the_call() {
    let (home, mut c) = layout("slow-git");
    let repo = home.join("Projects/acme/widget");
    git_init(&repo);
    let cwd = repo.to_str().unwrap();
    c.git = home.join("no-such-git").to_string_lossy().into_owned();
    assert_eq!(denied(&c, cwd, "git worktree add ../x"), None);
    let fake = home.join("slow-git.sh");
    std::fs::write(&fake, "#!/bin/sh\nsleep 5\n").unwrap();
    Command::new("chmod").arg("+x").arg(&fake).status().unwrap();
    c.git = fake.to_string_lossy().into_owned();
    c.timeout = Duration::from_millis(100);
    assert_eq!(denied(&c, cwd, "git worktree add ../x"), None);
    // A git that answers with nothing is an unknown repository.
    let quiet = home.join("quiet-git.sh");
    std::fs::write(&quiet, "#!/bin/sh\nexit 1\n").unwrap();
    Command::new("chmod")
        .arg("+x")
        .arg(&quiet)
        .status()
        .unwrap();
    c.git = quiet.to_string_lossy().into_owned();
    c.timeout = Duration::from_secs(10);
    assert_eq!(denied(&c, cwd, "git worktree add ../x"), None);
}

#[test]
fn enter_worktree_with_a_name_is_denied_and_with_a_path_allowed() {
    let (home, c) = layout("enter");
    let named = json!({"tool_name": "EnterWorktree", "cwd": "/tmp", "tool_input": {"name": "x"}});
    let r = reason(run(&named.to_string(), &c)).unwrap();
    assert!(r.starts_with(
        "projects-layout-guard: EnterWorktree with `name` creates .claude/worktrees/"
    ));
    assert!(r.ends_with("with `path`. See the projects-layout skill."));
    let no_input = json!({"tool_name": "EnterWorktree"});
    assert!(reason(run(&no_input.to_string(), &c)).is_some());
    let empty_path = json!({"tool_name": "EnterWorktree", "tool_input": {"path": ""}});
    assert!(reason(run(&empty_path.to_string(), &c)).is_some());
    let path = json!({"tool_name": "EnterWorktree", "tool_input": {"path": format!("{}/Projects/acme/worktrees/widget/x", home.display())}});
    assert_eq!(run(&path.to_string(), &c), None);
}

#[test]
fn payloads_the_guard_cannot_read_are_let_through() {
    let (_home, c) = layout("payloads");
    assert_eq!(run("{not json", &c), None);
    assert_eq!(run("[1]", &c), None);
    assert_eq!(
        run(
            "{\"tool_name\": \"Bash\", \"tool_input\": \"git clone\"}",
            &c
        ),
        None
    );
    assert_eq!(run("{\"tool_name\": 5, \"tool_input\": 0}", &c), None);
    assert_eq!(
        run(
            "{\"tool_name\": \"Bash\", \"tool_input\": {\"command\": 5}}",
            &c
        ),
        None
    );
    assert_eq!(run("{\"tool_name\": \"Bash\", \"cwd\": 5, \"tool_input\": {\"command\": \"git clone https://github.com/a/b\"}}", &c), None);
}

#[test]
fn a_missing_cwd_means_the_working_directory_of_the_hook() {
    let (_home, c) = layout("nocwd");
    let p = json!({"tool_name": "Bash", "tool_input": {"command": "git clone https://github.com/acme/widget"}});
    assert!(reason(run(&p.to_string(), &c)).is_some());
}

#[test]
fn truthiness_follows_python() {
    assert!(!truthy(&json!(null)));
    assert!(!truthy(&json!(false)));
    assert!(truthy(&json!(true)));
    assert!(!truthy(&json!(0)));
    assert!(truthy(&json!(2)));
    assert!(!truthy(&json!("")));
    assert!(truthy(&json!("a")));
    assert!(!truthy(&json!([])));
    assert!(truthy(&json!([1])));
    assert!(!truthy(&json!({})));
    assert!(truthy(&json!({"a": 1})));
}

fn run_main_with(home: Option<&str>, off: Option<&str>, input: &[u8]) -> (i32, String) {
    let mut out = Vec::new();
    let code = run_main(input, &mut out, |name| match name {
        "HOME" => home.map(str::to_string),
        "PROJECTS_LAYOUT_OFF" => off.map(str::to_string),
        _ => None,
    });
    (code, String::from_utf8(out).unwrap())
}

struct BadRead;
impl Read for BadRead {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("broken"))
    }
}

#[test]
fn the_executable_prints_a_deny_and_exits_zero() {
    let (home, _c) = layout("main");
    let h = home.to_str().unwrap();
    let payload = bash("/tmp", "git clone https://github.com/acme/widget");
    let (code, out) = run_main_with(Some(h), None, payload.as_bytes());
    assert_eq!(code, 0);
    assert!(out.ends_with('\n'));
    assert!(reason(Some(out.trim().to_string())).is_some());
    // HOME with a trailing slash is the same home.
    let with_slash = format!("{h}/");
    assert!(!run_main_with(Some(&with_slash), None, payload.as_bytes())
        .1
        .is_empty());
    // The kill switch, no ~/Projects, no HOME, unreadable input, a payload that is not JSON.
    assert_eq!(
        run_main_with(Some(h), Some("1"), payload.as_bytes()),
        (0, String::new())
    );
    assert!(!run_main_with(Some(h), Some("0"), payload.as_bytes())
        .1
        .is_empty());
    let bare = scratch("main-bare");
    assert_eq!(
        run_main_with(Some(bare.to_str().unwrap()), None, payload.as_bytes()),
        (0, String::new())
    );
    assert_eq!(
        run_main_with(None, None, payload.as_bytes()),
        (0, String::new())
    );
    assert_eq!(
        run_main_with(Some(h), None, b"\xff\xfe"),
        (0, String::new())
    );
    assert_eq!(
        run_main_with(Some(h), None, b"{not json"),
        (0, String::new())
    );
    let code = run_main(BadRead, Vec::new(), |name| {
        (name == "HOME").then(|| h.to_string())
    });
    assert_eq!(code, 0);
    // A home of "/" has no /Projects here, so the guard is off.
    assert_eq!(run_main_with(Some("/"), None, payload.as_bytes()).0, 0);
}

mod helpers {
    use crate::paths::*;
    use crate::shell::*;
    use crate::url::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn paths_match_os_path() {
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(join("/a/", "b"), "/a/b");
        assert_eq!(join("", "b"), "b");
        assert_eq!(join("/a", "/b"), "/b");
        assert_eq!(normpath(""), ".");
        assert_eq!(normpath("/a/./b//c/../d"), "/a/b/d");
        assert_eq!(normpath("//a"), "//a");
        assert_eq!(normpath("///a"), "/a");
        assert_eq!(normpath("a/../.."), "..");
        assert_eq!(normpath("../../a"), "../../a");
        assert_eq!(normpath("/.."), "/");
        assert_eq!(normpath("a/.."), ".");
        assert_eq!(norm("~", "/c", "/h"), "/h");
        assert_eq!(norm("~/x", "/c", "/h/"), "/h/x");
        assert_eq!(norm("~/x", "/c", ""), "/x");
        assert_eq!(norm("~", "/c", ""), "/");
        assert_eq!(norm("~bob", "/c", "/h"), "/c/~bob");
        assert_eq!(norm("x/../y", "/c", "/h"), "/c/y");
        assert_eq!(dirname("/a/b"), "/a");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(dirname("a"), "");
        assert_eq!(dirname("/a//b"), "/a");
        assert_eq!(basename("/a/b"), "b");
        assert_eq!(basename("b"), "b");
        assert_eq!(basename("/a/"), "");
        assert_eq!(relpath("/a/b/c", "/a/b"), "c");
        assert_eq!(relpath("/a", "/a/b/c"), "../..");
        assert_eq!(relpath("/a", "/a"), ".");
        assert_eq!(relpath("/x/y", "/a/b"), "../../x/y");
    }

    #[test]
    fn tokens_follow_shlex() {
        assert_eq!(tokens("a b\tc\nd"), Some(s(&["a", "b", "c", "d"])));
        assert_eq!(
            tokens("a;b&&c||d (e)"),
            Some(s(&["a", ";", "b", "&&", "c", "||", "d", "(", "e", ")"]))
        );
        assert_eq!(
            tokens("'a b' \"c d\" e\\ f"),
            Some(s(&["a b", "c d", "e f"]))
        );
        assert_eq!(tokens("\"a\\\"b\\\\c\\d\""), Some(s(&["a\"b\\c\\d"])));
        assert_eq!(tokens("'a\\b'"), Some(s(&["a\\b"])));
        assert_eq!(tokens("''"), Some(s(&[""])));
        assert_eq!(tokens("a'b'c"), Some(s(&["abc"])));
        assert_eq!(tokens(r"a\;b"), Some(s(&["a;b"])));
        assert_eq!(tokens("x;;y"), Some(s(&["x", ";;", "y"])));
        assert_eq!(tokens("a #c\nb"), Some(s(&["a", "b"])));
        assert_eq!(tokens("a;#c"), Some(s(&["a", ";"])));
        assert_eq!(tokens("a #c"), Some(s(&["a"])));
        assert_eq!(tokens("a\\ #c"), Some(s(&["a "])));
        assert_eq!(tokens("a'b' #c"), Some(s(&["ab"])));
        assert_eq!(tokens(""), Some(vec![]));
        assert_eq!(tokens("'a"), None);
        assert_eq!(tokens("a\\"), None);
        assert_eq!(tokens("\"a\\"), None);
        assert_eq!(tokens("\\"), None);
    }

    #[test]
    fn segments_split_on_separators() {
        assert_eq!(
            segments("a b; c | d && ';'"),
            vec![s(&["a", "b"]), s(&["c"]), s(&["d"])]
        );
        assert_eq!(segments("; ;"), Vec::<Vec<String>>::new());
        assert_eq!(segments("'a"), Vec::<Vec<String>>::new());
    }

    #[test]
    fn quote_follows_shlex() {
        assert_eq!(quote(""), "''");
        assert_eq!(quote("a-b/c.d_e@f%g+h=i:j,k"), "a-b/c.d_e@f%g+h=i:j,k");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\"'\"'s'");
    }

    fn gh(u: &str) -> Option<(&'static str, &'static str)> {
        github(u).map(|(o, r)| {
            (
                Box::leak(o.into_boxed_str()) as &str,
                Box::leak(r.into_boxed_str()) as &str,
            )
        })
    }

    #[test]
    fn github_urls() {
        assert_eq!(gh("https://github.com/a/b"), Some(("a", "b")));
        assert_eq!(gh("http://github.com/a/b.git"), Some(("a", "b")));
        assert_eq!(gh("https://github.com/a/b/"), Some(("a", "b")));
        assert_eq!(gh("https://github.com/a/b.git/"), Some(("a", "b")));
        assert_eq!(gh("https://github.com/a/.git"), Some(("a", ".git")));
        assert_eq!(gh("https://github.com/a/b.git.git"), Some(("a", "b.git")));
        assert_eq!(gh("https://tok@github.com/a/b"), Some(("a", "b")));
        assert_eq!(gh("https://@github.com/a/b"), None);
        assert_eq!(gh("ssh://git@github.com/a/b"), Some(("a", "b")));
        assert_eq!(gh("git://github.com/a/b"), Some(("a", "b")));
        assert_eq!(gh("git@github.com:a/b.git"), Some(("a", "b")));
        assert_eq!(gh("github.com:a/b"), Some(("a", "b")));
        assert_eq!(gh("a b@github.com:x/y"), None);
        assert_eq!(gh("@github.com:x/y"), None);
        assert_eq!(gh("https://github.com/a"), None);
        assert_eq!(gh("https://github.com//b"), None);
        assert_eq!(gh("https://github.com/a/"), None);
        assert_eq!(gh("https://github.com/a/b/c"), None);
        assert_eq!(gh("https://github.com/a/b c"), None);
        assert_eq!(gh("https://gitlab.com/a/b"), None);
        assert_eq!(gh("ftp://github.com/a/b"), None);
        assert_eq!(gh("acme/widget"), None);
        assert_eq!(gh("https://github.com"), None);
    }

    #[test]
    fn short_specs() {
        assert_eq!(
            short_spec("a/b.c-d_e"),
            Some(("a".to_string(), "b.c-d_e".to_string()))
        );
        assert_eq!(short_spec("a"), None);
        assert_eq!(short_spec("/b"), None);
        assert_eq!(short_spec("a/"), None);
        assert_eq!(short_spec("a/b/c"), None);
        assert_eq!(short_spec("a b/c"), None);
    }
}
