//! check-adrs' own pieces: what the goldens cannot reach from a repository git made.

use crate::adr_log::{check_adr_log, AdrSource};
use crate::dirs::{Dirs, SystemDirs};
use crate::git::blob_path;
use std::collections::HashSet;

#[test]
fn a_tree_entry_is_a_blob_that_is_no_symbolic_link() {
    assert_eq!(blob_path("100644 blob abc123\tsrc/a b.ts"), Some("src/a b.ts"));
    assert_eq!(blob_path("100644 blob abc123\ta\tb"), Some("a\tb"));
    for entry in ["", "100644 blob abc123", "120000 blob abc123\tlink", "160000 commit abc123\tsub", "100644 blob abc123\t", "100644 blob\tx", "100644 blob a b\tx", "x blob abc\tx", " blob abc\tx", "100644 bl-ob abc\tx"] {
        assert_eq!(blob_path(entry), None, "{entry:?}");
    }
}

#[test]
fn a_record_outside_any_directory_links_from_the_repository_root() {
    let sources = [AdrSource { path: "README.md".to_string(), text: "[a](b.md)".to_string() }];
    let existing: HashSet<String> = ["b.md".to_string()].into_iter().collect();
    assert!(check_adr_log(&sources, &existing).unresolved_links.is_empty());
    assert_eq!(check_adr_log(&sources, &HashSet::new()).unresolved_links.len(), 1);
    let rooted = [AdrSource { path: "/x.md".to_string(), text: "[a](b.md)".to_string() }];
    assert_eq!(check_adr_log(&rooted, &HashSet::new()).unresolved_links[0].target, "b.md");
}

#[test]
fn a_directory_that_cannot_be_read_lists_nothing() {
    assert!(SystemDirs.list_files("/nonexistent/critique-dir").is_empty());
}
