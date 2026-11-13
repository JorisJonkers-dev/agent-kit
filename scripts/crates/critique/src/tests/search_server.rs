//! What the corpus does not reach: the search server's command-line numbers, and its start through `run_with`.

use crate::search_server::js_number;

#[test]
fn a_command_line_number_reads_as_javascript_number_reads_it() {
    let cases: [(&str, Option<f64>); 12] = [
        ("", Some(0.0)),
        ("  ", Some(0.0)),
        (" 5 ", Some(5.0)),
        ("0x1F", Some(31.0)),
        ("0b", None),
        ("Infinity", Some(f64::INFINITY)),
        ("-Infinity", Some(f64::NEG_INFINITY)),
        ("1e3", Some(1000.0)),
        (".5", Some(0.5)),
        ("1e", None),
        ("inf", None),
        ("1.2.3", None),
    ];
    for (text, number) in cases {
        assert_eq!(js_number(text), number, "{text:?}");
    }
}

#[test]
fn the_search_server_starts_from_the_command_line() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let runtime = crate::runtime::default_runtime();
    let code = crate::run_with(["search-server".to_string()].into_iter(), &runtime, &mut std::io::empty(), &mut out, &mut err);
    assert_eq!((code, String::from_utf8_lossy(&err).into_owned()), (2, crate::search_server::SEARCH_SERVER_USAGE.to_string()));
}

#[test]
fn a_file_system_failure_reads_as_node_words_it() {
    let missing = std::fs::read("/nonexistent/critique-search");
    assert_eq!(crate::search_server::io(missing, "open", "/x").unwrap_err(), "ENOENT: no such file or directory, open '/x'");
}
