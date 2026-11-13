//! The regular expression engine the scans run on, against what JavaScript does with the same pattern.

use crate::js_regex::Regex;

fn chars(text: &str) -> Vec<char> {
    text.chars().collect()
}

/// Every match of `pattern` in `text`, as the whole match.
fn all(pattern: &str, multiline: bool, text: &str) -> Vec<String> {
    let text = chars(text);
    Regex::new(pattern, multiline).expect("a pattern").find_all(&text).iter().map(|hit| text[hit.start..hit.end].iter().collect()).collect()
}

#[test]
fn quantifiers_are_greedy_or_lazy_and_backtrack() {
    // An empty match is a match, as `"aaa b".match(/a*/g)` has it.
    assert_eq!(all("a*", false, "aaa b"), ["aaa", "", "", ""]);
    assert_eq!(all("<[^>]*>", false, "<a><b>"), ["<a>", "<b>"]);
    assert_eq!(all(r"<\w+?>", false, "<ab>"), ["<ab>"]);
    assert_eq!(all("a+?b", false, "aaab"), ["aaab"]);
    assert_eq!(all("a??b", false, "ab b"), ["ab", "b"]);
    assert_eq!(all("(ab)+", false, "ababab ab"), ["ababab", "ab"]);
    assert_eq!(all("(?:ab)+?c", false, "ababc"), ["ababc"]);
    assert_eq!(all("(?:ab)+?b", false, "ababb"), ["abab".to_string() + "b"]);
    assert_eq!(all("(?:a|ab)(?:c|bcd)", false, "abcd"), ["abcd"]);
    assert_eq!(all("(?:ab)*", false, "x"), ["", ""]);
    assert_eq!(all("a{", false, "a{"), ["a{"]);
}

#[test]
fn anchors_boundaries_and_lookahead() {
    assert_eq!(all("^a", false, "a\na"), ["a"]);
    assert_eq!(all("^a", true, "a\na\r\u{2028}a\u{2029}a b"), ["a", "a", "a", "a"]);
    assert_eq!(all("a$", true, "a\na"), ["a", "a"]);
    assert_eq!(all("a$", false, "a\na"), ["a"]);
    assert_eq!(all(r"\bab\b", false, "ab abc ab"), ["ab", "ab"]);
    assert_eq!(all("a(?!=)", false, "a= ab"), ["a"]);
    assert_eq!(all(r"\s+\w+", false, "a \u{a0}\u{feff}b_1"), [" \u{a0}\u{feff}b_1"]);
}

#[test]
fn classes_take_ranges_escapes_and_negation() {
    assert_eq!(all(r"[a-c\]x-]+", false, "abc]x-d"), ["abc]x-"]);
    assert_eq!(all(r"[^\s\w]+", false, "a ,. b"), [",."]);
    assert_eq!(all(r"[\n]", false, "a\nb"), ["\n"]);
    assert_eq!(all(r"[a-]", false, "-a"), ["-", "a"]);
}

#[test]
fn groups_hold_their_text_only_when_their_branch_matched() {
    let regex = Regex::new(r"(a)|(b)", false).expect("a pattern");
    let text = chars("b");
    let hit = regex.find_at(&text, 0).expect("a match");
    assert_eq!(hit.group(&text, 1), None);
    assert_eq!(hit.group(&text, 2), Some("b".to_string()));
    let regex = Regex::new(r"(?:(a)|b)c", false).expect("a pattern");
    let text = chars("ab bc");
    let hit = regex.find_at(&text, 0).expect("a match");
    assert_eq!((hit.start, hit.group(&text, 1)), (3, None));
    assert!(regex.find_at(&text, 5).is_none());
    // A group that matched, then lost to a later failure, is unset again.
    let regex = Regex::new(r"(?:(a)b|ac)", false).expect("a pattern");
    let text = chars("ac");
    assert_eq!(regex.find_at(&text, 0).expect("a match").group(&text, 1), None);
}

#[test]
fn a_long_run_of_one_class_costs_no_stack() {
    let long = "a".repeat(200_000);
    assert_eq!(all("a*?b", false, &format!("{long}b")).len(), 1);
    assert_eq!(all("a*", false, &long), [long.clone(), String::new()]);
}

#[test]
fn a_pattern_that_cannot_be_read_is_an_error() {
    for (pattern, message) in [
        (r"(a", "a group is not closed"),
        (r"a)", "a group is closed that was not opened"),
        (r"[a", "a class is not closed"),
        (r"a\", "a pattern ends with a backslash"),
        (r"*a", "nothing to repeat"),
        (r"(?=a)", "(?= is not supported"),
        (r"(?", "a pattern ends with a backslash"),
    ] {
        assert_eq!(Regex::new(pattern, false).expect_err("an error"), message, "{pattern}");
    }
}
