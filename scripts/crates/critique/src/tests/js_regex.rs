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
        (r"(?<a)b", "(?< is not supported"),
        (r"(?#c)b", "(?# is not supported"),
        (r"\d\q", "\\q is not supported"),
        (r"\1", "\\1 is not supported"),
        (r"[\B]", "\\B in a class is not supported"),
        (r"[a-\d]", "a class escape cannot end a range"),
        (r"\B*", "nothing to repeat"),
        (r"^*", "nothing to repeat"),
        (r"\b{2}", "nothing to repeat"),
        (r"(?", "a group is not closed"),
    ] {
        assert_eq!(Regex::new(pattern, false).expect_err("an error"), message, "{pattern}");
    }
    assert_eq!(Regex::with_flags("a", "y").expect_err("an error"), "the flag y is not supported");
    assert_eq!(Regex::new("a{2,1}", false).expect_err("an error"), "numbers out of order in {} quantifier");
    assert_eq!(Regex::new("[z-a]", false).expect_err("an error"), "range out of order in character class");
}

/// The first match of `pattern` with `flags` in `text`, with its groups.
fn first(pattern: &str, flags: &str, text: &str) -> Option<(String, Vec<Option<String>>)> {
    let regex = Regex::with_flags(pattern, flags).expect("pattern");
    let chars: Vec<char> = text.chars().collect();
    regex.find_at(&chars, 0).map(|hit| {
        let whole = chars[hit.start..hit.end].iter().collect();
        let groups = (1..=regex.group_count()).map(|index| hit.group(&chars, index)).collect();
        (whole, groups)
    })
}

#[test]
fn dot_digits_and_control_escapes_match_as_javascript_does() {
    assert_eq!(first(r"a.c", "", "a\nc abc"), Some(("abc".to_string(), vec![])));
    assert_eq!(first(r"a.c", "", "a\u{2028}c"), None);
    assert_eq!(first(r"\d+", "", "x١2٣45"), Some(("2".to_string(), vec![])));
    assert_eq!(first(r"[\d\-]+", "", "x-12-y"), Some(("-12-".to_string(), vec![])));
    assert_eq!(first(r"\t\r\n", "", "a\t\r\nb"), Some(("\t\r\n".to_string(), vec![])));
}

#[test]
fn negated_classes_control_and_numeric_escapes_match_as_javascript_does() {
    assert_eq!(first(r"\S+\s\W\D", "", " ab !x"), Some(("ab !x".to_string(), vec![])));
    assert_eq!(first(r"\Bb\B", "", "abc b"), Some(("b".to_string(), vec![])));
    assert_eq!(first(r"[\S][\W][\D]", "", "a  a-x"), Some(("a  ".to_string(), vec![])));
    assert_eq!(first(r"[\S][\W][\D]", "", "a-x"), Some(("a-x".to_string(), vec![])));
    assert_eq!(first(r"[\b-\t]\f\v\0", "", "\u{8}\u{c}\u{b}\0"), Some(("\u{8}\u{c}\u{b}\0".to_string(), vec![])));
    assert_eq!(first(r"[\d-z]+", "", "5-z"), Some(("5-z".to_string(), vec![])));
    assert_eq!(first(r"[\x01-\b]", "", "\u{7}"), Some(("\u{7}".to_string(), vec![])));
    assert_eq!(first(r"\0a", "", "x\0a"), Some(("\0a".to_string(), vec![])));
    assert_eq!(Regex::new(r"\01", false).expect_err("an error"), "\\0 is not supported");
    assert_eq!(first(r"\x41\u00e9\xZ\u12", "", "AéxZu12"), Some(("AéxZu12".to_string(), vec![])));
    assert_eq!(first(r"\ud800", "", "x"), None);
}

#[test]
fn braces_repeat_a_counted_number_of_times_or_read_as_a_literal_brace() {
    assert_eq!(first(r"^#{1,3} ", "", "#### x"), None);
    assert_eq!(first(r"#{1,3} ", "", "#### x"), Some(("### ".to_string(), vec![])));
    assert_eq!(first(r"a{2}", "", "aaa"), Some(("aa".to_string(), vec![])));
    assert_eq!(first(r"a{2,}", "", "aaaa"), Some(("aaaa".to_string(), vec![])));
    assert_eq!(first(r"a{2,}?", "", "aaaa"), Some(("aa".to_string(), vec![])));
    assert_eq!(first(r"a{,2}", "", "a{,2}"), Some(("a{,2}".to_string(), vec![])));
    assert_eq!(first(r"a{2", "", "a{2"), Some(("a{2".to_string(), vec![])));
    assert_eq!(first(r"a{2,x}", "", "a{2,x}"), Some(("a{2,x}".to_string(), vec![])));
    assert_eq!(first(r"(?:ab){2}", "", "ababab"), Some(("abab".to_string(), vec![])));
    assert_eq!(first(r"a{99999999999999999999}", "", "a"), None);
}

#[test]
fn a_lookahead_takes_no_text_and_a_failed_one_keeps_no_capture() {
    assert_eq!(first(r"^#?(\d+)(?=$|[\s\-:.(])", "", "12-schema"), Some(("12".to_string(), vec![Some("12".to_string())])));
    assert_eq!(first(r"^#?(\d+)(?=$|[\s\-:.(])", "", "12x"), None);
    assert_eq!(first(r"(?=(a))a", "", "a"), Some(("a".to_string(), vec![Some("a".to_string())])));
    assert_eq!(first(r"(?:(?!(b)c)(a|b))+", "", "ab"), Some(("ab".to_string(), vec![None, Some("b".to_string())])));
    assert_eq!(first(r"(?=a)*b", "", "b"), Some(("b".to_string(), vec![])));
}

#[test]
fn ignore_case_folds_as_javascript_canonicalizes() {
    assert_eq!(first(r"café", "i", "CAFÉ"), Some(("CAFÉ".to_string(), vec![])));
    assert_eq!(first(r"[A-z]", "i", "g"), Some(("g".to_string(), vec![])));
    assert_eq!(first(r"[A-z]", "i", "_"), Some(("_".to_string(), vec![])));
    assert_eq!(first(r"[à-ü]", "i", "À"), Some(("À".to_string(), vec![])));
    assert_eq!(first(r"[^\x00-\x7f]", "i", "é"), Some(("é".to_string(), vec![])));
    assert_eq!(first(r"[\u0100-\u017f]", "i", "S"), None);
    assert_eq!(first(r"ß", "i", "ẞ"), None);
    assert_eq!(first(r"[é]", "i", "É"), Some(("É".to_string(), vec![])));
    assert_eq!(first(r"\u{10428}", "", "x"), None);
}

#[test]
fn ignore_case_folds_ascii_letters_only() {
    assert_eq!(first(r"blocked by", "i", "BLOCKED By"), Some(("BLOCKED By".to_string(), vec![])));
    assert_eq!(first(r"[a-c]+", "i", "xAbC"), Some(("AbC".to_string(), vec![])));
    assert_eq!(first(r"[^a]", "i", "Ab"), Some(("b".to_string(), vec![])));
    assert_eq!(first(r"s", "i", "\u{17f}"), None);
    assert_eq!(first(r"k", "i", "\u{212a}"), None);
    assert_eq!(first(r"stor(?:y|ies)", "", "STORY"), None);
    assert_eq!(first(r"^x$", "gm", "a\nx\nb"), Some(("x".to_string(), vec![])));
}

#[test]
fn a_negative_lookbehind_looks_one_character_back() {
    assert_eq!(all(r"(?<![\w-])ab", false, "ab -ab xab a_ab (ab"), ["ab", "ab"]);
    assert_eq!(all(r"(?<!\d)x", false, "1x x"), ["x"]);
    assert!(Regex::new("(?<=a)b", false).is_err());
    assert!(Regex::new("(?<!ab)c", false).is_err());
    assert!(Regex::new("(?<!a|b)c", false).is_err());
}
