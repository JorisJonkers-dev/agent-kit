//! Symbols across repositories and related repositories, on a small workspace.

use super::*;
use crate::boundary::is_withheld;
use crate::{repos, symbols};

/// A workspace root holding: `alpha` (git; needs beta), `beta` (git), `gamma`
/// (no git, an npm package needing alpha and beta), `nedap-tools` (git; needs
/// alpha, withheld), a loose file at the root, and a binary in `beta`.
struct Workspace {
    root: PathBuf,
    boundary: Boundary,
}

fn workspace() -> Workspace {
    let root = temp_dir().join("ws");
    write(&root.join("notes.rs"), "pub fn loose_fn() {}\n");
    write(&root.join("alpha/.git/HEAD"), "ref\n");
    write(
        &root.join("alpha/Cargo.toml"),
        "[package]\nname = \"alpha\"\n\n[dependencies]\nbeta = { path = \"../beta\" }\nserde = \"1\"\n",
    );
    write(&root.join("alpha/src/lib.rs"), "pub fn alpha_main() { beta_fn(); }\n");
    write(&root.join("alpha/crates/inner/Cargo.toml"), "[package]\nname = \"inner\"\n[dependencies]\nalpha = { path = \"../..\" }\n");
    write(&root.join("beta/.git"), "gitdir: elsewhere\n");
    write(&root.join("beta/Cargo.toml"), "[package]\nname = \"beta\"\n");
    write(&root.join("beta/src/lib.rs"), "pub fn beta_fn() {}\npub struct BetaThing;\nimpl BetaThing {}\n");
    fs::write(root.join("beta/blob.rs"), b"fn binary_fn() {}\0").unwrap();
    write(&root.join("gamma/package.json"), r#"{"name":"gamma","dependencies":{"alpha":"1","beta":"1","left-pad":"1"}}"#);
    write(&root.join("gamma/index.js"), "function gamma_fn() {}\nfunction Beta_Fn() {}\n");
    write(&root.join("nedap-tools/.git/HEAD"), "ref\n");
    write(&root.join("nedap-tools/Cargo.toml"), "[package]\nname = \"nedap-tools\"\n[dependencies]\nalpha = \"1\"\n");
    write(&root.join("nedap-tools/src/lib.rs"), "pub fn nedap_fn() {}\n");
    write(&root.join("alpha/Nedap/secret.rs"), "pub fn nedap_nested_fn() {}\n");
    let boundary = Boundary::new(std::slice::from_ref(&root)).unwrap();
    Workspace { root, boundary }
}

fn names(links: &[repos::Link]) -> Vec<String> {
    links.iter().map(|l| format!("{} via {}", l.repo, l.via)).collect()
}

fn call_tool(w: &Workspace, tool: &str, args: &str) -> String {
    let input = format!("{}\n", tool_call(tool, args));
    let mut out = Vec::new();
    server::serve(&w.boundary, 20, &mut input.as_bytes(), &mut out).unwrap();
    text_of(String::from_utf8(out).unwrap().lines().next().unwrap())
}

#[test]
fn related_repos_lists_depends_on_and_depended_on_by() {
    let w = workspace();
    let manifest = repos::manifest(w.boundary.roots());
    let repo_names: Vec<&str> = manifest.repos.iter().map(|r| r.name.as_str()).collect();
    // `nedap-tools` is never walked; the loose root file is no repository.
    assert_eq!(repo_names, ["alpha", "beta", "gamma"]);
    assert!(!manifest.walk_capped);
    let alpha = manifest.find("alpha").unwrap();
    let related = manifest.related(alpha);
    assert_eq!(names(&related.depends_on), ["beta via beta"]);
    assert_eq!(names(&related.depended_on_by), ["gamma via alpha"]);
    let beta = manifest.related(manifest.find("BETA").unwrap());
    assert!(beta.depends_on.is_empty());
    assert_eq!(names(&beta.depended_on_by), ["alpha via beta", "gamma via beta"]);
    let gamma = manifest.related(manifest.find(&w.root.join("gamma").display().to_string()).unwrap());
    assert_eq!(names(&gamma.depends_on), ["alpha via alpha", "beta via beta"]);
    assert!(manifest.find("nedap-tools").is_none());
    assert!(manifest.find("nowhere").is_none());
}

#[test]
fn the_related_repos_tool_prints_both_directions() {
    let w = workspace();
    let text = call_tool(&w, "related_repos", r#"{"repo":"alpha"}"#);
    let root = w.root.display();
    assert_eq!(text, format!("repo: alpha ({root}/alpha)\ndepends on (1):\nbeta (via beta)\ndepended on by (1):\ngamma (via alpha)"));
    let by_path = call_tool(&w, "related_repos", &format!(r#"{{"repo":"{root}/beta"}}"#));
    assert!(by_path.starts_with(&format!("repo: beta ({root}/beta)\ndepends on (0):\ndepended on by (2):\n")), "{by_path}");
}

#[test]
fn the_related_repos_tool_refuses_what_the_boundary_refuses() {
    let w = workspace();
    let outside = temp_dir().display().to_string();
    assert_eq!(call_tool(&w, "related_repos", "{}"), "repo is required");
    assert!(call_tool(&w, "related_repos", r#"{"repo":"ghost"}"#).contains("no repository named ghost"));
    assert!(call_tool(&w, "related_repos", &format!(r#"{{"repo":"{outside}"}}"#)).contains("outside the allowed roots"));
    assert!(call_tool(&w, "related_repos", r#"{"repo":"nedap-tools/src"}"#).contains("withheld"));
    assert!(call_tool(&w, "related_repos", r#"{"repo":"nedap-tools"}"#).contains("no repository named nedap-tools"));
}

#[test]
fn a_capped_walk_says_the_lists_may_be_incomplete() {
    let w = workspace();
    let manifest = repos::manifest_with(w.boundary.roots(), 2, &mut |_| None);
    assert!(manifest.walk_capped);
    assert_eq!(manifest.repos.len(), 1);
    assert!(manifest.repos[0].requires.is_empty());
}

#[test]
fn only_build_files_are_read() {
    let w = workspace();
    let mut reads: Vec<String> = Vec::new();
    repos::manifest_with(w.boundary.roots(), 100, &mut |path| {
        reads.push(path.strip_prefix(&w.root).unwrap().display().to_string());
        fs::read_to_string(path).ok()
    });
    assert_eq!(reads, ["alpha/Cargo.toml", "alpha/crates/inner/Cargo.toml", "beta/Cargo.toml", "gamma/package.json"]);
}

#[test]
fn a_repository_is_found_from_git_or_from_the_top_level_directory() {
    let w = workspace();
    let roots = w.boundary.roots();
    let mut finder = repos::RepoFinder::new(roots);
    assert_eq!(finder.repo_of(&w.root.join("alpha/crates/inner")), Some(w.root.join("alpha")));
    assert_eq!(finder.repo_of(&w.root.join("alpha/crates/inner")), Some(w.root.join("alpha")));
    assert_eq!(finder.repo_of(&w.root.join("gamma")), Some(w.root.join("gamma")));
    assert_eq!(finder.repo_of(&w.root), None);
    assert_eq!(finder.repo_of(&temp_dir()), None);
    // A root that is itself a repository owns every file under it.
    let single = temp_dir().join("single");
    write(&single.join(".git/HEAD"), "ref\n");
    write(&single.join("sub/a.rs"), "fn a() {}\n");
    let single_roots = [single.clone()];
    let mut finder = repos::RepoFinder::new(&single_roots);
    assert_eq!(finder.repo_of(&single.join("sub")), Some(single.clone()));
    let manifest = repos::manifest(&single_roots);
    assert_eq!(manifest.repos.len(), 1);
    assert_eq!(manifest.repos[0].path, single);
}

#[test]
fn symbols_find_a_name_across_repositories_exact_first() {
    let w = workspace();
    let found = symbols::find(w.boundary.roots(), w.boundary.roots(), "beta_fn", 10);
    let rows: Vec<(Option<&str>, &str, usize, &str)> =
        found.found.iter().map(|f| (f.repo.as_deref(), f.name.as_str(), f.line, f.kind.as_str())).collect();
    // Exact, then case-insensitive; `BetaThing` does not contain `beta_fn`.
    assert_eq!(rows, [(Some("beta"), "beta_fn", 1, "fn"), (Some("gamma"), "Beta_Fn", 2, "function")]);
    assert_eq!((found.total, found.walk_capped), (2, false));
    let sub = symbols::find(w.boundary.roots(), w.boundary.roots(), "beta", 10);
    let classes: Vec<(&str, u8)> = sub.found.iter().map(|f| (f.name.as_str(), f.class)).collect();
    // Same class: the shallower path first, then the line.
    assert_eq!(classes, [("Beta_Fn", 2), ("beta_fn", 2), ("BetaThing", 2)]);
}

#[test]
fn symbols_outside_a_repository_have_no_repo_and_refused_places_are_not_read() {
    let w = workspace();
    let loose = symbols::find(w.boundary.roots(), w.boundary.roots(), "loose_fn", 5);
    assert_eq!(loose.found.len(), 1);
    assert_eq!(loose.found[0].repo, None);
    for refused in ["nedap_fn", "nedap_nested_fn", "binary_fn"] {
        assert_eq!(symbols::find(w.boundary.roots(), w.boundary.roots(), refused, 5).total, 0, "{refused}");
    }
}

#[test]
fn symbols_cap_the_result_and_the_walk() {
    let w = workspace();
    let capped = symbols::find(w.boundary.roots(), w.boundary.roots(), "_fn", 1);
    assert_eq!((capped.found.len(), capped.total >= 4), (1, true));
    let mut many = String::new();
    for i in 0..30 {
        many.push_str(&format!("fn many_{i}() {{}}\n"));
    }
    write(&w.root.join("beta/many.rs"), &many);
    assert_eq!(symbols::find(w.boundary.roots(), w.boundary.roots(), "many_", 3).found.len(), 3);
    let walk = symbols::find_with(w.boundary.roots(), w.boundary.roots(), "fn", 5, 2, &mut |_| None);
    assert!(walk.walk_capped);
}

#[test]
fn the_symbols_tool_prints_repo_path_line_kind_and_name() {
    let w = workspace();
    let root = w.root.display();
    assert_eq!(call_tool(&w, "symbols", r#"{"name":"beta_fn"}"#), format!("beta {root}/beta/src/lib.rs:1 fn beta_fn\ngamma {root}/gamma/index.js:2 function Beta_Fn"));
    assert_eq!(call_tool(&w, "symbols", r#"{"name":"beta_fn","limit":1}"#), format!("beta {root}/beta/src/lib.rs:1 fn beta_fn\nshowing 1 of 2 symbols; narrow the name or path"));
    assert_eq!(call_tool(&w, "symbols", r#"{"name":"beta_fn","path":"gamma"}"#), format!("gamma {root}/gamma/index.js:2 function Beta_Fn"));
    assert_eq!(call_tool(&w, "symbols", r#"{"name":"absent_name"}"#), "no symbols");
    assert_eq!(call_tool(&w, "symbols", "{}"), "name is required");
    assert!(call_tool(&w, "symbols", r#"{"name":"x","path":"nedap-tools"}"#).contains("withheld"));
    let outside = temp_dir().display().to_string();
    assert!(call_tool(&w, "symbols", &format!(r#"{{"name":"x","path":"{outside}"}}"#)).contains("outside the allowed roots"));
}

#[test]
fn a_symbol_lookup_that_hit_the_file_cap_says_so() {
    let capped = symbols::Symbols { found: vec![], total: 0, walk_capped: true };
    assert_eq!(server::render_symbols(&capped), "no symbols\nstopped after the file cap; narrow the path");
}

#[test]
fn withheld_locations_are_never_admitted_or_read() {
    let w = workspace();
    assert!(is_withheld(Path::new("a/NEDAP/b")) && is_withheld(Path::new("nedap-tools")));
    assert!(!is_withheld(Path::new("alpha/src")));
    assert_eq!(w.boundary.admit(Path::new("alpha/Nedap/secret.rs")), Err(Refusal::Withheld(w.root.join("alpha/Nedap/secret.rs"))));
    assert!(Refusal::Withheld(PathBuf::from("/x")).to_string().contains("withheld location"));
    let hidden = w.root.join("alpha/Nedap");
    assert!(Boundary::new(&[hidden]).unwrap_err().contains("withheld location"));
    // Search and the map skip the same places.
    assert_eq!(search(w.boundary.roots(), "nedap_nested_fn", 5).total, 0);
    let repo = map::map(w.boundary.roots(), 50);
    assert!(repo.symbols.iter().all(|s| !s.name.starts_with("nedap")));
}

#[test]
fn a_related_listing_from_a_capped_walk_says_it_may_be_incomplete() {
    let w = workspace();
    let manifest = repos::manifest(w.boundary.roots());
    let alpha = manifest.find("alpha").unwrap();
    let text = server::render_related(alpha, &manifest.related(alpha), true);
    assert!(text.ends_with("stopped after the file cap; the lists may be incomplete"), "{text}");
}
