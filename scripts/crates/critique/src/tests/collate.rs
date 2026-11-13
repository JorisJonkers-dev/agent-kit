//! `locale_compare` against what `localeCompare` answered in the node runner, and its edges.

use crate::collate::locale_compare;
use model_gateway::json::{parse, Value};
use std::cmp::Ordering;

const COMMITTED: &str = include_str!("../../tests/fixtures/collate.json");

fn sign(order: Ordering) -> f64 {
    match order {
        Ordering::Less => -1.0,
        Ordering::Equal => 0.0,
        Ordering::Greater => 1.0,
    }
}

/// Replays a corpus of `[a, b, numeric, expected]` cases; `CRITIQUE_COLLATE_CORPUS` names a bigger one.
#[test]
fn collate_corpus() {
    let text = match std::env::var("CRITIQUE_COLLATE_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("corpus file"),
        Err(_) => COMMITTED.to_string(),
    };
    let Value::Arr(cases) = parse(&text).expect("corpus JSON") else { panic!("corpus is not an array") };
    assert!(cases.len() >= 1000);
    for case in &cases {
        let Value::Arr(parts) = case else { panic!("case is not an array") };
        let (Value::Str(a), Value::Str(b), Value::Bool(numeric), Value::Num(expected)) = (&parts[0], &parts[1], &parts[2], &parts[3]) else {
            panic!("case has the wrong shape")
        };
        assert_eq!(sign(locale_compare(a, b, *numeric)), *expected, "{a:?} against {b:?}, numeric {numeric}");
    }
}

const PAIRS: &str = include_str!("../../tests/fixtures/collate-pairs.txt");

/// Every ordered pair of one-character ASCII strings, with and without `numeric`, against node's answers
/// (`tests/gen-collate-pairs.mjs`): the whole primary and tertiary order of ASCII, not a random sample of it.
#[test]
fn every_ascii_pair_collates_as_node_does() {
    let lines: Vec<&str> = PAIRS.lines().collect();
    assert_eq!(lines.len(), 2);
    for (line, numeric) in lines.iter().zip([false, true]) {
        let signs: Vec<char> = line.chars().collect();
        assert_eq!(signs.len(), 128 * 128);
        for a in 0..128u8 {
            for b in 0..128u8 {
                let expected = match signs[usize::from(a) * 128 + usize::from(b)] {
                    '-' => Ordering::Less,
                    '+' => Ordering::Greater,
                    _ => Ordering::Equal,
                };
                let (a, b) = ((a as char).to_string(), (b as char).to_string());
                assert_eq!(locale_compare(&a, &b, numeric), expected, "{a:?} against {b:?}, numeric {numeric}");
            }
        }
    }
}

/// Beyond ASCII the port does not follow ICU, and this records how it differs: ICU sorts an accented letter with
/// its base letter (node: `'z'.localeCompare('é')` is 1), where the port sorts every character beyond ASCII after
/// all of ASCII, by code point. Run ids, client names, timestamps and file names the ledger holds are ASCII.
#[test]
fn text_beyond_ascii_differs_from_icu_by_sorting_after_ascii() {
    assert_eq!(locale_compare("z", "é", false), Ordering::Less, "ICU says Greater; the port says Less");
    assert_eq!(locale_compare("é", "é", false), Ordering::Equal);
}

#[test]
fn numeric_compares_runs_of_digits_as_numbers() {
    assert_eq!(locale_compare("run-10", "run-9", true), Ordering::Greater);
    assert_eq!(locale_compare("run-10", "run-9", false), Ordering::Less);
    assert_eq!(locale_compare("007", "7", true), Ordering::Equal);
    assert_eq!(locale_compare("a1", "ab", true), Ordering::Less);
}
