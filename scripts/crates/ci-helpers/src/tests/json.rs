use crate::json::{parse, Json};
use serde::de::Expected;

#[test]
fn every_kind_of_value_reads_back() {
    let value = parse(r#"{"a": null, "b": true, "c": -1, "d": 18446744073709551615, "e": 1.5, "f": "s", "g": [1, "x"], "h": {"k": 0}}"#).unwrap();
    assert_eq!(value.get("a"), Some(&Json::Null));
    assert_eq!(value.get("b"), Some(&Json::Bool(true)));
    assert_eq!(value.get("c"), Some(&Json::Num("-1".into())));
    assert_eq!(
        value.get("d"),
        Some(&Json::Num("18446744073709551615".into()))
    );
    assert_eq!(value.get("e"), Some(&Json::Num("1.5".into())));
    assert_eq!(value.get("f").and_then(Json::as_str), Some("s"));
    assert_eq!(
        value.get("g").map(Json::canonical).as_deref(),
        Some("[1, \"x\"]")
    );
    assert_eq!(
        value.get("h").map(Json::canonical).as_deref(),
        Some("{\"k\": 0}")
    );
    assert_eq!(value.get("zz"), None);
    assert_eq!(Json::Null.get("a"), None);
    assert_eq!(Json::Null.as_str(), None);
}

#[test]
fn a_duplicate_key_and_bad_text_are_refused() {
    assert!(parse(r#"{"a": 1, "a": 2}"#)
        .unwrap_err()
        .contains("duplicate key 'a'"));
    assert!(parse("{not json").is_err());
}

#[test]
fn truth_follows_python() {
    assert!(!Json::Null.truthy() && !Json::Bool(false).truthy() && Json::Bool(true).truthy());
    assert!(
        !Json::Num("0".into()).truthy()
            && Json::Num("2".into()).truthy()
            && !Json::Num("0.0".into()).truthy()
    );
    assert!(!Json::Str(String::new()).truthy() && Json::Str("x".into()).truthy());
    assert!(!Json::Arr(vec![]).truthy() && Json::Arr(vec![Json::Null]).truthy());
    assert!(!Json::Obj(vec![]).truthy() && Json::Obj(vec![("a".into(), Json::Null)]).truthy());
}

#[test]
fn show_and_canonical_sort_keys() {
    assert_eq!(Json::Str("x".into()).show(), "x");
    assert_eq!(Json::Num("3".into()).show(), "3");
    let unsorted = parse(r#"{"b": [true, null], "a": "q"}"#).unwrap();
    assert_eq!(unsorted.canonical(), "{\"a\": \"q\", \"b\": [true, null]}");
}

#[test]
fn the_reader_describes_what_it_expects() {
    struct Describe;
    impl Expected for Describe {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            serde::de::Visitor::expecting(&crate::json::testing::reader(), f)
        }
    }
    assert_eq!(format!("{}", &Describe as &dyn Expected), "any JSON value");
}
