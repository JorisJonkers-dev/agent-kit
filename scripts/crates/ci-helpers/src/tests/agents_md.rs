//! The AGENTS.md lint, driven through its command line against tiny fake repositories.

use super::support::{run, Tmp};
use std::fs;

const CLEAN_POINTER: &str = "# Agent contract\n\nThe estate conventions live in one place:\nhttps://github.com/JorisJonkers-dev/workspace/blob/main/CLAUDE.md\n\nRead [the contract](https://example.com/contract) before a non-trivial change,\nand see [this section](#agent-contract) for the rest.\n\nRun `scripts/run.py` and read `docs/guide.md`.\n";

/// A repository named `name` under `tmp`; AGENTS.md holds `agents_md` unless None.
fn repo(tmp: &Tmp, name: &str, agents_md: Option<&str>, files: &[(&str, &str)]) -> String {
    fs::create_dir_all(tmp.join(name)).unwrap();
    for (rel, content) in files {
        tmp.write(&format!("{name}/{rel}"), content);
    }
    if let Some(text) = agents_md {
        tmp.write(&format!("{name}/AGENTS.md"), text);
    }
    tmp.join(name).display().to_string()
}

fn lint(args: &[&str]) -> super::support::Run {
    let mut all = vec!["agents-md"];
    all.extend_from_slice(args);
    run(&all, "")
}

fn problems(result: &super::support::Run) -> Vec<&str> {
    result.err.lines().collect()
}

const OK: &str = "agents-md: 1 repositories checked\n";

/// Lint a repository whose AGENTS.md is `text`, with `files` beside it.
fn lint_text(text: &str, files: &[(&str, &str)]) -> (String, super::support::Run) {
    let tmp = Tmp::new();
    let root = repo(&tmp, "repo", Some(text), files);
    let result = lint(&[&root]);
    (root, result)
}

fn assert_clean(text: &str, files: &[(&str, &str)]) {
    let (_, result) = lint_text(text, files);
    assert_eq!(
        (result.code, result.err.as_str(), result.out.as_str()),
        (0, "", OK),
        "{text}"
    );
}

fn assert_problems(text: &str, files: &[(&str, &str)], expected: &[&str]) {
    let (root, result) = lint_text(text, files);
    let want: Vec<String> = expected
        .iter()
        .map(|message| format!("{root}/AGENTS.md:{message}"))
        .collect();
    assert_eq!(result.code, 1, "{text}");
    assert_eq!(
        problems(&result),
        want.iter().map(String::as_str).collect::<Vec<_>>(),
        "{text}"
    );
}

#[test]
fn missing_empty_and_whitespace_only_files() {
    let tmp = Tmp::new();
    let root = repo(&tmp, "repo", None, &[]);
    let result = lint(&[&root]);
    assert_eq!(result.code, 1);
    assert_eq!(
        problems(&result),
        [format!("{root}/AGENTS.md:1: AGENTS.md is missing")]
    );
    assert_eq!(result.out, "");
    assert_problems("", &[], &["1: AGENTS.md is empty"]);
    assert_problems("  \n\t\n\n   \n", &[], &["1: AGENTS.md is empty"]);
}

#[test]
fn a_clean_pointer_file() {
    assert_clean(
        CLEAN_POINTER,
        &[
            ("scripts/run.py", "print('run')\n"),
            ("docs/guide.md", "# Guide\n"),
        ],
    );
}

#[test]
fn dead_and_valid_links() {
    assert_problems(
        "# Agent contract\n\nRead [the guide](docs/guide.md).\n",
        &[],
        &["3: dead path: docs/guide.md"],
    );
    assert_clean(
        "# Agent contract\n\nRead [the guide](docs/guide.md).\n",
        &[("docs/guide.md", "# Guide\n")],
    );
    assert_clean(
        "# Agent contract\n\nRead [the docs](docs).\n",
        &[("docs/guide.md", "# Guide\n")],
    );
}

#[test]
fn external_and_anchor_links_are_skipped() {
    for target in [
        "https://example.com/contract",
        "http://example.com/contract",
        "mailto:someone@example.com",
        "ftp://example.com/contract",
        "#agent-contract",
        "#",
        "?q=1",
    ] {
        assert_clean(
            &format!("# Agent contract\n\nRead [the contract]({target}).\n"),
            &[],
        );
    }
}

#[test]
fn fragment_and_query_are_stripped() {
    for target in [
        "docs/guide.md#install",
        "docs/guide.md?plain=1",
        "docs/guide.md#a?b",
    ] {
        assert_clean(
            &format!("Read [the guide]({target}).\n"),
            &[("docs/guide.md", "# Guide\n")],
        );
    }
    assert_problems(
        "Read [the guide](docs/missing.md#install).\n",
        &[],
        &["1: dead path: docs/missing.md"],
    );
}

#[test]
fn inline_paths() {
    assert_problems(
        "# Agent contract\n\nRun `scripts/missing.py` first.\n",
        &[],
        &["3: dead path: scripts/missing.py"],
    );
    assert_clean("Run `scripts/run.py` first.\n", &[("scripts/run.py", "x")]);
    assert_clean(
        "Everything lives in `scripts/`.\n",
        &[("scripts/run.py", "x")],
    );
    assert_problems(
        "Everything lives in `missing/`.\n",
        &[],
        &["1: dead path: missing"],
    );
    assert_clean(
        "Run `./scripts/run.py` first.\n",
        &[("scripts/run.py", "x")],
    );
}

#[test]
fn spans_that_are_not_repository_paths_are_ignored() {
    for span in [
        "*.py",
        "scripts/*.py",
        "a?b",
        "<path>",
        "scripts/<name>.py",
        "${HOME}/notes.md",
        "{a,b}.md",
        "a|b",
        "~/notes.md",
        "-rf",
        "https://example.com/x",
        "foo://bar",
        "/etc/passwd",
        "two words",
        "scripts/run.py --verbose",
        "",
    ] {
        assert_clean(&format!("Run `{span}` first.\n"), &[]);
    }
}

#[test]
fn fenced_blocks_are_skipped() {
    let text = "# Agent contract\n\n```sh\ncat `missing.py`\nsee [the guide](docs/missing.md)\ncall `scripts/gone.py::main`\n```\n\n~~~\n`also/missing.py`\n~~~\n";
    assert_clean(text, &[]);
    // A longer fence closes only on an equal or longer run of the same character.
    assert_clean("````\n```\n`gone/x.py`\n````\n", &[]);
    assert_clean("~~~\n```\n`gone/x.py`\n~~~\n", &[]);
    assert_problems(
        "```\n~~~\n```\n`gone/x.py`\n",
        &[],
        &["4: dead path: gone/x.py"],
    );
    // Four spaces of indent is not a fence; a tab-free two-space indent is.
    assert_problems("    ```\n`gone/x.py`\n", &[], &["2: dead path: gone/x.py"]);
    assert_clean("  ```\n`gone/x.py`\n  ```\n", &[]);
    assert_problems("``\n`gone/x.py`\n", &[], &["2: dead path: gone/x.py"]);
}

#[test]
fn no_slash_spans_use_the_extension_allowlist() {
    let files = [("README.md", "# Readme\n"), ("notes.txt", "notes\n")];
    for span in ["README.md", "notes.txt", "Makefile", "notes.rst", ".md"] {
        assert_clean(&format!("Read `{span}` first.\n"), &files);
    }
    assert_problems(
        "Read `missing.md` first.\n",
        &files,
        &["1: dead path: missing.md"],
    );
    assert_problems(
        "Read `missing.py` first.\n",
        &files,
        &["1: dead path: missing.py"],
    );
}

#[test]
fn symbols() {
    assert_problems(
        "Call `scripts/run.py::main` first.\n",
        &[("scripts/run.py", "def other() -> None:\n    pass\n")],
        &["1: dead symbol: scripts/run.py::main"],
    );
    assert_clean(
        "Call `scripts/run.py::main()` first.\n",
        &[("scripts/run.py", "def main() -> int:\n    return 0\n")],
    );
    assert_problems(
        "Call `scripts/run.py::main` first.\n",
        &[("scripts/run.py", "def main_helper() -> None:\n    pass\n")],
        &["1: dead symbol: scripts/run.py::main"],
    );
    assert_problems(
        "Call `scripts/run.py::main` first.\n",
        &[("scripts/run.py", "def amain() -> None:\n    pass\n")],
        &["1: dead symbol: scripts/run.py::main"],
    );
    assert_clean(
        "Call `lib/::helper` first.\n",
        &[("lib/deep/helper.py", "def helper() -> None:\n    pass\n")],
    );
    assert_problems(
        "Call `lib/::helper` first.\n",
        &[("lib/deep/other.py", "def other() -> None:\n    pass\n")],
        &["1: dead symbol: lib/::helper"],
    );
    assert_problems(
        "Call `scripts/gone.py::main` first.\n",
        &[],
        &["1: dead path: scripts/gone.py"],
    );
}

#[test]
fn symbol_reference_on_a_non_path_is_ignored() {
    assert_clean(
        "Call `foo::bar` or `Class::method()`. Also `a::` and `::b` and `x::y z`.\n",
        &[],
    );
}

#[test]
fn nested_symbols_line_numbers_and_prefixed_dirs() {
    let text = "`scripts/run.py::Thing.method` `scripts/run.py::Thing::method()` `scripts/run.py:2` `httpx/x.py`\n";
    assert_problems(
        text,
        &[(
            "scripts/run.py",
            "class Thing:\n    def method(self): ...\n",
        )],
        &["1: dead path: httpx/x.py"],
    );
}

#[test]
fn invalid_utf8() {
    let tmp = Tmp::new();
    let root = repo(&tmp, "repo", None, &[]);
    fs::write(
        tmp.join("repo/AGENTS.md"),
        b"# Agent contract\n\xff\xfe not utf-8\n",
    )
    .unwrap();
    let result = lint(&[&root]);
    assert_eq!(result.code, 1);
    assert_eq!(
        problems(&result),
        [format!("{root}/AGENTS.md:1: AGENTS.md is not valid utf-8")]
    );
}

#[test]
fn line_numbers_and_duplicates() {
    assert_problems(
        "# Agent contract\n\nOne.\n\nTwo.\n\nRun `scripts/missing.py`.\n",
        &[],
        &["7: dead path: scripts/missing.py"],
    );
    assert_problems(
        "# Agent contract\n\nRun `a/missing.py` and `a/missing.py`.\n",
        &[],
        &["3: dead path: a/missing.py"],
    );
    assert_problems(
        "Run `a/missing.py`.\n\nRun `a/missing.py`.\n",
        &[],
        &["1: dead path: a/missing.py", "3: dead path: a/missing.py"],
    );
}

#[test]
fn problems_on_one_line_come_in_the_order_they_appear() {
    assert_problems(
        "`b/two.py` [x](a/one.md) `c/three.py`\n",
        &[],
        &[
            "1: dead path: b/two.py",
            "1: dead path: a/one.md",
            "1: dead path: c/three.py",
        ],
    );
}

#[test]
fn multiple_repositories() {
    let tmp = Tmp::new();
    let clean = repo(
        &tmp,
        "clean",
        Some("# Agent contract\n\nNothing to see.\n"),
        &[],
    );
    let dirty = repo(
        &tmp,
        "dirty",
        Some("# Agent contract\n\nRun `missing.py`.\n"),
        &[],
    );
    let result = lint(&[&clean, &dirty]);
    assert_eq!(result.code, 1);
    assert_eq!(
        problems(&result),
        [format!("{dirty}/AGENTS.md:3: dead path: missing.py")]
    );
    assert_eq!(result.out, "");
    let other = repo(
        &tmp,
        "other",
        Some("# Agent contract\n\nNothing to see.\n"),
        &[],
    );
    let result = lint(&[&clean, &other]);
    assert_eq!(
        (result.code, result.out.as_str()),
        (0, "agents-md: 2 repositories checked\n")
    );
}

#[test]
fn the_default_root_is_the_current_directory() {
    // The test binary runs from the crate directory, which holds no AGENTS.md.
    let result = lint(&[]);
    assert_eq!(result.code, 1);
    assert_eq!(problems(&result), ["./AGENTS.md:1: AGENTS.md is missing"]);
}

#[test]
fn usage_errors() {
    let tmp = Tmp::new();
    let result = lint(&[&tmp.join("nope").display().to_string()]);
    assert_eq!((result.code, result.out.as_str()), (2, ""));
    assert!(result.err.ends_with("nope: not a directory\n"));
    let file = tmp.write("AGENTS.md", "# Agent contract\n");
    assert_eq!(lint(&[&file.display().to_string()]).code, 2);
    let clean = repo(
        &tmp,
        "clean",
        Some("# Agent contract\n\nNothing to see.\n"),
        &[],
    );
    assert_eq!(
        lint(&[&clean, &tmp.join("nope").display().to_string()]).code,
        2
    );
    let result = lint(&["--bogus"]);
    assert_eq!(
        (result.code, result.err.as_str()),
        (2, "agents-md: unknown option --bogus\n")
    );
}

#[test]
fn the_require_flag_is_accepted() {
    let tmp = Tmp::new();
    let clean = repo(
        &tmp,
        "clean",
        Some("# Agent contract\n\nNothing to see.\n"),
        &[],
    );
    let result = lint(&["--require", &clean]);
    assert_eq!((result.code, result.out.as_str()), (0, OK));
    let bare = repo(&tmp, "bare", None, &[]);
    let result = lint(&["--require", &bare]);
    assert_eq!(result.code, 1);
    assert_eq!(
        problems(&result),
        [format!("{bare}/AGENTS.md:1: AGENTS.md is missing")]
    );
}

#[test]
fn link_titles_angle_brackets_and_percent_encoding() {
    let text = "[a](docs/my%20guide.md \"t\") [b](<docs/my guide.md>) [c](docs/nope.md \"t\") [d](docs/nope2.md \"open) [e](<docs/nope3.md>) [f]() [g](docs/%zz.md)\n";
    assert_problems(
        text,
        &[("docs/my guide.md", "x")],
        &[
            "1: dead path: docs/nope.md",
            "1: dead path: docs/nope3.md",
            "1: dead path: docs/%zz.md",
        ],
    );
    assert_clean("[x] (docs/a.md) [y](docs/a.md\n", &[]);
    assert_clean("[no close\n", &[]);
}

#[test]
fn the_symbol_must_be_a_whole_word_across_unicode() {
    assert_problems(
        "`a.py::main`\n",
        &[("a.py", "def éain(): ...\n")],
        &["1: dead symbol: a.py::main"],
    );
    assert_clean("`a.py::main`\n", &[("a.py", "é main é\n")]);
}

#[test]
fn a_dangling_link_inside_a_directory_is_not_a_file() {
    let tmp = Tmp::new();
    let root = repo(
        &tmp,
        "repo",
        Some("`lib/::helper`\n"),
        &[("lib/real.py", "def helper(): ...\n")],
    );
    std::os::unix::fs::symlink(tmp.join("nowhere"), tmp.join("repo/lib/dangling")).unwrap();
    assert_eq!(lint(&[&root]).code, 0);
}
