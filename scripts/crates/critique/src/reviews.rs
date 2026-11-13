//! `reviews`: the review entry of `/flow`, a repository's recent review runs, each a flow of its own, with its stage,
//! run id, verdict counts and spend. It reads the run directories and the ledgers and writes nothing; it never says a
//! review is due, since a review runs only when someone asks for one. It sends nothing anywhere, so it needs no family
//! allowlist entry, but a repository whose remotes refuse a critique is refused here too. The node runner's
//! `application/reviews.ts`.

use crate::cold_read::read_manifest;
use crate::error::{CritiqueError, ShapeError};
use crate::fields::Fields;
use crate::ledger_source::LedgerSource;
use crate::locations::{manifest_path, report_path};
use crate::paths::join;
use crate::repository::{resolve, Resolution};
use crate::review_runs::{review_runs, ReviewRunSource, ReviewRunSources};
use crate::runtime::Runtime;
use model_gateway::json::{self, Value};

/// How many runs `/flow` lists unless it asks for another number.
const DEFAULT_LIMIT: f64 = 10.0;

struct Input {
    limit: f64,
    repo: String,
}

fn read_input(raw: &Value) -> Result<Input, ShapeError> {
    let fields = Fields::of(raw, "")?;
    let limit = if fields.has("limit") { fields.positive_integer("limit")? } else { DEFAULT_LIMIT };
    Ok(Input { limit, repo: fields.string("repo")? })
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let input = read_input(raw).map_err(|error| error.into_error("invalid-input", "reviews input"))?;
    let repository = match resolve(runtime, &input.repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(repository) => repository,
    };
    let read = LedgerSource::new(runtime).read()?;
    let ours = |name: &str| name == repository.key;
    let recorded: Vec<_> = read.recorded_runs.iter().filter(|line| ours(&line.context.repository)).cloned().collect();
    let sources = ReviewRunSources {
        directories: directories(runtime, &join(&[&repository.common_dir, "critique"]))?,
        findings: read.ledger.findings.iter().filter(|line| ours(&line.context.repository)).collect(),
        invocations: read.invocations.iter().filter(|line| ours(&line.repository)).collect(),
        ledger_runs: &recorded,
    };
    let runs = review_runs(&sources, input.limit);
    Ok(json::object(vec![("runs", Value::Arr(runs.iter().map(|run| run.to_json()).collect())), ("status", Value::str("read"))]))
}

/// The runs the repository's run directories hold, recorded or not.
fn directories(runtime: &Runtime, critique_dir: &str) -> Result<Vec<ReviewRunSource>, CritiqueError> {
    let mut runs = Vec::new();
    for name in runtime.dirs.list_directories(critique_dir) {
        let run_dir = join(&[critique_dir, &name]);
        if !runtime.files.exists(&manifest_path(&run_dir)).unwrap_or(false) {
            continue;
        }
        let manifest = read_manifest(runtime, &run_dir)?;
        let recorded = runtime.files.exists(&report_path(&run_dir)).unwrap_or(false);
        runs.push(ReviewRunSource { recorded, round: manifest.round, run_id: manifest.run_id, stage: manifest.stage });
    }
    Ok(runs)
}
