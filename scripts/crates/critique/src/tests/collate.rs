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

#[test]
fn text_beyond_ascii_sorts_after_ascii_by_code_point() {
    assert_eq!(locale_compare("z", "é", false), Ordering::Less);
    assert_eq!(locale_compare("é", "ü", false), Ordering::Less);
    assert_eq!(locale_compare("é", "é", false), Ordering::Equal);
}

#[test]
fn numeric_compares_runs_of_digits_as_numbers() {
    assert_eq!(locale_compare("run-10", "run-9", true), Ordering::Greater);
    assert_eq!(locale_compare("run-10", "run-9", false), Ordering::Less);
    assert_eq!(locale_compare("007", "7", true), Ordering::Equal);
    assert_eq!(locale_compare("a1", "ab", true), Ordering::Less);
}
