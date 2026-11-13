//! The read-only tripwire: what it compares, what it restores, and the porcelain listing it reads.

use super::manifest::manifest_text;
use super::scenarios::Sandbox;
use crate::git::{parse_porcelain, Git, PorcelainEntry};
use crate::manifest::parse_manifest;
use crate::tripwire::{packet_changes, working_tree_changes, PacketState, ReadOnlyTripwire, WorkingTreeEntry, WorkingTreeState};
use crate::Runtime;
use model_gateway::ports::GitRemote;
use model_gateway::sha256::sha256_hex;
use std::sync::Mutex;

fn state(entries: &[(&str, &str)]) -> PacketState {
    entries.iter().map(|(path, hash)| (path.to_string(), hash.to_string())).collect()
}

#[test]
fn a_packet_change_is_added_removed_or_changed_by_path() {
    let before = state(&[("a", "1"), ("b", "1"), ("c", "1")]);
    let after = state(&[("a", "1"), ("b", "2"), ("d", "1")]);
    assert_eq!(packet_changes("packet", &before, &after), ["packet: changed b", "packet: removed c", "packet: added d"]);
    assert!(packet_changes("packet", &before, &before).is_empty());
}

#[test]
fn a_working_tree_change_names_the_status_after_or_clean() {
    let entry = |code: &str, hash: &str| WorkingTreeEntry { code: code.to_string(), hash: hash.to_string() };
    let before: WorkingTreeState = [("a".to_string(), entry(" M", "1")), ("b".to_string(), entry("??", "1")), ("c".to_string(), entry(" M", "1"))].into_iter().collect();
    let after: WorkingTreeState = [("a".to_string(), entry(" M", "2")), ("b".to_string(), entry("??", "1")), ("d".to_string(), entry("A ", "1"))].into_iter().collect();
    assert_eq!(working_tree_changes(&before, &after), ["working tree: M a", "working tree: clean c", "working tree: A d"]);
}

#[test]
fn a_porcelain_listing_skips_the_source_of_a_rename_or_copy() {
    let listing = " M src/a.rs\0R  new name.rs\0old name.rs\0?? x\0C  copy.rs\0from.rs\0\0A  é.rs\0";
    let found = parse_porcelain(listing);
    let pairs: Vec<(&str, &str)> = found.iter().map(|entry| (entry.code.as_str(), entry.path.as_str())).collect();
    assert_eq!(pairs, [(" M", "src/a.rs"), ("R ", "new name.rs"), ("??", "x"), ("C ", "copy.rs"), ("A ", "é.rs")]);
    assert!(parse_porcelain("").is_empty());
    assert_eq!(parse_porcelain("ab\0"), vec![PorcelainEntry { code: "ab".to_string(), path: String::new() }]);
}

struct Status(Mutex<Result<Vec<PorcelainEntry>, String>>);

impl Git for Status {
    fn commit_log(&self, _cwd: &str, _revision: &str, _paths: &[String], _limit: usize) -> Result<Vec<crate::recovery::HistoryEntry>, String> {
        Err("unused".to_string())
    }

    fn remotes(&self, _cwd: &str) -> Result<Vec<GitRemote>, String> {
        Err("unused".to_string())
    }
    fn common_dir(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn root(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn resolve_commit(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn last_change(&self, _cwd: &str, _path: &str) -> Result<Option<crate::git::LastChange>, String> {
        Err("unused".to_string())
    }
    fn file_commits(&self, _cwd: &str, _revision: &str, _path: &str) -> Result<Vec<crate::git::LastChange>, String> {
        Err("unused".to_string())
    }
    fn file_at(&self, _cwd: &str, _commit: &str, _path: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn tree_files(&self, _cwd: &str, _commit: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }
    fn untracked_paths(&self, _cwd: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }
    fn merge_base(&self, _cwd: &str, _a: &str, _b: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn diff_commits(&self, _cwd: &str, _base: &str, _head: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn changed_paths(&self, _cwd: &str, _base: &str, _head: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }
    fn staged_entries(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn tree_of(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn write_index_tree(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn commit_tree(&self, _cwd: &str, _tree: &str, _parent: &str, _message: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn commits_between(&self, _cwd: &str, _base: &str, _head: &str) -> Result<Vec<crate::git::CommitLink>, String> {
        Err("unused".to_string())
    }
    fn tree_entries(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn merge_tree(&self, _cwd: &str, _ours: &str, _theirs: &str) -> Result<crate::git::MergeTree, String> {
        Err("unused".to_string())
    }
    fn diff_against_head(&self, _cwd: &str, _staged: bool) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn paths_against_head(&self, _cwd: &str, _staged: bool) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }
    fn current_branch(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
    fn porcelain_status(&self, _cwd: &str) -> Result<Vec<PorcelainEntry>, String> {
        self.0.lock().expect("lock").clone()
    }
}

fn runtime(sandbox: &Sandbox, status: Result<Vec<PorcelainEntry>, String>) -> Runtime {
    let mut base = super::cli::runtime("");
    base.git = Box::new(Status(Mutex::new(status)));
    base.files = Box::new(model_gateway::adapters::fs::SystemFiles { cwd: sandbox.text() });
    base
}

fn write(path: &str, text: &str) {
    std::fs::create_dir_all(std::path::Path::new(path).parent().expect("a parent")).expect("dirs");
    std::fs::write(path, text).expect("write");
}

/// A manifest whose packet holds `a.md` and `sub/b.md`, in the sandbox, with the repository root beside it.
fn manifest(sandbox: &Sandbox, hash: &str) -> crate::manifest::RunManifest {
    let root = sandbox.text();
    let files = format!(
        r#"[{{"kind":"artifact","path":"a.md","sha256":"{}","source":"s"}},{{"kind":"artifact","path":"sub/b.md","sha256":"{hash}","source":"s"}}]"#,
        sha256_hex(b"A")
    );
    let text = manifest_text(&[("packet_dir", &format!("\"{root}/packet\"")), ("repo_root", &format!("\"{root}/repo\"")), ("packet_files", &files)]);
    parse_manifest(&text).expect("a manifest")
}

#[test]
fn a_packet_that_no_longer_matches_its_manifest_is_refused() {
    let sandbox = Sandbox::new();
    let root = sandbox.text();
    write(&format!("{root}/packet/a.md"), "A");
    write(&format!("{root}/packet/sub/b.md"), "B");
    let run = runtime(&sandbox, Ok(Vec::new()));
    let error = ReadOnlyTripwire::load(&run, &manifest(&sandbox, "0000")).err().expect("a refusal");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("invalid-run", "sub/b.md in the packet no longer matches the manifest"));
    std::fs::remove_file(format!("{root}/packet/a.md")).expect("remove");
    assert!(ReadOnlyTripwire::load(&run, &manifest(&sandbox, &sha256_hex(b"B"))).is_err());
}

#[test]
fn a_critic_that_writes_anywhere_it_could_is_caught_and_the_packets_are_restored() {
    let sandbox = Sandbox::new();
    let root = sandbox.text();
    write(&format!("{root}/packet/a.md"), "A");
    write(&format!("{root}/packet/sub/b.md"), "B");
    write(&format!("{root}/repo/owned.txt"), "mine");
    let manifest = manifest(&sandbox, &sha256_hex(b"B"));
    let quiet = runtime(&sandbox, Ok(vec![PorcelainEntry { code: " M".to_string(), path: "owned.txt".to_string() }]));
    let tripwire = ReadOnlyTripwire::load(&quiet, &manifest).expect("a tripwire");
    let copy = format!("{root}/critic/packet");
    // The copy is written from the frozen files, over a stray one.
    write(&format!("{copy}/stray.txt"), "left over");
    let armed = tripwire.arm(&quiet, &copy).expect("armed");
    assert_eq!(std::fs::read_to_string(format!("{copy}/sub/b.md")).expect("copied"), "B");
    assert!(!std::path::Path::new(&format!("{copy}/stray.txt")).exists());
    assert!(tripwire.check(&quiet, &armed).expect("checked").is_empty());
    // A critic that edits its copy, the run's packet and a file the owner had already changed.
    write(&format!("{copy}/a.md"), "edited");
    write(&format!("{copy}/new.md"), "new");
    write(&format!("{root}/packet/sub/b.md"), "edited");
    write(&format!("{root}/repo/owned.txt"), "theirs");
    let changes = tripwire.check(&quiet, &armed).expect("checked");
    assert_eq!(changes, ["packet: changed a.md", "packet: added new.md", "run packet: changed sub/b.md", "working tree: M owned.txt"]);
    assert_eq!(std::fs::read_to_string(format!("{copy}/a.md")).expect("restored"), "A");
    assert!(!std::path::Path::new(&format!("{copy}/new.md")).exists());
    assert_eq!(std::fs::read_to_string(format!("{root}/packet/sub/b.md")).expect("restored"), "B");
}

#[test]
fn a_status_git_cannot_give_is_an_error_and_an_unreadable_file_counts_as_one() {
    let sandbox = Sandbox::new();
    let root = sandbox.text();
    write(&format!("{root}/packet/a.md"), "A");
    write(&format!("{root}/packet/sub/b.md"), "B");
    let manifest = manifest(&sandbox, &sha256_hex(b"B"));
    let good = runtime(&sandbox, Ok(vec![PorcelainEntry { code: "??".to_string(), path: "gone.txt".to_string() }]));
    let tripwire = ReadOnlyTripwire::load(&good, &manifest).expect("a tripwire");
    let armed = tripwire.arm(&good, &format!("{root}/critic")).expect("armed");
    // A listed file that cannot be read has a hash of its own, so the state still compares.
    assert!(tripwire.check(&good, &armed).expect("checked").is_empty());
    let broken = runtime(&sandbox, Err("git is gone".to_string()));
    assert_eq!(tripwire.arm(&broken, &format!("{root}/critic")).err().map(|error| error.message), Some("git is gone".to_string()));
}

#[test]
fn a_tree_lists_its_files_at_any_depth_and_leaves_out_what_is_not_text() {
    use crate::dirs::{Dirs, SystemDirs};
    use std::os::unix::ffi::OsStrExt;
    let sandbox = Sandbox::new();
    let root = sandbox.text();
    write(&format!("{root}/t/b.txt"), "b");
    write(&format!("{root}/t/a/é.txt"), "é");
    std::fs::write(std::path::Path::new(&root).join("t").join(std::ffi::OsStr::from_bytes(b"bad\xff")), "x").expect("a name that is not text");
    assert_eq!(SystemDirs.list_tree(&format!("{root}/t")), ["a/é.txt", "b.txt"]);
    assert!(SystemDirs.list_tree(&format!("{root}/missing")).is_empty());
    assert_eq!(SystemDirs.read_bytes(&format!("{root}/t/b.txt")), Some(b"b".to_vec()));
    assert_eq!(SystemDirs.read_bytes(&format!("{root}/t")), None);
}
