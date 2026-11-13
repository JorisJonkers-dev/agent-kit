//! The record foundation's own readers, where the corpus replays only files the node runner wrote itself.

use crate::feature_review::parse_gate_record;

#[test]
fn a_gate_record_that_is_not_an_object_or_has_a_wrong_field_is_refused_in_the_node_runners_words() {
    assert_eq!(parse_gate_record("{").map(|_| ()), Err("Expected property name or '}' in JSON at position 1 (line 1 column 2)".to_string()));
    assert_eq!(parse_gate_record("[1]").map(|_| ()), Err("the value must be a JSON object".to_string()));
    assert_eq!(parse_gate_record("{}").map(|_| ()), Err("base_commit must be a non-empty string".to_string()));
}

#[test]
fn a_gate_record_names_the_path_of_a_wrong_follow_up() {
    let head = r#""base_commit":"a","critical_open":0"#;
    assert_eq!(parse_gate_record(&format!("{{{head},\"follow_ups\":[{{}}]}}")).map(|_| ()), Err("follow_ups[0].finding_id must be a non-empty string".to_string()));
    let tail = r#""follow_ups":[],"gate":"pass","head_commit":"b","line":"GATE: PASS""#;
    assert_eq!(parse_gate_record(&format!("{{{head},{tail},\"open_critical\":[{{}}]}}")).map(|_| ()), Err("open_critical[0].finding_id must be a non-empty string".to_string()));
}
