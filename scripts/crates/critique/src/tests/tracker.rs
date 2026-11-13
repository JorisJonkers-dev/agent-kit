//! Tests for the tracker domain: the doc's heading, the default tracker, issue references and the
//! rendered issue.

use crate::tracker::{
    default_tracker, parse_issue_reference, render_issue, tracker_from_doc, IssueComment, IssueReference, IssueText,
    TrackerChoice, TRACKER_DOC,
};

fn known(kind: &'static str) -> TrackerChoice {
    TrackerChoice::Known { kind }
}

fn unsupported(name: &str) -> TrackerChoice {
    TrackerChoice::Unsupported { name: name.to_string() }
}

fn reference(repository: &str, number: f64) -> Option<IssueReference> {
    Some(IssueReference { repository: repository.to_string(), number })
}

#[test]
fn tracker_doc_names_the_setup_doc() {
    assert_eq!(TRACKER_DOC, "docs/agents/issue-tracker.md");
}

#[test]
fn tracker_from_doc_reads_github() {
    assert_eq!(tracker_from_doc("# Issue tracker: github\n"), known("github"));
    assert_eq!(tracker_from_doc("# Issue tracker: GitHub\n"), known("github"));
    assert_eq!(tracker_from_doc("# ISSUE TRACKER: GITHUB\n"), known("github"));
    assert_eq!(tracker_from_doc("# Issue tracker: github"), known("github"));
    assert_eq!(tracker_from_doc("# Issue tracker: github  \n"), known("github"));
    assert_eq!(tracker_from_doc("#\tIssue tracker:\tgithub\n"), known("github"));
    assert_eq!(tracker_from_doc("# Issue tracker:github\n"), known("github"));
    assert_eq!(tracker_from_doc("intro\n# Issue tracker: github\nbody\n"), known("github"));
    assert_eq!(tracker_from_doc("# Issue tracker:\n\ngithub\n"), known("github"));
    assert_eq!(tracker_from_doc("# Issue tracker: github\r\nbody"), known("github"));
    assert_eq!(tracker_from_doc("intro\u{2028}# Issue tracker: github\u{2028}body"), known("github"));
}

#[test]
fn tracker_from_doc_reads_local_markdown() {
    assert_eq!(tracker_from_doc("# Issue tracker: local markdown\n"), known("local-markdown"));
    assert_eq!(tracker_from_doc("# Issue tracker: Local Markdown\n"), known("local-markdown"));
}

#[test]
fn tracker_from_doc_reports_an_unsupported_name() {
    assert_eq!(tracker_from_doc("# Issue tracker: GitLab\n"), unsupported("GitLab"));
    assert_eq!(tracker_from_doc("# Issue tracker: Jira and Linear\n"), unsupported("Jira and Linear"));
    assert_eq!(tracker_from_doc("# Issue tracker:  x y\n"), unsupported("x y"));
    // The capture is not trimmed beyond what the regex does: `\s*` backs off to one space.
    assert_eq!(tracker_from_doc("# Issue tracker:   \n"), unsupported(" "));
}

#[test]
fn tracker_from_doc_without_a_heading_has_no_tracker() {
    assert_eq!(tracker_from_doc(""), unsupported("no tracker"));
    assert_eq!(tracker_from_doc("no heading here\n"), unsupported("no tracker"));
    assert_eq!(tracker_from_doc("#Issue tracker: github\n"), unsupported("no tracker"));
    assert_eq!(tracker_from_doc("# Something else\n"), unsupported("no tracker"));
    assert_eq!(tracker_from_doc("x # Issue tracker: github\n"), unsupported("no tracker"));
    assert_eq!(tracker_from_doc("# Issue tracker:\n"), unsupported("no tracker"));
    assert_eq!(tracker_from_doc("# Issue tracker:\u{2028}"), unsupported("no tracker"));
}

#[test]
fn default_tracker_follows_the_allowlist_key() {
    assert_eq!(default_tracker("github.com/owner/repo"), "github");
    assert_eq!(default_tracker("gitlab.com/owner/repo"), "local-markdown");
    assert_eq!(default_tracker(""), "local-markdown");
    assert_eq!(default_tracker("github.com"), "local-markdown");
}

#[test]
fn parse_issue_reference_reads_a_bare_number() {
    let key = "github.com/owner/repo";
    assert_eq!(parse_issue_reference("167", key), reference(key, 167.0));
    assert_eq!(parse_issue_reference("#167", key), reference(key, 167.0));
    assert_eq!(parse_issue_reference("  #167\n", key), reference(key, 167.0));
    assert_eq!(parse_issue_reference("007", key), reference(key, 7.0));
}

#[test]
fn parse_issue_reference_reads_a_short_reference() {
    assert_eq!(
        parse_issue_reference("owner/repo#167", "local"),
        reference("github.com/owner/repo", 167.0)
    );
    assert_eq!(parse_issue_reference("Owner/Repo#7", "local"), reference("github.com/owner/repo", 7.0));
    assert_eq!(parse_issue_reference("a.b-c_d/e.f#1", "local"), reference("github.com/a.b-c_d/e.f", 1.0));
}

#[test]
fn parse_issue_reference_reads_a_full_reference() {
    assert_eq!(
        parse_issue_reference("GitHub.com/Owner/Repo#167", "local"),
        reference("github.com/owner/repo", 167.0)
    );
    assert_eq!(
        parse_issue_reference("gitlab.example.com/owner/repo#3", "local"),
        reference("gitlab.example.com/owner/repo", 3.0)
    );
}

#[test]
fn parse_issue_reference_reads_an_issue_url() {
    let expected = reference("github.com/owner/repo", 12.0);
    assert_eq!(parse_issue_reference("https://github.com/owner/repo/issues/12", "local"), expected);
    assert_eq!(parse_issue_reference("http://github.com/owner/repo/issues/12", "local"), expected);
    assert_eq!(parse_issue_reference("HTTP://GitHub.com/Owner/Repo/Issues/12", "local"), expected);
    assert_eq!(parse_issue_reference("https://github.com/owner/repo/issues/12/", "local"), expected);
    assert_eq!(parse_issue_reference("https://github.com/owner/repo/issues/12?q=1", "local"), expected);
    assert_eq!(parse_issue_reference("https://github.com/owner/repo/issues/12#note", "local"), expected);
    assert_eq!(parse_issue_reference("https://github.com/owner/repo/issues/12/files", "local"), expected);
    assert_eq!(parse_issue_reference("  https://github.com/owner/repo/issues/12  ", "local"), expected);
    assert_eq!(
        parse_issue_reference("https://例え.jp/owner/repo/issues/1", "local"),
        reference("例え.jp/owner/repo", 1.0)
    );
}

#[test]
fn parse_issue_reference_rejects_anything_else() {
    for text in [
        "",
        "#",
        "abc",
        "167x",
        "owner/repo",
        "owner/repo#",
        "owner/repo#x",
        "owner/repo#1x",
        "owner/repo#1/2",
        "a/b/c",
        "a/b/c#1x",
        "https://github.com/owner/repo/issues/",
        "https://github.com/owner/repo/issues/x",
        "https://github.com/owner/repo/issues/12 x",
        "https://github.com/owner/repo/issues/12/ x",
        "https://github.com/owner/repo/pulls/12",
        "https:///owner/repo/issues/12",
        "https://github.com//repo/issues/12",
        "https://github.com/owner//issues/12",
        "https://github.com/owner repo/issues/12",
        "ftp://github.com/owner/repo/issues/12",
        "https:/github.com/owner/repo/issues/12",
    ] {
        assert_eq!(parse_issue_reference(text, "local"), None, "{text}");
    }
}

#[test]
fn render_issue_writes_the_title_and_body() {
    let issue = IssueText { title: "Bug".to_string(), body: "  It breaks.  ".to_string(), comments: Vec::new() };
    assert_eq!(render_issue(&issue), "# Bug\n\nIt breaks.\n");
}

#[test]
fn render_issue_skips_an_empty_body() {
    let issue = IssueText { title: "Bug".to_string(), body: " \n\t ".to_string(), comments: Vec::new() };
    assert_eq!(render_issue(&issue), "# Bug\n");
}

#[test]
fn render_issue_writes_comments() {
    let issue = IssueText {
        title: "Bug".to_string(),
        body: "It breaks.".to_string(),
        comments: vec![
            IssueComment {
                author: "ann".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                body: " Same here. ".to_string(),
            },
            IssueComment { author: "bob".to_string(), created_at: "2024-01-02T00:00:00Z".to_string(), body: String::new() },
        ],
    };
    assert_eq!(
        render_issue(&issue),
        "# Bug\n\nIt breaks.\n\n## Comments\n\n### ann, 2024-01-01T00:00:00Z\n\nSame here.\n\n### bob, 2024-01-02T00:00:00Z\n\n\n"
    );
}

#[test]
fn render_issue_normalises_line_endings() {
    let issue =
        IssueText { title: "Bug".to_string(), body: "one\r\ntwo\rthree\nfour".to_string(), comments: Vec::new() };
    assert_eq!(render_issue(&issue), "# Bug\n\none\ntwo\nthree\nfour\n");
}

#[test]
fn render_issue_normalises_line_endings_in_comments() {
    let issue = IssueText {
        title: "Bug".to_string(),
        body: String::new(),
        comments: vec![IssueComment {
            author: "ann".to_string(),
            created_at: "now".to_string(),
            body: "a\r\nb\rc".to_string(),
        }],
    };
    assert_eq!(render_issue(&issue), "# Bug\n\n## Comments\n\n### ann, now\n\na\nb\nc\n");
}
