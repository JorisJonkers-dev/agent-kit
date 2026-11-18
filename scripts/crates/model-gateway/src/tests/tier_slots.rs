//! The per-tier concurrency limit: a tier takes only so many runs at once, and the rest wait.

use super::machine::Machine;
use crate::tier_slots::{acquire, SlotPlan};
use std::sync::atomic::{AtomicUsize, Ordering};

fn plan(limit: usize, wait_s: f64) -> SlotPlan {
    SlotPlan { tier: "tier-go-cheap".into(), limit, wait_s, stale_s: 7200.0 }
}

/// The real time, which is when a slot file written in the test is stamped.
fn real_now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("after 1970").as_millis() as i64
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
fn a_slot_a_dead_run_left_behind_is_broken_once_it_is_old_enough() {
    // The clock reads two hours and a minute after the slot file was written: the slot is older than `stale_s`.
    let process = super::machine::FakeProcess::default();
    let machine = Machine::with_process("{}", real_now_ms() + 7260 * 1000, process.clone(), Box::new(process));
    let dir = format!("{}/state/agent-kit/model-gateway/tier-slots", machine.sandbox.text());
    let stale = format!("{dir}/tier-go-cheap.0");
    machine.write_at(&stale, "");
    let _slot = acquire(&machine.runtime, &plan(1, 0.0)).expect("the dead run's slot is taken over");
    // A slot that is not old is not broken.
    let fresh = Machine::new("{}");
    let _held = acquire(&fresh.runtime, &plan(1, 0.0)).expect("a slot");
    assert!(acquire(&fresh.runtime, &plan(1, 0.0)).is_err());
}

#[test]
fn a_machine_with_no_state_directory_gives_no_slot() {
    let mut machine = machine();
    machine.runtime.env.remove("HOME");
    machine.runtime.env.remove("XDG_STATE_HOME");
    assert_eq!(acquire(&machine.runtime, &plan(1, 0.0)).err().map(|error| error.code), Some("unexpected".to_string()));
}
