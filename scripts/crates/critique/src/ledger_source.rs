//! The one ledger critique reads: the gateway's, where every critic run is
//! recorded from now on, together with critique's own ledger, which holds the
//! runs recorded before the move and the flow events that belong to no run.
//! Nothing is migrated or deleted, so a run that both ledgers hold — one
//! recorded while the move was made — counts once, by its run and critic id.

use crate::budget::{invocation_lines, LoggedInvocation};
use crate::error::CritiqueError;
use crate::gateway_ledger::gateway_ledger_lines;
use crate::ledger::{parse_ledger, Ledger, LedgerCriticRun};
use crate::runtime::Runtime;
use std::collections::HashSet;

/// What critique's own ledger and the gateway's hold together: the runs,
/// findings and launches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LedgerRead {
    pub ledger: Ledger,
    pub invocations: Vec<LoggedInvocation>,
    /// The runs critique's own ledger holds a run line for, which is what
    /// `record` writes: the gateway's record says a critic ran, not that the run
    /// was recorded, so it never makes a run look recorded.
    pub recorded_runs: Vec<LedgerCriticRun>,
}

/// The one ledger critique reads, from both files.
pub struct LedgerSource<'a> {
    runtime: &'a Runtime,
}

impl<'a> LedgerSource<'a> {
    pub fn new(runtime: &'a Runtime) -> LedgerSource<'a> {
        LedgerSource { runtime }
    }

    pub fn read(&self) -> Result<LedgerRead, CritiqueError> {
        let own_path = crate::locations::ledger_path(&self.runtime.env)?;
        let own = self.text(&own_path)?;
        let gateway_path = model_gateway::locations::ledger_path(&self.runtime.gateway.env)
            .map_err(|error| CritiqueError::unexpected(error.message))?;
        let gateway = gateway_ledger_lines(&self.text(&gateway_path)?);
        let ledger = if own.is_empty() {
            Ledger::default()
        } else {
            parse_ledger(&own).map_err(|error| error.into_error("invalid-ledger", &own_path))?
        };
        let Ledger { findings, flow_events, runs, skipped_reviews } = ledger;
        let recorded_runs = runs.clone();
        Ok(LedgerRead {
            invocations: merge(invocation_lines(&own), gateway.invocations, |line| {
                format!("{} {} {}", line.run_id.clone().unwrap_or_default(), line.critic_id, line.kind)
            }),
            ledger: Ledger {
                findings,
                flow_events,
                runs: merge(runs, gateway.runs, |run| format!("{} {}", run.context.run_id, run.critic_id)),
                skipped_reviews,
            },
            recorded_runs,
        })
    }

    fn text(&self, path: &str) -> Result<String, CritiqueError> {
        if self.runtime.files.exists(path).unwrap_or(false) {
            self.runtime.files.read_text(path).map_err(CritiqueError::from_fs)
        } else {
            Ok(String::new())
        }
    }
}

/// Critique's own lines, in order, then the gateway's that critique's do not
/// already hold. A launch both ledgers recorded counts once; two launches of the
/// same critic in the same run, which critique's ledger holds as two lines, still
/// count twice.
fn merge<T>(own: Vec<T>, gateway: Vec<T>, key: impl Fn(&T) -> String) -> Vec<T> {
    let seen: HashSet<String> = own.iter().map(|line| key(line)).collect();
    let mut merged = own;
    merged.extend(gateway.into_iter().filter(|line| !seen.contains(&key(line))));
    merged
}
