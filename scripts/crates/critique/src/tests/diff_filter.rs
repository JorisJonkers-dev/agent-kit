//! Tests for the diff filter: the sections a diff is cut into, the path each one
//! names, and the line that stands in for a file whose hunks are left out.

use crate::diff_filter::{filter_diff, parse_diff, section_text, DiffHunk, DiffSection, HunkSide, LeftOutFile};

const MODIFIED: &str = "diff --git a/src/main.rs b/src/main.rs\nindex 1234567..89abcde 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    new();\n+    more();\n }\n@@ -10,2 +11,2 @@ fn other() {\n keep();\n-drop();\n+add();\n";

const DELETED: &str = "diff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\nindex abc1234..0000000\n--- a/gone.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n";

const RENAMED: &str = "diff --git a/old.txt b/new.txt\nsimilarity index 100%\nrename from old.txt\nrename to new.txt\n";

const BINARY: &str = "diff --git a/bin.dat b/bin.dat\nindex 1111111..2222222 100644\nBinary files a/bin.dat and b/bin.dat differ\n";

const MODE: &str = "diff --git a/mode.sh b/mode.sh\nold mode 100644\nnew mode 100755\n";

fn paths(sections: &[DiffSection]) -> Vec<Option<&str>> {
    sections.iter().map(|section| section.path.as_deref()).collect()
}

#[test]
fn parses_a_git_diff_into_sections() {
    let sections = parse_diff(&format!("{MODIFIED}{DELETED}{RENAMED}{BINARY}{MODE}"));
    assert_eq!(paths(&sections), vec![Some("src/main.rs"), Some("gone.txt"), Some("new.txt"), Some("bin.dat"), Some("mode.sh")]);
    assert_eq!(
        sections[0].header,
        "diff --git a/src/main.rs b/src/main.rs\nindex 1234567..89abcde 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n"
    );
    assert_eq!(
        sections[0].hunks,
        vec![
            DiffHunk {
                text: "@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    new();\n+    more();\n }\n".to_string(),
                old: HunkSide { start: 1.0, count: 3.0 },
                new: HunkSide { start: 1.0, count: 4.0 },
            },
            DiffHunk {
                text: "@@ -10,2 +11,2 @@ fn other() {\n keep();\n-drop();\n+add();\n".to_string(),
                old: HunkSide { start: 10.0, count: 2.0 },
                new: HunkSide { start: 11.0, count: 2.0 },
            },
        ]
    );
    assert_eq!(sections[1].hunks, vec![DiffHunk {
        text: "@@ -1,2 +0,0 @@\n-a\n-b\n".to_string(),
        old: HunkSide { start: 1.0, count: 2.0 },
        new: HunkSide { start: 0.0, count: 0.0 },
    }]);
    assert_eq!(sections[2].hunks, Vec::new());
    assert_eq!(sections[3].header, BINARY);
    assert_eq!(sections[4].header, MODE);
    assert_eq!(section_text(&sections[0]), MODIFIED);
    assert_eq!(section_text(&sections[3]), BINARY);
}

#[test]
fn an_empty_diff_is_one_empty_section() {
    let sections = parse_diff("");
    assert_eq!(sections, vec![DiffSection { path: None, header: String::new(), hunks: Vec::new() }]);
    assert_eq!(section_text(&sections[0]), "");
}

#[test]
fn keeps_text_before_the_first_section() {
    let sections = parse_diff("preamble\nmore\ndiff --git a/x b/x\n--- a/x\n+++ b/x\n");
    assert_eq!(paths(&sections), vec![None, Some("x")]);
    assert_eq!(sections[0].header, "preamble\nmore\n");
    assert_eq!(sections[1].header, "diff --git a/x b/x\n--- a/x\n+++ b/x\n");
}

#[test]
fn keeps_lines_that_are_not_hunk_headers_in_the_hunk() {
    let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n@@ -x +1 @@\n@@ -1, +2 @@\n@@ -1 +2\n";
    let sections = parse_diff(diff);
    assert_eq!(sections[0].hunks.len(), 1);
    assert_eq!(sections[0].hunks[0].text, "@@ -1 +1 @@\n-a\n+b\n@@ -x +1 @@\n@@ -1, +2 @@\n@@ -1 +2\n");
    assert_eq!(sections[0].hunks[0].old, HunkSide { start: 1.0, count: 1.0 });
}

#[test]
fn reads_a_quoted_path_from_the_git_line() {
    let diff = "diff --git \"a/with \\\"quote\\\".txt\" \"b/with \\\"quote\\\".txt\"\nBinary files differ\n";
    assert_eq!(paths(&parse_diff(diff)), vec![Some("with \"quote\".txt")]);
    let elsewhere = "diff --git \"a/x.txt\" \"c/x.txt\"\nBinary files differ\n";
    assert_eq!(paths(&parse_diff(elsewhere)), vec![None]);
}

#[test]
fn reads_octal_escapes_in_a_quoted_path() {
    let diff = "diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"\nBinary files differ\n";
    assert_eq!(paths(&parse_diff(diff)), vec![Some("café.txt")]);
}

#[test]
fn reads_an_escaped_astral_character() {
    let diff = "diff --git \"a/\\360\\237\\230\\200.txt\" \"b/\\360\\237\\230\\200.txt\"\nBinary files differ\n";
    assert_eq!(paths(&parse_diff(diff)), vec![Some("😀.txt")]);
}

#[test]
fn replaces_a_lone_surrogate_with_the_replacement_character() {
    let diff = "diff --git \"a/😀.txt\" \"b/😀.txt\"\nBinary files differ\n";
    assert_eq!(paths(&parse_diff(diff)), vec![Some("\u{fffd}\u{fffd}.txt")]);
}

#[test]
fn reads_a_path_holding_a_space_b_from_the_header_lines() {
    let diff = "diff --git a/dir b/one.txt b/dir b/two.txt\n--- a/dir b/one.txt\n+++ b/dir b/two.txt\n@@ -1 +1 @@\n-a\n+b\n";
    assert_eq!(paths(&parse_diff(diff)), vec![Some("dir b/two.txt")]);
    let bare = "diff --git a/dir b/one.txt b/dir b/two.txt\nBinary files differ\n";
    assert_eq!(paths(&parse_diff(bare)), vec![None]);
}

#[test]
fn falls_back_to_the_base_path_of_a_deleted_file() {
    let deleted = "diff --git a/x b/x\n--- /dev/null\n";
    assert_eq!(paths(&parse_diff(deleted)), vec![Some("x")]);
    let unhead = "diff --git a/x.txt b/x.txt\n--- a/x.txt\n+++ x.txt\n";
    assert_eq!(paths(&parse_diff(unhead)), vec![Some("x.txt")]);
}

#[test]
fn reads_a_copied_path() {
    let diff = "diff --git a/src.txt b/copy.txt\nsimilarity index 100%\ncopy from src.txt\ncopy to copy.txt\n";
    assert_eq!(paths(&parse_diff(diff)), vec![Some("copy.txt")]);
}

#[test]
fn a_git_line_without_paths_names_no_path() {
    assert_eq!(paths(&parse_diff("diff --git \n")), vec![None]);
    assert_eq!(paths(&parse_diff("diff --git x\n")), vec![None]);
}

#[test]
fn unquotes_every_escape() {
    let diff = "diff --git a/x b/x\n--- a/x\n+++ \"b/\\a\\b\\f\\n\\r\\t\\v\\\"\\\\\"\n";
    assert_eq!(paths(&parse_diff(diff)), vec![Some("\u{7}\u{8}\u{c}\n\r\t\u{b}\"\\")]);
}

#[test]
fn unquotes_a_trailing_backslash_as_a_nul() {
    let diff = "diff --git a/x b/x\n--- a/x\n+++ \"b/x\\\"\n";
    assert_eq!(paths(&parse_diff(diff)), vec![Some("x\u{0}")]);
}

#[test]
fn keeps_an_escape_that_is_not_octal() {
    let diff = "diff --git a/x b/x\n--- a/x\n+++ \"b/\\089\"\n";
    assert_eq!(paths(&parse_diff(diff)), vec![Some("089")]);
}

#[test]
fn leaves_out_a_file_with_the_stand_in_line() {
    let diff = format!("{MODIFIED}{BINARY}");
    let filtered = filter_diff(&diff, &|path: &str| (path == "src/main.rs").then(|| "too big".to_string()));
    assert_eq!(
        filtered.left_out,
        vec![LeftOutFile { path: "src/main.rs".to_string(), reason: "too big".to_string(), changed_lines: 5.0 }]
    );
    assert_eq!(filtered.diff, format!("Left out of this diff: src/main.rs, too big; its 5 changed lines are not shown.\n{BINARY}"));
}

#[test]
fn a_filtered_diff_filters_to_itself() {
    let diff = format!("{MODIFIED}{BINARY}");
    let once = filter_diff(&diff, &|path: &str| (path == "src/main.rs").then(|| "too big".to_string()));
    let twice = filter_diff(&once.diff, &|path: &str| (path == "src/main.rs").then(|| "too big".to_string()));
    assert_eq!(twice.diff, once.diff);
    assert_eq!(twice.left_out, Vec::new());
}
