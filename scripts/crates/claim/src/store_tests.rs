//! The setup canary crate: the Rust workspace's proof of path.
//!
//! The standards repo's ADR-0014 puts all of agent-kit's Rust in one workspace
//! under `scripts/`. This member is deliberately trivial and std-only, with no
//! dependency of any kind.
//! `--version` proves the build end to end; the command also validates a path.
//! New Rust joins this workspace as a new member, never as its own Cargo
//! project.
//!
//! Tests for the claim store: a real local bare repository for the locking
//! races and a scripted git stub for the error paths.

use super::*;
use crate::lease::{Clock, FixedClock, Lease};
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    base: PathBuf,
    url: String,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "claim-store-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let bare = base.join("remote.git");
        let setup = RealGit { dir: base.clone() };
        setup
            .git(&["init", "--bare", "-q", bare.to_str().unwrap()], "")
            .unwrap();
        // A machine-wide hook may refuse the test identity; the test remote takes none.
        setup
            .git(&["-C", bare.to_str().unwrap(), "config", "core.hooksPath", "/dev/null"], "")
            .unwrap();
        Self {
            url: bare.to_string_lossy().to_string(),
            base,
        }
    }

    fn scratch(&self, name: &str) -> RealGit {
        let dir = self.base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let git = RealGit { dir };
        git.git(&["init", "-q"], "").unwrap();
        // A machine-wide hook may refuse the test identity; the scratch repository takes none.
        git.git(&["config", "core.hooksPath", "/dev/null"], "").unwrap();
        git
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

struct TestClock(AtomicU64);

impl Clock for TestClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn lease_at(device: &str, run: &str, issue: u64, expires: u64) -> Lease {
    Lease {
        device: device.to_string(),
        run: run.to_string(),
        issue,
        expires,
    }
}

fn wins(outcome: Outcome) -> String {
    match outcome { Outcome::Won(sha) => sha, Outcome::Lost(_) => "lost".to_string() }
}

fn held_by(outcome: Outcome) -> Lease {
    match outcome { Outcome::Won(_) => lease_at("won", "won", 0, 0), Outcome::Lost(lease) => lease }
}

/// A git stub that answers one canned reply per call.
struct Scripted {
    replies: Mutex<VecDeque<Result<String, String>>>,
    identity: bool,
}

impl Git for Scripted {
    fn git(&self, args: &[&str], _stdin: &str) -> Result<String, String> {
        if args == ["config", "user.name"] {
            return if self.identity { Ok("owner".to_string()) } else { Err("unset".to_string()) };
        }
        self.replies.lock().unwrap().pop_front().unwrap()
    }
}

fn scripted(replies: Vec<Result<String, String>>) -> Scripted {
    Scripted {
        replies: Mutex::new(replies.into()),
        identity: true,
    }
}

/// Moves the ref with a second git just before the store's first push.
struct MoveOnPush<'a> {
    inner: &'a dyn Git,
    mover: &'a RealGit,
    url: &'a str,
    reference: String,
    move_to: String,
    moved: Cell<bool>,
}

impl Git for MoveOnPush<'_> {
    fn git(&self, args: &[&str], stdin: &str) -> Result<String, String> {
        if args.first() == Some(&"push") && !self.moved.replace(true) {
            self.mover
                .git(
                    &[
                        "push",
                        "--force",
                        self.url,
                        &format!("{}:{}", self.move_to, self.reference),
                    ],
                    "",
                )?;
        }
        self.inner.git(args, stdin)
    }
}

fn push_message(git: &RealGit, url: &str, reference: &str, message: &str) {
    let clock = FixedClock(0);
    let store = Store { git, clock: &clock, url: url.to_string() };
    let sha = store.commit_message(message, None).unwrap();
    git.git(&["push", url, &format!("{sha}:{reference}")], "")
        .unwrap();
}

#[test]
fn acquire_wins_then_another_device_loses_to_the_holder() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_700_000_000);
    let first = fixture.scratch("first");
    let store = Store {
        git: &first,
        clock: &clock,
        url: fixture.url.clone(),
    };
    assert_eq!(refname("dev-a"), "refs/claims/dev-a");
    let sha = wins(store.acquire("dev-a", "dev-1", "run-1", 42, 60).unwrap());
    assert_eq!(sha.len(), 40);
    let (current_sha, lease) = store.current("dev-a").unwrap().unwrap();
    assert_eq!(current_sha, sha);
    assert_eq!(lease, lease_at("dev-1", "run-1", 42, 1_700_000_060));

    let second = fixture.scratch("second");
    let other = Store {
        git: &second,
        clock: &clock,
        url: fixture.url.clone(),
    };
    let holder = held_by(
        other
            .acquire("dev-a", "dev-2", "run-2", 43, 60)
            .unwrap(),
    );
    assert_eq!(holder.device, "dev-1");
    assert_eq!(holder.run, "run-1");
}

#[test]
fn concurrent_acquires_have_exactly_one_winner() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_700_000_000);
    let barrier = Barrier::new(2);
    let a = fixture.scratch("thread-a");
    let b = fixture.scratch("thread-b");
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            let store = Store {
                git: &a,
                clock: &clock,
                url: fixture.url.clone(),
            };
            store.acquire("race", "dev-a", "run", 9, 60)
        });
        let second = scope.spawn(|| {
            barrier.wait();
            let store = Store {
                git: &b,
                clock: &clock,
                url: fixture.url.clone(),
            };
            store.acquire("race", "dev-b", "run", 9, 60)
        });
        let one = first.join().unwrap();
        let two = second.join().unwrap();
        assert_eq!(
            matches!(one, Ok(Outcome::Won(_))) as usize + matches!(two, Ok(Outcome::Won(_))) as usize,
            1
        );
    });
}

#[test]
fn an_expired_lease_is_taken_over_but_an_active_one_is_not() {
    let fixture = Fixture::new();
    let clock = TestClock(AtomicU64::new(1_000));
    let first = fixture.scratch("first");
    let store = Store {
        git: &first,
        clock: &clock,
        url: fixture.url.clone(),
    };
    wins(store.acquire("dev", "dev-1", "run-1", 1, 60).unwrap());

    let taker = fixture.scratch("taker");
    let later = TestClock(AtomicU64::new(1_060));
    let taking = Store {
        git: &taker,
        clock: &later,
        url: fixture.url.clone(),
    };
    wins(taking.acquire("dev", "dev-2", "run-2", 2, 60).unwrap());
    assert_eq!(store.current("dev").unwrap().unwrap().1.device, "dev-2");

    let third = fixture.scratch("third");
    let active = TestClock(AtomicU64::new(1_099));
    let other = Store {
        git: &third,
        clock: &active,
        url: fixture.url.clone(),
    };
    let holder = held_by(other.acquire("dev", "dev-3", "run-3", 3, 60).unwrap());
    assert_eq!(holder.device, "dev-2");
}

#[test]
fn a_takeover_loses_when_the_ref_moved_before_the_push() {
    let fixture = Fixture::new();
    let clock = TestClock(AtomicU64::new(1_000));
    let holder_git = fixture.scratch("holder");
    let store = Store {
        git: &holder_git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    wins(store.acquire("dev", "dev-1", "run-1", 1, 60).unwrap());
    let (old_sha, _) = store.current("dev").unwrap().unwrap();

    let mover = fixture.scratch("mover");
    mover
        .git(&["fetch", &fixture.url, &refname("dev")], "")
        .unwrap();
    let mover_store = Store {
        git: &mover,
        clock: &clock,
        url: fixture.url.clone(),
    };
    let moved_sha = mover_store
        .commit(&lease_at("dev-3", "run-3", 3, 1_060), Some(&old_sha))
        .unwrap();

    let later = TestClock(AtomicU64::new(1_100));
    let wrapper = MoveOnPush {
        inner: &holder_git,
        mover: &mover,
        url: &fixture.url,
        reference: refname("dev"),
        move_to: moved_sha.clone(),
        moved: Cell::new(false),
    };
    let racing = Store {
        git: &wrapper,
        clock: &later,
        url: fixture.url.clone(),
    };
    let holder = held_by(
        racing
            .acquire("dev", "dev-2", "run-2", 2, 60)
            .unwrap(),
    );
    assert_eq!(holder.device, "dev-3");
    assert_eq!(store.current("dev").unwrap().unwrap().0, moved_sha);
}

#[test]
fn renew_extends_the_lease_and_keeps_the_issue() {
    let fixture = Fixture::new();
    let clock = TestClock(AtomicU64::new(1_000));
    let git = fixture.scratch("first");
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    let sha = wins(store.acquire("dev", "dev-1", "run-1", 77, 60).unwrap());
    let renewed = store.renew("dev", "dev-1", "run-1", 120).unwrap();
    assert_ne!(renewed, sha);
    let (current_sha, lease) = store.current("dev").unwrap().unwrap();
    assert_eq!(current_sha, renewed);
    assert_eq!(lease, lease_at("dev-1", "run-1", 77, 1_120));
}

#[test]
fn renew_by_another_device_or_run_fails() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let git = fixture.scratch("first");
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    wins(store.acquire("dev", "dev-1", "run-1", 1, 60).unwrap());
    let refusal = Err("not held by this device and run".to_string());
    assert_eq!(store.renew("dev", "dev-2", "run-1", 60), refusal);
    assert_eq!(store.renew("dev", "dev-1", "run-2", 60), refusal);
}

#[test]
fn renew_when_there_is_no_claim_fails() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let git = fixture.scratch("first");
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    assert_eq!(
        store.renew("dev", "dev-1", "run-1", 60),
        Err("no claim".to_string())
    );
}

#[test]
fn renew_fails_when_the_ref_changed_before_the_push() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let git = fixture.scratch("holder");
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    wins(store.acquire("dev", "dev-1", "run-1", 77, 60).unwrap());
    let (old_sha, _) = store.current("dev").unwrap().unwrap();

    let mover = fixture.scratch("mover");
    mover
        .git(&["fetch", &fixture.url, &refname("dev")], "")
        .unwrap();
    let mover_store = Store {
        git: &mover,
        clock: &clock,
        url: fixture.url.clone(),
    };
    let moved_sha = mover_store
        .commit(&lease_at("dev-3", "run-1", 77, 2_000), Some(&old_sha))
        .unwrap();
    let wrapper = MoveOnPush {
        inner: &git,
        mover: &mover,
        url: &fixture.url,
        reference: refname("dev"),
        move_to: moved_sha.clone(),
        moved: Cell::new(false),
    };
    let racing = Store {
        git: &wrapper,
        clock: &clock,
        url: fixture.url.clone(),
    };
    assert_eq!(
        racing.renew("dev", "dev-1", "run-1", 120),
        Err("claim dev changed while renewing".to_string())
    );
    assert_eq!(store.current("dev").unwrap().unwrap().0, moved_sha);
}

#[test]
fn release_deletes_the_ref_and_a_second_release_fails() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let git = fixture.scratch("first");
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    wins(store.acquire("dev", "dev-1", "run-1", 1, 60).unwrap());
    let lease = store.release("dev").unwrap();
    assert_eq!(lease, lease_at("dev-1", "run-1", 1, 1_060));
    assert!(store.current("dev").unwrap().is_none());
    assert_eq!(store.release("dev"), Err("no claim".to_string()));
}

#[test]
fn list_returns_every_claim_sorted_by_key() {
    let fixture = Fixture::new();
    let clock = FixedClock(1_000);
    let git = fixture.scratch("first");
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    wins(store.acquire("zeta", "dev-z", "run-z", 1, 60).unwrap());
    wins(store.acquire("alpha", "dev-a", "run-a", 2, 60).unwrap());
    let found = store.list().unwrap();
    assert_eq!(
        found.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
    assert_eq!(found[0].1.device, "dev-a");
    assert_eq!(found[1].1.issue, 1);
}

#[test]
fn a_missing_remote_url_is_an_error() {
    let fixture = Fixture::new();
    let git = fixture.scratch("first");
    let clock = FixedClock(1_000);
    let missing = fixture.base.join("absent.git").to_string_lossy().to_string();
    let store = Store {
        git: &git,
        clock: &clock,
        url: missing,
    };
    assert!(store.acquire("dev", "dev-1", "run-1", 1, 60).is_err());
}

#[test]
fn a_malformed_lease_message_is_an_error() {
    let fixture = Fixture::new();
    let git = fixture.scratch("first");
    push_message(&git, &fixture.url, &refname("bad"), "not a lease\n");
    let clock = FixedClock(1_000);
    let store = Store {
        git: &git,
        clock: &clock,
        url: fixture.url.clone(),
    };
    assert_eq!(store.current("bad"), Err("missing device".to_string()));
}

#[test]
fn a_git_stub_covers_the_remaining_error_paths() {
    let clock = FixedClock(0);
    let url = "stub-remote".to_string();

    let git = scripted(vec![Err("ls-remote broke".to_string())]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert_eq!(
        store.acquire("dev", "dev-1", "run-1", 1, 60),
        Err("ls-remote broke".to_string())
    );

    let git = scripted(vec![Ok(String::new()), Err("mktree broke".to_string())]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert_eq!(
        store.acquire("dev", "dev-1", "run-1", 1, 60),
        Err("mktree broke".to_string())
    );

    let git = scripted(vec![
        Ok(String::new()),
        Ok("tree".to_string()),
        Err("commit-tree broke".to_string()),
    ]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert_eq!(
        store.acquire("dev", "dev-1", "run-1", 1, 60),
        Err("commit-tree broke".to_string())
    );

    let git = scripted(vec![
        Ok(String::new()),
        Ok("tree".to_string()),
        Ok(SHA.to_string()),
        Err("push broke".to_string()),
        Ok(String::new()),
    ]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert_eq!(
        store.acquire("dev", "dev-1", "run-1", 1, 60),
        Err("claim dev vanished after a failed push".to_string())
    );

    let git = scripted(vec![
        Ok(String::new()),
        Ok("tree".to_string()),
        Ok(SHA.to_string()),
        Err("push broke".to_string()),
        Err("ls-remote broke".to_string()),
    ]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert_eq!(
        store.acquire("dev", "dev-1", "run-1", 1, 60),
        Err("ls-remote broke".to_string())
    );

    let git = scripted(vec![Ok("   ".to_string())]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert!(store.current("dev").is_err());

    let git = scripted(vec![
        Ok(format!("{SHA}\t{}", refname("dev"))),
        Err("fetch broke".to_string()),
    ]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert_eq!(store.current("dev"), Err("fetch broke".to_string()));

    let git = scripted(vec![
        Ok(format!("{SHA}\t{}", refname("dev"))),
        Ok(String::new()),
        Err("log broke".to_string()),
    ]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert_eq!(store.current("dev"), Err("log broke".to_string()));

    let git = scripted(vec![Ok("garbage".to_string())]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert!(store.list().is_err());

    let git = scripted(vec![Ok(format!("{SHA}\trefs/heads/main\n"))]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert!(store.list().is_err());

    let git = scripted(vec![
        Ok(format!("{SHA}\t{}\n", refname("dev"))),
        Ok(String::new()),
    ]);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert!(store.list().unwrap().is_empty());

    let message = lease_at("dev-1", "run-1", 1, 60).message();
    let listing: Result<String, String> = Ok(format!("{SHA}\t{}\n", refname("dev")));
    let shown = || {
        vec![
            listing.clone(),
            Ok(String::new()),
            Ok(message.clone()),
        ]
    };

    let mut replies = shown();
    replies.push(Ok("tree".to_string()));
    replies.push(Ok(SHA.to_string()));
    replies.push(Err("push broke".to_string()));
    replies.extend(shown());
    let git = scripted(replies);
    let store = Store {
        git: &git,
        clock: &clock,
        url: url.clone(),
    };
    assert_eq!(
        store.renew("dev", "dev-1", "run-1", 60),
        Err("claim dev changed while renewing".to_string())
    );

    let mut replies = shown();
    replies.push(Err("push broke".to_string()));
    replies.extend(shown());
    let git = scripted(replies);
    let store = Store {
        git: &git,
        clock: &clock,
        url,
    };
    assert_eq!(
        store.release("dev"),
        Err("claim dev changed while releasing".to_string())
    );
}

#[test]
fn a_machine_without_a_git_identity_gets_the_fallback_identity() {
    let git = Scripted {
        replies: Mutex::new(
            vec![Ok(String::new()), Ok("tree".to_string()), Ok(SHA.to_string()), Ok(String::new())].into(),
        ),
        identity: false,
    };
    let clock = FixedClock(0);
    let store = Store { git: &git, clock: &clock, url: "stub-remote".to_string() };
    assert_eq!(store.acquire("dev", "dev-1", "run-1", 1, 60), Ok(Outcome::Won(SHA.to_string())));
}
