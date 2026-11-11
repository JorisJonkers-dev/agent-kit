//! What the page shows for each ticket, and how the tickets group into sections.

use crate::state::{BuildTrackerState, Status, Ticket};
use std::collections::{HashMap, HashSet};

/// What the page shows for a ticket. A `todo` ticket is Ready once every blocker is merged and no blocker outside the
/// spec is known to be open, and Blocked before that; every other status shows as itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Merged,
    Review,
    Running,
    Ready,
    Owner,
    Blocked,
}

/// Every shown status in the summary strip's order.
pub const SHOWN_ORDER: [Shown; 6] = [Shown::Merged, Shown::Review, Shown::Running, Shown::Ready, Shown::Owner, Shown::Blocked];

impl Shown {
    /// The class and key the status goes by.
    pub fn name(self) -> &'static str {
        match self {
            Shown::Merged => "merged",
            Shown::Review => "review",
            Shown::Running => "running",
            Shown::Ready => "ready",
            Shown::Owner => "owner",
            Shown::Blocked => "blocked",
        }
    }

    /// The label the status carries on the page.
    pub fn label(self) -> &'static str {
        match self {
            Shown::Merged => "Merged",
            Shown::Review => "Merging",
            Shown::Running => "Building",
            Shown::Ready => "Ready",
            Shown::Owner => "Needs you",
            Shown::Blocked => "Blocked",
        }
    }
}

pub struct BoardTicket<'a> {
    pub ticket: &'a Ticket,
    pub shown: Shown,
}

pub struct BoardSection<'a> {
    pub name: String,
    pub tickets: Vec<&'a BoardTicket<'a>>,
    pub merged: usize,
}

pub struct Board<'a> {
    pub tickets: Vec<BoardTicket<'a>>,
    /// A count per shown status, for the statuses at least one ticket shows.
    pub counts: Vec<(Shown, usize)>,
}

pub const OTHER_SECTION: &str = "Other";
pub const SINGLE_SECTION: &str = "Tickets";

pub fn build_board(state: &BuildTrackerState) -> Board<'_> {
    let merged: HashSet<u64> = state.tickets.iter().filter(|ticket| ticket.status == Status::Merged).map(|ticket| ticket.number).collect();
    let tickets: Vec<BoardTicket> = state.tickets.iter().map(|ticket| BoardTicket { shown: shown_status(ticket, &merged), ticket }).collect();
    let counts = SHOWN_ORDER
        .iter()
        .map(|shown| (*shown, tickets.iter().filter(|ticket| ticket.shown == *shown).count()))
        .filter(|(_, count)| *count > 0)
        .collect();
    Board { counts, tickets }
}

fn shown_status(ticket: &Ticket, merged: &HashSet<u64>) -> Shown {
    match ticket.status {
        Status::Merged => Shown::Merged,
        Status::Review => Shown::Review,
        Status::Running => Shown::Running,
        Status::Owner => Shown::Owner,
        Status::Todo => {
            let waiting = ticket.blocked_by.iter().any(|blocker| !merged.contains(blocker)) || ticket.external.iter().any(|blocker| blocker.state.as_deref() == Some("open"));
            if waiting {
                Shown::Blocked
            } else {
                Shown::Ready
            }
        }
    }
}

/// One section per milestone, in the state's order. A ticket sits in the first milestone whose closing ticket depends on
/// it, directly or through other tickets; the rest go in Other. A state without milestones has one section.
pub fn sections<'a>(state: &BuildTrackerState, tickets: &'a [BoardTicket<'a>]) -> Vec<BoardSection<'a>> {
    if state.milestones.is_empty() {
        return vec![section(SINGLE_SECTION, tickets.iter().collect())];
    }
    let by_number: HashMap<u64, &BoardTicket> = tickets.iter().map(|ticket| (ticket.ticket.number, ticket)).collect();
    let mut milestone_of: HashMap<u64, usize> = HashMap::new();
    for (index, milestone) in state.milestones.iter().enumerate() {
        for number in std::iter::once(milestone.closes).chain(dependencies(milestone.closes, &by_number)) {
            milestone_of.entry(number).or_insert(index);
        }
    }
    let mut result: Vec<BoardSection> = state
        .milestones
        .iter()
        .enumerate()
        .map(|(index, milestone)| section(&milestone.name, tickets.iter().filter(|ticket| milestone_of.get(&ticket.ticket.number) == Some(&index)).collect()))
        .collect();
    let other: Vec<&BoardTicket> = tickets.iter().filter(|ticket| !milestone_of.contains_key(&ticket.ticket.number)).collect();
    if !other.is_empty() {
        result.push(section(OTHER_SECTION, other));
    }
    result
}

/// Every ticket `number` depends on, directly or through other tickets.
fn dependencies(number: u64, by_number: &HashMap<u64, &BoardTicket>) -> HashSet<u64> {
    let mut seen = HashSet::new();
    let mut pending = vec![number];
    while let Some(next) = pending.pop() {
        for blocker in by_number.get(&next).map(|ticket| ticket.ticket.blocked_by.as_slice()).unwrap_or_default() {
            if seen.insert(*blocker) {
                pending.push(*blocker);
            }
        }
    }
    seen
}

fn section<'a>(name: &str, tickets: Vec<&'a BoardTicket<'a>>) -> BoardSection<'a> {
    BoardSection { merged: tickets.iter().filter(|ticket| ticket.ticket.status == Status::Merged).count(), name: name.to_string(), tickets }
}
