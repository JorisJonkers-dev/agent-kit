//! The six commands. Each returns the JSON object it prints, or the failure.

use crate::assignments::{apply_assignments, landed_assignments, parse_assignments};
use crate::board::Shown;
use crate::json::{numbers, object, Object, Value};
use crate::ports::Runtime;
use crate::seed::{parse_milestones, seed_state, sync_state};
use crate::spec::{state_base, state_location, ResolvedSpec, TicketFiles};
use crate::specs::{local_spec_place, locate_spec, LocatedSpec};
use crate::state::{parse_state, BuildTrackerState, Failure, SpecReference, Status};
use crate::state_file::{load_state, read_state_file, write_page, write_state};
use crate::text::resolve;

fn parse_object(raw: &Object) -> Result<BuildTrackerState, Failure> {
    parse_state(raw)
}

/// What every command that takes `--spec` prints about where the spec's files are: page, spec, state.
fn spec_paths(located: &LocatedSpec) -> Vec<(&'static str, Value)> {
    vec![("page", Value::str(&located.location.page)), ("spec", Value::str(&located.spec)), ("state", Value::str(&located.location.state))]
}

fn counts(counts: &[(Shown, usize)]) -> Value {
    object(counts.iter().map(|(shown, count)| (shown.name(), Value::count(*count))).collect())
}

fn with_paths(located: &LocatedSpec, leading: Vec<(&'static str, Value)>, trailing: Vec<(&'static str, Value)>) -> Value {
    object(leading.into_iter().chain(spec_paths(located)).chain(trailing).collect())
}

/// Writes a new state for the spec from its tickets on the issue tracker. It refuses a spec that has a state already,
/// naming sync, and one whose state file it cannot read, saying to delete it first.
pub fn init(runtime: &Runtime, spec: &str, milestones: &[String]) -> Result<Value, Failure> {
    let located = locate_spec(spec, runtime)?;
    let LocatedSpec { location, source, spec } = &located;
    if read_state_file(runtime.files.as_ref(), &location.state)?.is_some() {
        return Err(Failure::new("state-exists", format!("a state for {spec} already exists at {}; bring it up to date with sync", location.state)));
    }
    let snapshot = source.snapshot(runtime)?;
    let parsed = parse_milestones(milestones, &snapshot)?;
    write_state(runtime.files.as_ref(), &location.state, &seed_state(&source.spec, &snapshot, &parsed))?;
    let names = Value::Arr(parsed.iter().map(|milestone| Value::str(&milestone.name)).collect());
    let trailing = vec![("status", Value::str("initialized")), ("tickets", Value::count(snapshot.tickets.len()))];
    Ok(with_paths(&located, vec![("milestones", names)], trailing))
}

/// Brings the spec's state up to date with its tickets on the issue tracker, keeping what was set.
pub fn sync(runtime: &Runtime, spec: &str) -> Result<Value, Failure> {
    let located = locate_spec(spec, runtime)?;
    let loaded = load_state(runtime.files.as_ref(), &located.location.state)?;
    let outcome = sync_state(&loaded.raw, &located.source.snapshot(runtime)?);
    let synced = parse_object(&outcome.raw)?;
    write_state(runtime.files.as_ref(), &located.location.state, &outcome.raw)?;
    let leading = vec![("added", numbers(&outcome.added)), ("merged", numbers(&outcome.merged))];
    let trailing = vec![("status", Value::str("synced")), ("tickets", Value::count(synced.tickets.len()))];
    Ok(with_paths(&located, leading, trailing))
}

/// Says where the spec's state file and page live, whether or not they exist yet.
pub fn path(runtime: &Runtime, spec: &str) -> Result<Value, Failure> {
    let located = locate_spec(spec, runtime)?;
    Ok(with_paths(&located, Vec::new(), vec![("status", Value::str("path"))]))
}

/// Brings the spec's page up to date in one step, as /flow runs it: init (no milestones) or sync, then each landed
/// ticket merged, then the page.
pub fn refresh(runtime: &Runtime, spec: &str, landed: &[String]) -> Result<Value, Failure> {
    let located = locate_spec(spec, runtime)?;
    let before = read_state_file(runtime.files.as_ref(), &located.location.state)?;
    let snapshot = located.source.snapshot(runtime)?;
    let (added, raw) = match &before {
        None => (snapshot.tickets.iter().map(|ticket| ticket.number).collect(), seed_state(&located.source.spec, &snapshot, &[])),
        Some(before) => {
            let outcome = sync_state(&before.raw, &snapshot);
            (outcome.added, outcome.raw)
        }
    };
    let raw = apply_assignments(&raw, &landed_assignments(landed, &parse_object(&raw)?)?);
    let state = parse_object(&raw)?;
    let was_merged: Vec<u64> = before.iter().flat_map(|before| before.state.tickets.iter()).filter(|ticket| ticket.status == Status::Merged).map(|ticket| ticket.number).collect();
    // The page first: when it cannot be written, the state stays as it was.
    let shown = write_page(runtime, &located.location.page, &state, located.source.files.as_ref())?;
    write_state(runtime.files.as_ref(), &located.location.state, &raw)?;
    let merged: Vec<u64> = state.tickets.iter().filter(|ticket| ticket.status == Status::Merged && !was_merged.contains(&ticket.number)).map(|ticket| ticket.number).collect();
    let leading = vec![("added", numbers(&added)), ("counts", counts(&shown)), ("merged", numbers(&merged))];
    let trailing = vec![
        ("ran", Value::str(if before.is_none() { "init" } else { "sync" })),
        ("status", Value::str("refreshed")),
        ("tickets", Value::count(state.tickets.len())),
    ];
    Ok(with_paths(&located, leading, trailing))
}

/// Writes the build tracker page for a state file.
pub fn render(runtime: &Runtime, state_path: &str, out: &str) -> Result<Value, Failure> {
    let loaded = load_state(runtime.files.as_ref(), state_path)?;
    let files = ticket_files(runtime, state_path, &loaded.state)?;
    let shown = write_page(runtime, out, &loaded.state, files.as_ref())?;
    Ok(object(vec![("counts", counts(&shown)), ("out", Value::str(out)), ("status", Value::str("rendered"))]))
}

/// Where a local spec's ticket files are shown, read from the repository the command runs in. Only the spec's own
/// repository will do: its state file is the one that repository's spec names, so a state rendered from anywhere else
/// gets no links rather than links into the wrong tree.
fn ticket_files(runtime: &Runtime, state_path: &str, state: &BuildTrackerState) -> Result<Option<TicketFiles>, Failure> {
    let SpecReference::Local { path } = &state.spec else {
        return Ok(None);
    };
    let location = runtime.repositories.locate(&runtime.cwd).map_err(|message| Failure::new("unexpected", message))?;
    let (Some(base), Some(location)) = (state_base(&runtime.env), location) else {
        return Ok(None);
    };
    let (repository, files) = local_spec_place(&location);
    let own = state_location(&base, &ResolvedSpec::Local { path: path.clone(), repository }).state;
    Ok((own == resolve(&runtime.cwd, state_path)).then_some(files))
}

/// Changes the named tickets' statuses in the state file, every one or none.
pub fn set(runtime: &Runtime, state_path: &str, assignments: &[String]) -> Result<Value, Failure> {
    let loaded = load_state(runtime.files.as_ref(), state_path)?;
    let parsed = parse_assignments(assignments, &loaded.state)?;
    write_state(runtime.files.as_ref(), state_path, &apply_assignments(&loaded.raw, &parsed))?;
    let changed = parsed.iter().map(|assignment| object(vec![("status", Value::str(assignment.status.name())), ("ticket", Value::Num(assignment.ticket as f64))]));
    Ok(object(vec![("changed", Value::Arr(changed.collect())), ("status", Value::str("set"))]))
}
