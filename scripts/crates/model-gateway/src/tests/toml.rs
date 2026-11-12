//! Tests for the TOML reader: what it reads, in what order, and what it refuses.

use crate::json::{object, Object, Value};
use crate::toml::read_toml;

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    object(pairs)
}

fn text(value: &str) -> Value {
    Value::str(value)
}

fn num(value: f64) -> Value {
    Value::Num(value)
}

/// The document `text` reads, or a panic naming what it refused.
fn read(text: &str) -> Value {
    match read_toml(text) {
        Ok(value) => value,
        Err(message) => panic!("refused {text:?}: {message}"),
    }
}

#[test]
fn read_toml_reads_an_empty_document_as_an_empty_table() {
    assert_eq!(read(""), Value::Obj(Object::new()));
    assert_eq!(read("\n# only a comment\n\n"), Value::Obj(Object::new()));
}

#[test]
fn read_toml_reads_every_shape_a_configuration_holds() {
    let document = read(
        "\
# the home's configuration
active_model = \"mistral-large-4-0\"   # the model a run starts on
enabled = true
disabled = false
ratio = 0.8
count = 1_000
hex = 0x10
when = 2026-11-09
tags = [\"a\", \"b\"]
empty = []
inline = { name = \"small\", provider = \"mistral\" }
\"quoted key\" = 1
dotted.key = 2

[models.mistral-large-4-0]
name = \"mistral-large-4-0\"
alias = \"mistral-large-latest\"

[[agents]]
name = \"one\"

[[agents]]
name = \"two\"
",
    );

    let expected = obj(vec![
        ("active_model", text("mistral-large-4-0")),
        ("enabled", Value::Bool(true)),
        ("disabled", Value::Bool(false)),
        ("ratio", num(0.8)),
        ("count", num(1000.0)),
        ("hex", num(16.0)),
        ("when", text("2026-11-09")),
        ("tags", Value::Arr(vec![text("a"), text("b")])),
        ("empty", Value::Arr(Vec::new())),
        ("inline", obj(vec![("name", text("small")), ("provider", text("mistral"))])),
        ("quoted key", num(1.0)),
        ("dotted", obj(vec![("key", num(2.0))])),
        ("models", obj(vec![("mistral-large-4-0", obj(vec![("name", text("mistral-large-4-0")), ("alias", text("mistral-large-latest"))]))])),
        (
            "agents",
            Value::Arr(vec![obj(vec![("name", text("one"))]), obj(vec![("name", text("two"))])]),
        ),
    ]);
    assert_eq!(document, expected);
}

#[test]
fn read_toml_keeps_the_order_the_file_declares() {
    let document = read("b = 1\na = 2\nc = 3\n");
    let keys: Vec<&str> = document.as_object().expect("a table").entries().into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["b", "a", "c"]);
}

#[test]
fn read_toml_reads_a_table_after_an_array_of_tables_as_its_latest_table() {
    let document = read("[[a]]\nb = 1\n[a.c]\nd = 2\n[[a]]\nb = 3\n");
    assert_eq!(
        document,
        obj(vec![(
            "a",
            Value::Arr(vec![
                obj(vec![("b", num(1.0)), ("c", obj(vec![("d", num(2.0))]))]),
                obj(vec![("b", num(3.0))]),
            ]),
        )])
    );
}

#[test]
fn read_toml_reads_a_table_declared_twice_as_one_table() {
    assert_eq!(read("[a]\nb = 1\n[a]\nc = 2\n"), obj(vec![("a", obj(vec![("b", num(1.0)), ("c", num(2.0))]))]));
}

#[test]
fn read_toml_reads_multi_line_and_literal_strings() {
    let document = read("a = \"\"\"\nline one\nline two\"\"\"\nb = '''\r\nliteral'''\nc = 'raw \\n text'\nd = \"\"\"\"\"\"\n");
    assert_eq!(
        document,
        obj(vec![
            ("a", text("line one\nline two")),
            ("b", text("literal")),
            ("c", text("raw \\n text")),
            ("d", text("")),
        ])
    );
}

#[test]
fn read_toml_reads_the_escapes_a_basic_string_carries() {
    let document = read("a = \"q\\\"b\\\\s\\bn\\nf\\fr\\rt\\tu\\u00e9U\\U0001F600\"\n");
    assert_eq!(document, obj(vec![("a", text("q\"b\\s\u{8}n\nf\u{c}r\rt\tu\u{e9}U\u{1f600}"))]));
}

#[test]
fn read_toml_reads_a_lone_surrogate_escape_as_the_replacement_character() {
    assert_eq!(read("a = \"\\uD800\"\n"), obj(vec![("a", text("\u{fffd}"))]));
}

#[test]
fn read_toml_reads_numbers_as_javascript_reads_them() {
    let cases = [
        ("1", num(1.0)),
        ("-1", num(-1.0)),
        ("+5", num(5.0)),
        ("1.5", num(1.5)),
        (".5", num(0.5)),
        ("1.", num(1.0)),
        ("1e3", num(1000.0)),
        ("1E+3", num(1000.0)),
        ("1e-3", num(0.001)),
        ("1e999", num(f64::INFINITY)),
        ("1_000_000", num(1_000_000.0)),
        ("0x10", num(16.0)),
        ("0X10", num(16.0)),
        ("0o17", num(15.0)),
        ("0b101", num(5.0)),
        ("Infinity", num(f64::INFINITY)),
        ("-Infinity", num(f64::NEG_INFINITY)),
        ("inf", text("inf")),
        ("nan", text("nan")),
        ("-0x10", text("-0x10")),
        ("0x", text("0x")),
        ("0xzz", text("0xzz")),
        ("1e", text("1e")),
        ("1.2.3", text("1.2.3")),
        ("2026-11-09", text("2026-11-09")),
        ("yes", text("yes")),
    ];
    for (word, expected) in cases {
        assert_eq!(read(&format!("a = {word}\n")), obj(vec![("a", expected)]), "{word}");
    }
}

#[test]
fn read_toml_reads_a_trailing_comma_and_a_comment_inside_an_array() {
    assert_eq!(read("a = [\n  1, # one\n  2,\n]\n"), obj(vec![("a", Value::Arr(vec![num(1.0), num(2.0)]))]));
}

#[test]
fn read_toml_reads_an_empty_inline_table() {
    assert_eq!(read("a = {}\n"), obj(vec![("a", Value::Obj(Object::new()))]));
}

#[test]
fn read_toml_reads_a_document_that_ends_without_a_newline() {
    assert_eq!(read("a = 1"), obj(vec![("a", num(1.0))]));
}

#[test]
fn read_toml_reads_a_carriage_return_line_ending() {
    assert_eq!(read("a = 1\r\nb = 2\r\n"), obj(vec![("a", num(1.0)), ("b", num(2.0))]));
}

#[test]
fn read_toml_refuses_what_it_cannot_read() {
    let cases = [
        "this is not TOML\n",
        "= 1\n",
        "a = \n",
        "a = \"unterminated\n",
        "a = \"unterminated",
        "a = \"\\q\"\n",
        "a = \"\\u12",
        "a = \"\\u12zz\"\n",
        "a = \"\\U00110000\"\n",
        "a = [1 2]\n",
        "a = {b = 1 c = 2}\n",
        "a = 1 b = 2\n",
        "a = 1\na = 2\n",
        "a = {b = 1, b = 2}\n",
        "a = 1\n[a]\n",
        "a = 1\n[[a]]\n",
        "a = []\n[a]\n",
        "a = {}\n[[a]]\n",
        "[a\n",
        "[[a]\n",
    ];
    for case in cases {
        assert!(read_toml(case).is_err(), "read {case:?}");
    }
}

#[test]
fn read_toml_names_the_line_it_refused() {
    let message = read_toml("a = 1\nb = \n").expect_err("a value it cannot read");
    assert_eq!(message, "line 2: expected a value");
}
