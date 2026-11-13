//! `stats`: how each critic client does per stage, read from the ledger and routing. It writes nothing: the
//! suggestions are sentences for the owner, and routing changes only when the owner edits it. A missing ledger is an
//! empty one. The node runner's `application/stats.ts`.

use crate::error::{CritiqueError, ShapeError};
use crate::fields::Fields;
use crate::ledger_source::LedgerSource;
use crate::locations::ledger_path;
use crate::repository::{resolve, Resolution};
use crate::routing::{Routing, StatsThresholds};
use crate::routing_source::load_routing;
use crate::runtime::Runtime;
use crate::stats::{critique_stats, render_stats, StatsFilter, StatsScope};
use model_gateway::json::{object, Object, Value};

struct Input {
    /// A working directory whose repository the figures are limited to.
    repo: Option<String>,
    filter: StatsFilter,
}

fn read_input(raw: &Value) -> Result<Input, ShapeError> {
    let fields = Fields::of(raw, "")?;
    if fields.has("repo") && fields.has("repository") {
        return Err(ShapeError("name the repository by repo or by repository, not both".to_string()));
    }
    let since = fields.optional_matching("since", is_day, "a date, YYYY-MM-DD")?;
    let until = fields.optional_matching("until", is_day, "a date, YYYY-MM-DD")?;
    if let (Some(since), Some(until)) = (&since, &until) {
        if since > until {
            return Err(ShapeError(format!("since {since} is after until {until}")));
        }
    }
    let repository = fields.optional_string("repository")?;
    let repo = fields.optional_string("repo")?;
    Ok(Input { repo, filter: StatsFilter { repository, since, until } })
}

/// Whether a value is a real calendar day, `YYYY-MM-DD`: the shape `/^\d{4}-\d{2}-\d{2}$/` accepts, and a date
/// `new Date(`${value}T00:00:00Z`)` reads back as the same day.
fn is_day(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        let ok = if index == 4 || index == 7 { *byte == b'-' } else { byte.is_ascii_digit() };
        if !ok {
            return false;
        }
    }
    let digits = |from: usize, to: usize| value[from..to].parse::<i64>().unwrap_or(0);
    valid_day(digits(0, 4), digits(5, 7), digits(8, 10))
}

/// Whether the proleptic Gregorian calendar has that day, which is what a date that reads back as itself means.
fn valid_day(year: i64, month: i64, day: i64) -> bool {
    if !(1..=12).contains(&month) || day < 1 {
        return false;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let length = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    day <= length
}

/// The clients a stage's base and adr rosters route to now.
fn routed_clients(routing: &Routing, stage: &str) -> Vec<String> {
    let Some(found) = routing.stages.iter().find(|(name, _)| name == stage).map(|(_, found)| found) else {
        return Vec::new();
    };
    found.base.iter().chain(found.adr.iter()).map(|slot| slot.client.clone()).collect()
}

/// The filter as the node runner prints it: `repository` first, unless the repository was resolved from `repo`, which
/// spreads the filter and then adds the key, so it comes last.
fn filter_json(filter: &StatsFilter, resolved: bool) -> Value {
    let mut object = Object::new();
    let repository = filter.repository.as_ref().map(|repository| Value::str(repository));
    if let (false, Some(repository)) = (resolved, &repository) {
        object.set("repository", repository.clone());
    }
    if let Some(since) = &filter.since {
        object.set("since", Value::str(since));
    }
    if let Some(until) = &filter.until {
        object.set("until", Value::str(until));
    }
    if let (true, Some(repository)) = (resolved, repository) {
        object.set("repository", repository);
    }
    Value::Obj(object)
}

fn thresholds_json(thresholds: &StatsThresholds) -> Value {
    object(vec![
        ("min_adjudicated_findings", Value::Num(thresholds.min_adjudicated_findings)),
        ("removal_features", Value::Num(thresholds.removal_features)),
    ])
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let input = read_input(raw).map_err(|error| error.into_error("invalid-input", "stats input"))?;
    let mut filter = input.filter;
    if let Some(repo) = &input.repo {
        match resolve(runtime, repo)? {
            Resolution::Refused(refusal) => return Ok(refusal),
            Resolution::Resolved(resolved) => filter.repository = Some(resolved.key),
        }
    }
    let routing = load_routing(runtime)?;
    let Some(thresholds) = routing.stats.clone() else {
        return Err(CritiqueError::new("invalid-routing", "routing has no stats thresholds"));
    };
    let path = ledger_path(&runtime.env)?;
    let ledger = LedgerSource::new(runtime).read()?.ledger;
    let stats = critique_stats(&ledger, &filter, &thresholds, &|stage: &str| routed_clients(&routing, stage))?;
    let report = render_stats(&stats, &StatsScope { ledger_path: path.clone(), filter: filter.clone(), thresholds: thresholds.clone() });
    let mut result = stats.to_object();
    result.set("filter", filter_json(&filter, input.repo.is_some()));
    result.set("ledger_path", Value::str(&path));
    result.set("report", Value::str(&report));
    result.set("status", Value::str("reported"));
    result.set("thresholds", thresholds_json(&thresholds));
    Ok(Value::Obj(result))
}
