//! The import scan, the source scan and the code facts, by hand; the corpus replay (`scan_corpus`)
//! holds the node runner's own answers over generated sources.

use crate::code_facts::{code_facts, history_facts, FileHistory, ModuleSource};
use crate::git::LastChange;
use crate::import_scan::{language_of, scan_imports, ImportKind, ScanLanguage};
use crate::source_scan::{self, mention_lines, mentions, normalize, posix_dirname, posix_extname, posix_join, scan_source};
use std::collections::HashSet;

#[test]
fn a_path_is_scanned_as_its_extension_says() {
    for (path, language) in [
        ("a.ts", Some(ScanLanguage::Typescript)),
        ("a/b.cjs", Some(ScanLanguage::Typescript)),
        ("a.py", Some(ScanLanguage::Python)),
        ("a.kts", Some(ScanLanguage::Kotlin)),
        ("a.go", Some(ScanLanguage::Go)),
        ("a.rs", Some(ScanLanguage::Rust)),
        ("a.TS", None),
        ("noext", None),
        ("a.d/file", None),
        ("README.md", None),
    ] {
        assert_eq!(language_of(path), language, "{path}");
    }
    assert_eq!(source_scan::language_of("a.tsx"), Some("typescript"));
    assert_eq!(source_scan::language_of("a/.py"), None);
    assert_eq!(source_scan::language_of("a/b.py"), Some("python"));
    assert_eq!(source_scan::language_of(".py"), None);
    assert_eq!(source_scan::language_of("a.kt"), None);
    assert_eq!(source_scan::resolve_import("a.kt", "./b", &HashSet::new()), None);
    let tree: HashSet<String> = ["src/pkg/n.py", "other/pkg/n.py", "src/pkg/m/__init__.py"].iter().map(|path| path.to_string()).collect();
    assert_eq!(source_scan::resolve_import("x.py", "pkg.m", &tree), Some("src/pkg/m/__init__.py".to_string()));
    assert_eq!(source_scan::resolve_import("x.py", "pkg.n", &tree), None);
    assert_eq!(scan_source("a.kt", "import a.B"), None);
}

#[test]
fn imports_come_in_order_of_first_appearance_without_repeats() {
    let found = scan_imports(ScanLanguage::Rust, "mod b;\nuse a::b;\nuse a::b::;\nmod b;\n");
    let seen: Vec<(&str, ImportKind)> = found.iter().map(|import| (import.specifier.as_str(), import.kind)).collect();
    assert_eq!(seen, [("b", ImportKind::Module), ("a::b", ImportKind::Import)]);
    let go = scan_imports(ScanLanguage::Go, "import \"a\"\nimport (\n  \"b\"\n  x \"c\"\n)\nvar s = \"d\"\n/* import \"e\"\n*/\nimport (\n \"f\"\n  )\n  \"g\"\n");
    assert_eq!(go.iter().map(|import| import.specifier.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "f"]);
}

#[test]
fn the_posix_helpers_answer_as_node_does_for_the_odd_cases() {
    assert_eq!((normalize(""), normalize("/"), normalize("./"), normalize("a/.."), normalize("a/../"), normalize("/..")), (".".into(), "/".into(), "./".into(), ".".into(), "./".into(), "/".into()));
    assert_eq!((normalize("../a/../.."), normalize("/a/b/"), normalize("a//b/./c")), ("../..".into(), "/a/b/".into(), "a/b/c".into()));
    assert_eq!((posix_dirname(""), posix_dirname("/"), posix_dirname("/a"), posix_dirname("//a"), posix_dirname("a/b/"), posix_dirname("a")), (".".into(), "/".into(), "/".into(), "//".into(), "a".into(), ".".into()));
    assert_eq!((posix_extname("a.ts"), posix_extname(".ts"), posix_extname("a/.ts"), posix_extname("a."), posix_extname("a.b.c"), posix_extname("..")), (".ts", "", "", ".", ".c", ""));
    assert_eq!((posix_extname("a/b/"), posix_extname("a.b/"), posix_extname("..a"), posix_extname("")), ("", ".b", ".a", ""));
    assert_eq!(posix_join("a/b", "../c"), "a/c");
}

#[test]
fn a_declaration_s_signature_is_shortened_to_the_repo_map_s_width() {
    let long = format!("export function f({}) {{\n}}\n", "parameter, ".repeat(30));
    let scan = scan_source("a.ts", &long).expect("a scan");
    assert_eq!(scan.symbols.len(), 1);
    let signature = &scan.symbols[0].signature;
    assert_eq!(signature.chars().count(), 200);
    assert!(signature.starts_with("export function f(parameter, parameter,") && signature.ends_with('…'), "{signature}");
    let scan = scan_source("a.py", "x: int = (1)\nasync  def g(a):\n    pass\n\n\nclass C:\n").expect("a scan");
    let shown: Vec<(&str, &str, usize, usize)> = scan.symbols.iter().map(|symbol| (symbol.kind.as_str(), symbol.signature.as_str(), symbol.first, symbol.last)).collect();
    assert_eq!(shown, [("assignment", "x: int", 1, 1), ("async def", "async def g(a)", 2, 3), ("class", "class C", 6, 6)]);
}

#[test]
fn a_word_is_mentioned_whole_or_not_at_all() {
    assert!(mentions("call(foo)", "foo"));
    assert!(!mentions("foobar $foo foo$", "foo"));
    assert!(!mentions("fo", "foo"));
    assert_eq!(mention_lines("foo\nxfoo\n(foo)\r\nfoo_", "foo"), [1, 3]);
}

#[test]
fn code_facts_name_what_was_not_computed_and_keep_a_repeated_path_once_as_a_node() {
    let tree: HashSet<String> = ["a.ts", "b.ts", "c.py"].iter().map(|path| path.to_string()).collect();
    let source = |path: &str, text: Option<&str>| ModuleSource { path: path.to_string(), text: text.map(str::to_string) };
    let facts = code_facts(&[source("a.ts", Some("import './b'\n")), source("b.ts", Some("import './a'\n")), source("c.py", None), source("d.txt", Some("")), source("a.ts", Some("import './b'"))], &tree);
    assert_eq!(facts.notes, ["c.py: no import facts were computed (the file is not in the commit)", "d.txt: no import facts were computed (the import scan does not know this language)"]);
    assert_eq!(facts.cycles, [["a.ts", "b.ts"]]);
    assert_eq!(facts.modules.len(), 5);
    assert_eq!((facts.modules[0].fan_in, facts.modules[0].fan_out, facts.modules[0].instability), (1, 1, Some(0.5)));
    assert_eq!(facts.modules[3].instability, None);
    let history = history_facts(&[FileHistory { path: "x".to_string(), commits: vec![LastChange { commit: "c".to_string(), date: "d".to_string() }, LastChange { commit: "e".to_string(), date: "f".to_string() }] }]);
    assert_eq!((history[0].churn, history[0].last_commit.as_deref(), history[0].last_date.as_deref()), (2, Some("c"), Some("d")));
}
