//! The per-tier concurrency limit: a tier takes only so many runs at once, and the rest wait. The limit is an OS lock
//! on a slot file, so it holds across processes and is released when its holder dies.

use super::machine::Machine;
use crate::tier_slots::{acquire, SlotPlan};
use std::sync::atomic::{AtomicUsize, Ordering};

fn plan(limit: usize, wait_s: f64) -> SlotPlan {
    SlotPlan { tier: "tier-go-cheap".into(), limit, wait_s }
}

fn machine() -> Machine {
    Machine::new("{}")
}

#[test]
fn a_tier_takes_its_limit_of_runs_and_refuses_the_next_once_the_wait_is_over() {
    let machine = machine();
    let first = acquire(&machine.runtime, &plan(2, 0.0)).expect("a first slot");
    let second = acquire(&machine.runtime, &plan(2, 0.0)).expect("a second slot");
    let refused = acquire(&machine.runtime, &plan(2, 0.0)).err().expect("no third slot");
    assert_eq!(refused.code, "tier-busy");
    assert!(refused.message.starts_with("tier-go-cheap already runs 2 at once, and none was free for 0 s"), "{}", refused.message);
    // A slot given back is free again, and another tier's slots are its own.
    drop(first);
    let third = acquire(&machine.runtime, &plan(2, 0.0)).expect("the slot that was given back");
    let other = acquire(&machine.runtime, &SlotPlan { tier: "tier-free".into(), ..plan(1, 0.0) }).expect("another tier");
    drop((second, third, other));
    assert!(acquire(&machine.runtime, &plan(1, 0.0)).is_ok(), "every slot was given back");
}

#[test]
fn a_run_that_waits_gets_the_slot_the_moment_it_is_given_back() {
    let machine = machine();
    let held = acquire(&machine.runtime, &plan(1, 0.0)).expect("the slot");
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| acquire(&machine.runtime, &plan(1, 30.0)).map(|_| ()).map_err(|error| error.code));
        std::thread::sleep(std::time::Duration::from_millis(150));
        drop(held);
        assert_eq!(waiter.join().expect("the waiter"), Ok(()));
    });
}

#[test]
fn many_runs_at_once_never_hold_more_slots_than_the_limit() {
    let machine = machine();
    let (running, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let slot = acquire(&machine.runtime, &plan(3, 60.0)).expect("a slot in the end");
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(40));
                running.fetch_sub(1, Ordering::SeqCst);
                drop(slot);
            });
        }
    });
    assert_eq!((most.load(Ordering::SeqCst), running.load(Ordering::SeqCst)), (3, 0));
}

#[test]
fn a_slot_is_never_taken_over_however_long_its_holder_runs_and_a_late_release_frees_only_its_own() {
    // A slot file that is days old is still held: there is no staleness, only the lock.
    let machine = machine();
    let held = acquire(&machine.runtime, &plan(1, 0.0)).expect("the slot");
    let path = format!("{}/state/agent-kit/model-gateway/tier-slots/tier-go-cheap.0", machine.sandbox.text());
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 24 * 3600);
    std::fs::File::options().write(true).open(&path).expect("the slot file").set_modified(old).expect("an old mtime");
    assert_eq!(acquire(&machine.runtime, &plan(1, 0.0)).err().map(|error| error.code), Some("tier-busy".to_string()), "an old slot is still held");
    // Dropping a slot that was refused (or any other) does not remove the file a holder is using.
    drop(acquire(&machine.runtime, &plan(1, 0.0)));
    assert!(std::path::Path::new(&path).exists());
    drop(held);
    assert!(acquire(&machine.runtime, &plan(1, 0.0)).is_ok());
}

/// Run as a child process by `a_slot_a_killed_run_held_is_free_again`: takes the slot, says so, and holds it.
#[test]
fn slot_holder_child() {
    let Ok(home) = std::env::var("SLOT_HOLDER_HOME") else { return };
    let mut machine = machine();
    machine.runtime.env.insert("XDG_STATE_HOME".into(), format!("{home}/state"));
    let _slot = acquire(&machine.runtime, &plan(1, 0.0)).expect("the child takes the slot");
    std::fs::write(format!("{home}/ready"), "").expect("ready");
    std::thread::sleep(std::time::Duration::from_secs(120));
}

#[test]
fn a_slot_a_killed_run_held_is_free_again_and_one_a_live_process_holds_is_not() {
    let home = super::scenarios::Sandbox::new().text();
    let mut machine = machine();
    machine.runtime.env.insert("XDG_STATE_HOME".into(), format!("{home}/state"));
    let mut child = std::process::Command::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", "tests::tier_slots::slot_holder_child", "--nocapture"])
        .env("SLOT_HOLDER_HOME", &home)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a child process");
    let ready = format!("{home}/ready");
    let started = std::time::Instant::now();
    while !std::path::Path::new(&ready).exists() {
        assert!(started.elapsed().as_secs() < 60, "the child never took its slot");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(acquire(&machine.runtime, &plan(1, 0.0)).err().map(|error| error.code), Some("tier-busy".to_string()), "another process holds it");
    child.kill().expect("killed");
    child.wait().expect("reaped");
    assert!(acquire(&machine.runtime, &plan(1, 5.0)).is_ok(), "the dead process's slot is free at once");
}

#[test]
fn a_machine_with_no_state_directory_gives_no_slot() {
    let mut machine = machine();
    machine.runtime.env.remove("HOME");
    machine.runtime.env.remove("XDG_STATE_HOME");
    assert_eq!(acquire(&machine.runtime, &plan(1, 0.0)).err().map(|error| error.code), Some("unexpected".to_string()));
}

#[test]
fn a_slot_file_that_cannot_be_opened_is_an_error() {
    let machine = machine();
    let dir = format!("{}/state/agent-kit/model-gateway/tier-slots", machine.sandbox.text());
    std::fs::create_dir_all(format!("{dir}/tier-go-cheap.0")).expect("a directory where the file would be");
    let refused = acquire(&machine.runtime, &plan(1, 0.0)).err().expect("refused");
    assert_eq!(refused.code, "unexpected");
}
