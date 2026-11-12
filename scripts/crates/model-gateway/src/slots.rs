//! How many attempts may run on one client at once. A batch shares one of these
//! between its delegations, and an attempt takes a slot of the client it is
//! about to run on, which after a fall-through is not the client its delegation
//! was planned for. An attempt that finds the client full waits, in the order
//! it asked, and gives its slot back when it ends.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Condvar, Mutex};

/// The slots of every client, and the attempts waiting for one of each.
pub struct ClientSlots {
    limit: usize,
    state: Mutex<Slots>,
    condvar: Condvar,
}

#[derive(Default)]
struct Slots {
    running: BTreeMap<String, usize>,
    /// The tickets waiting for a client, oldest first: the order they asked in.
    waiting: BTreeMap<String, VecDeque<u64>>,
    next: u64,
}

/// A slot an attempt holds until it ends: dropping it gives the slot back.
pub struct SlotGuard<'a> {
    slots: &'a ClientSlots,
    client: String,
}

impl ClientSlots {
    pub fn new(limit: usize) -> ClientSlots {
        ClientSlots { limit, state: Mutex::new(Slots::default()), condvar: Condvar::new() }
    }

    /// Waits until the client has a slot free, in the order the attempts asked, and takes it.
    pub fn acquire(&self, client: &str) -> SlotGuard<'_> {
        let mut state = self.state.lock().expect("the client slots");
        let ticket = state.next;
        state.next += 1;
        state.waiting.entry(client.to_string()).or_default().push_back(ticket);
        loop {
            let first = state.waiting.get(client).and_then(|queue| queue.front().copied()) == Some(ticket);
            let running = state.running.get(client).copied().unwrap_or(0);
            if first && running < self.limit {
                state.waiting.get_mut(client).expect("the queue it just joined").pop_front();
                *state.running.entry(client.to_string()).or_insert(0) += 1;
                return SlotGuard { slots: self, client: client.to_string() };
            }
            state = self.condvar.wait(state).expect("the client slots");
        }
    }

    /// Gives a slot back, and wakes the attempts waiting for that client.
    fn release(&self, client: &str) {
        let mut state = self.state.lock().expect("the client slots");
        let running = state.running.entry(client.to_string()).or_insert(0);
        *running = running.saturating_sub(1);
        drop(state);
        self.condvar.notify_all();
    }
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        self.slots.release(&self.client);
    }
}
