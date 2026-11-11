//! Reading and writing the state file and the page.

use crate::board::Shown;
use crate::json::{parse, pretty, Object, Value};
use crate::page::render_page;
use crate::ports::{Files, Runtime};
use crate::spec::TicketFiles;
use crate::state::{parse_state, BuildTrackerState, Failure};

/// A state file as read: its parsed JSON, kept whole for writing back, and the state it holds.
pub struct LoadedState {
    pub raw: Object,
    pub state: BuildTrackerState,
}

fn unreadable(path: &str, reason: &str) -> Failure {
    Failure::new("unreadable-state", format!("the state file at {path} is unreadable: {reason}; delete it and run init again"))
}

/// The state file read and checked, or `None` when there is none. Any other failure is `unreadable-state`, which names
/// the file and says to delete it and run init again.
pub fn read_state_file(files: &dyn Files, path: &str) -> Result<Option<LoadedState>, Failure> {
    let text = match files.read_text(path) {
        Ok(text) => text,
        Err(error) if error.code == "ENOENT" => return Ok(None),
        Err(error) => return Err(unreadable(path, &error.message)),
    };
    let raw = parse(&text).map_err(|error| unreadable(path, &format!("it is not JSON (SyntaxError: {error})")))?;
    let state = parse_state(&raw).map_err(|error| unreadable(path, &error.message))?;
    Ok(Some(LoadedState { raw: raw.as_object().cloned().unwrap_or_default(), state }))
}

/// The state file read and checked; a missing one is `missing-state`, which names init.
pub fn load_state(files: &dyn Files, path: &str) -> Result<LoadedState, Failure> {
    read_state_file(files, path)?.ok_or_else(|| Failure::new("missing-state", format!("there is no state file at {path}; create it with init")))
}

pub fn write_state(files: &dyn Files, path: &str, raw: &Object) -> Result<(), Failure> {
    files.write_atomic(path, &format!("{}\n", pretty(&Value::Obj(raw.clone())))).map_err(|error| Failure::new("unexpected", error.message))
}

/// Renders the state's page to `path`, stamped with the clock's time, and returns its count per shown status.
pub fn write_page(runtime: &Runtime, path: &str, state: &BuildTrackerState, ticket_files: Option<&TicketFiles>) -> Result<Vec<(Shown, usize)>, Failure> {
    let page = render_page(state, &stamp(runtime.clock.now()), ticket_files);
    runtime.files.write_atomic(path, &page.html).map_err(|error| Failure::new("unexpected", error.message))?;
    Ok(page.counts)
}

/// The page's "updated" time, in UTC to the minute: `2026-11-03 12:00 UTC`.
pub fn stamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let minutes = seconds.rem_euclid(86_400) / 60;
    // Days since 1970-01-01 to a civil date (proleptic Gregorian).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC", minutes / 60, minutes % 60)
}
