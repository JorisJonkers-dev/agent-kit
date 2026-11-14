//! `check-tickets`: the deterministic ticket checker. It reads a spec and its tickets through the repository's
//! tracker, the way prepare reads a spec, and reports what a script can judge. It sends nothing to any critic and
//! writes nothing, so it needs no family allowlist entry; a repository whose remotes are refused is still refused.
//! The node runner's `application/check-tickets.ts`.

use crate::error::{CritiqueError, ShapeError};
use crate::fields::Fields;
use crate::repository::{resolve, Resolution};
use crate::runtime::Runtime;
use crate::tickets::{check_json, check_tickets, ticket_check_status, Ticket, TicketCheck};
use crate::tracker_resolver::{resolve_tracker, ArtifactRead, TicketsRead, TrackedArtifact, TrackedTicket, Tracker, TrackerResolution};
use model_gateway::json::{self, Value};

/// A spec and its tickets as the tracker holds them, with what the checker reports about them.
pub struct CheckedTickets {
    pub spec: TrackedArtifact,
    pub tickets: Vec<TrackedTicket>,
    pub check: TicketCheck,
}

/// What reading the tickets came to: the checked tickets, or the refusal the tracker gave.
pub enum CheckedRead {
    Read(CheckedTickets),
    Refused(Value),
}

impl CheckedTickets {
    /// The check as `check-tickets` prints it: `failed` while the tickets hold a dependency cycle, and the spec it read.
    pub fn result(&self) -> Value {
        self.result_with(Vec::new())
    }

    /// [`Self::result`] with `extra` fields after the check's own; a field the check already has is replaced in place.
    pub fn result_with(&self, extra: Vec<(&str, Value)>) -> Value {
        let spec = json::object(vec![("reference", Value::str(&self.spec.reference)), ("tracker", Value::str(self.spec.tracker))]);
        let mut pairs = vec![("status", Value::str(ticket_check_status(&self.check))), ("spec", spec)];
        pairs.extend(check_json(&self.check));
        pairs.extend(extra);
        json::object(pairs)
    }
}

/// Reads a spec and its tickets through the repository's tracker and checks them. Both the check-tickets command and
/// the tickets stage's prepare read them this way, so the packet's report is what check-tickets prints.
pub fn read_checked_tickets(runtime: &Runtime, tracker: &Tracker<'_>, spec: &str, comments: bool) -> Result<CheckedRead, CritiqueError> {
    let spec_read = match tracker.artifact(runtime, spec, comments)? {
        ArtifactRead::Refused(refusal) => return Ok(CheckedRead::Refused(refusal)),
        ArtifactRead::Read(read) => read,
    };
    let tickets = match tracker.tickets(runtime, spec)? {
        TicketsRead::Refused(refusal) => return Ok(CheckedRead::Refused(refusal)),
        TicketsRead::Read(tickets) => tickets,
    };
    let plain: Vec<Ticket> = tickets.iter().map(|tracked| tracked.ticket.clone()).collect();
    let check = check_tickets(&spec_read.text, &plain);
    Ok(CheckedRead::Read(CheckedTickets { check, spec: spec_read, tickets }))
}

/// Runs `check-tickets` on its input: `repo` and `spec`.
pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let shape = |error: ShapeError| error.into_error("invalid-input", "check-tickets input");
    let fields = Fields::of(raw, "").map_err(shape)?;
    let repo = fields.string("repo").map_err(shape)?;
    let spec = fields.string("spec").map_err(shape)?;
    let repository = match resolve(runtime, &repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(repository) => repository,
    };
    let tracker = match resolve_tracker(runtime, &repository)? {
        TrackerResolution::Refused(refusal) => return Ok(refusal),
        TrackerResolution::Resolved(tracker) => tracker,
    };
    Ok(match read_checked_tickets(runtime, &tracker, &spec, false)? {
        CheckedRead::Refused(refusal) => refusal,
        CheckedRead::Read(checked) => checked.result(),
    })
}
