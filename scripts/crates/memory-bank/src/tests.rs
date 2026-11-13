use super::*;
use std::cell::RefCell;
use std::collections::BTreeMap;

fn remote(name: &str, url: &str) -> GitRemote {
    GitRemote { name: name.to_string(), url: url.to_string() }
}

/// An in-process memory server: one store per bank, every call logged with the bank it reached.
#[derive(Default)]
struct Stub {
    stores: RefCell<BTreeMap<&'static str, Vec<String>>>,
    calls: RefCell<Vec<(&'static str, &'static str)>>,
    fail: bool,
}

impl Memory for Stub {
    fn recall(&self, bank: Bank, query: &str) -> Result<Vec<String>, String> {
        self.calls.borrow_mut().push(("recall", bank.name()));
        if self.fail {
            return Err("server down".to_string());
        }
        let stores = self.stores.borrow();
        Ok(stores.get(bank.name()).map(|facts| facts.iter().filter(|fact| fact.contains(query)).cloned().collect()).unwrap_or_default())
    }

    fn retain(&self, bank: Bank, content: &str) -> Result<(), String> {
        self.calls.borrow_mut().push(("retain", bank.name()));
        if self.fail {
            return Err("server down".to_string());
        }
        self.stores.borrow_mut().entry(bank.name()).or_default().push(content.to_string());
        Ok(())
    }
}

struct FakeGit {
    root: Result<Option<String>, String>,
    remotes: Result<Vec<GitRemote>, String>,
}

impl FakeGit {
    fn with(remotes: Vec<GitRemote>) -> FakeGit {
        FakeGit { root: Ok(Some("/repo".to_string())), remotes: Ok(remotes) }
    }
}

impl Remotes for FakeGit {
    fn in_repository(&self, _path: &str) -> Result<bool, String> {
        self.root.clone().map(|root| root.is_some())
    }
    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        self.remotes.clone()
    }
}

fn nedap() -> FakeGit {
    FakeGit::with(vec![remote("origin", "git@github.com:nedap/platform.git")])
}

fn personal() -> FakeGit {
    FakeGit::with(vec![remote("origin", "git@github.com:JorisJonkers-dev/agent-kit.git")])
}

#[test]
fn a_nedap_session_recalls_and_retains_in_the_nedap_bank_only() {
    let memory = Stub::default();
    let session = Session::open(&nedap(), "/repo", &memory).unwrap();
    session.retain("deploy window is friday").unwrap();
    assert_eq!(session.recall("deploy").unwrap(), vec!["deploy window is friday".to_string()]);
    assert_eq!(session.bank(), Bank::Nedap);
    assert_eq!(*memory.calls.borrow(), vec![("retain", "nedap"), ("recall", "nedap")]);
    assert!(!memory.stores.borrow().contains_key("shared"));
}

#[test]
fn a_nedap_session_does_not_recall_what_shared_memory_holds() {
    let memory = Stub::default();
    Session::open(&personal(), "/repo", &memory).unwrap().retain("deploy notes").unwrap();
    assert!(Session::open(&nedap(), "/repo", &memory).unwrap().recall("deploy").unwrap().is_empty());
}

#[test]
fn a_personal_session_never_reaches_the_nedap_bank() {
    let memory = Stub::default();
    Session::open(&nedap(), "/repo", &memory).unwrap().retain("employer fact").unwrap();
    memory.calls.borrow_mut().clear();
    let session = Session::open(&personal(), "/repo", &memory).unwrap();
    session.retain("personal fact").unwrap();
    assert!(session.recall("employer").unwrap().is_empty());
    assert_eq!(session.recall("personal").unwrap(), vec!["personal fact".to_string()]);
    assert!(memory.calls.borrow().iter().all(|(_, bank)| *bank == "shared"));
}

#[test]
fn a_session_with_no_remote_is_shared_memory() {
    let memory = Stub::default();
    let session = Session::open(&FakeGit::with(Vec::new()), "/repo", &memory).unwrap();
    assert_eq!(session.bank(), Bank::Shared);
    session.retain("x").unwrap();
    assert_eq!(*memory.calls.borrow(), vec![("retain", "shared")]);
}

#[test]
fn a_directory_that_is_no_repository_is_shared_memory() {
    let git = FakeGit { root: Ok(None), remotes: Err("never asked".to_string()) };
    assert_eq!(resolve_path(&git, "/tmp/x"), Ok(Bank::Shared));
}

#[test]
fn a_nedap_remote_wins_over_origin_on_any_host_or_alias() {
    let remotes = vec![remote("origin", "git@github.com:me/fork.git"), remote("work", "git@github-work:Nedap/platform.git")];
    assert_eq!(resolve(&remotes), Ok(Bank::Nedap));
    assert_eq!(resolve(&[remote("origin", "https://github.com/NEDAP/x")]), Ok(Bank::Nedap));
    assert_eq!(resolve(&[remote("origin", "ssh://git@github.com/nedap/x.git")]), Ok(Bank::Nedap));
}

#[test]
fn an_owner_that_only_contains_nedap_is_shared_memory() {
    assert_eq!(resolve(&[remote("origin", "git@github.com:nedap-fan/x.git")]), Ok(Bank::Shared));
    assert_eq!(resolve(&[remote("origin", "git@github.com:me/nedap.git")]), Ok(Bank::Shared));
}

#[test]
fn a_failure_to_read_the_remotes_is_never_a_bank() {
    let git = FakeGit { root: Ok(Some("/repo".to_string())), remotes: Err("boom".to_string()) };
    assert_eq!(resolve_path(&git, "/repo"), Err("boom".to_string()));
    let git = FakeGit { root: Err("no git".to_string()), remotes: Ok(Vec::new()) };
    assert!(Session::open(&git, "/repo", &Stub::default()).is_err());
}

#[test]
fn a_backend_failure_reaches_the_caller() {
    let memory = Stub { fail: true, ..Stub::default() };
    let session = Session::open(&personal(), "/repo", &memory).unwrap();
    assert_eq!(session.recall("x"), Err("server down".to_string()));
    assert_eq!(session.retain("x"), Err("server down".to_string()));
}

#[test]
fn each_bank_has_its_registry_name_and_hindsight_id() {
    assert_eq!((Bank::Shared.name(), Bank::Shared.id()), ("shared", "default"));
    assert_eq!((Bank::Nedap.name(), Bank::Nedap.id()), ("nedap", "nedap"));
}

fn command(args: &[&str], git: &FakeGit) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_with(args.iter().map(OsString::from), git, &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn resolve_prints_the_bank_as_one_json_object() {
    assert_eq!(command(&["resolve"], &nedap()), (0, "{\"bank\":\"nedap\",\"bank_id\":\"nedap\"}\n".to_string(), String::new()));
    assert_eq!(command(&["resolve", "/p"], &personal()), (0, "{\"bank\":\"shared\",\"bank_id\":\"default\"}\n".to_string(), String::new()));
}

#[test]
fn resolve_fails_closed_when_the_remotes_cannot_be_read() {
    let git = FakeGit { root: Ok(Some("/repo".to_string())), remotes: Err("boom".to_string()) };
    let (code, out, err) = command(&["resolve", "/p"], &git);
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(err.contains("boom"));
}

#[test]
fn bad_usage_exits_two_with_the_usage_text() {
    for args in [&[][..], &["whoami"], &["resolve", "a", "b"]] {
        let (code, out, err) = command(args, &nedap());
        assert_eq!((code, out.as_str(), err.as_str()), (2, "", USAGE));
    }
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git").arg("-C").arg(dir).args(args).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn the_real_git_resolves_a_repository_by_its_remote_owner() {
    let base = std::env::temp_dir().join(format!("memory-bank-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for (name, url, want) in [("work", "git@github.com:nedap/platform.git", "nedap"), ("home", "git@github.com:JorisJonkers-dev/x.git", "shared")] {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["remote", "add", "origin", url]);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run([OsString::from("resolve"), dir.clone().into_os_string()], &mut out, &mut err);
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        assert!(String::from_utf8(out).unwrap().contains(&format!("\"bank\":\"{want}\"")));
    }
    let bare = base.join("plain");
    std::fs::create_dir_all(&bare).unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(run([OsString::from("resolve"), bare.into_os_string()], &mut out, &mut err), 0);
    assert!(String::from_utf8(out).unwrap().contains("\"bank\":\"shared\""));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_remote_that_names_no_owner_is_an_error_not_shared_memory() {
    assert!(resolve(&[remote("origin", "/srv/mirrors/platform.git")]).is_err());
    assert!(resolve(&[remote("origin", "git@github.com:me/x.git"), remote("mirror", "file:///srv/nedap")]).is_err());
}

#[test]
fn a_nedap_upstream_beside_a_personal_origin_is_the_nedap_bank() {
    assert_eq!(resolve(&[remote("origin", "git@github.com:me/fork.git"), remote("upstream", "git@github.com:nedap/x.git")]), Ok(Bank::Nedap));
}
