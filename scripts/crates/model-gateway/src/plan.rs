//! What the gateway says about one client's plan limit, in its own words: each
//! window of the plan with what the ledger says is spent in it, the warning for
//! every window at or above the warning ratio, and why the client cannot start
//! at all. A client the machine file gives no plan limit is unwatched: no
//! windows, no warnings, and it can start.

use crate::caps::run_records;
use crate::config_parse::load_config;
use crate::error::GatewayError;
use crate::ledger_file::LedgerStore;
use crate::ports::Runtime;
use crate::usage::{describe_window, plan_windows, warnings_of, PlanState, PlanWindowUsage};

/// One client's plan limit as a caller that runs the client itself reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanLimitReading {
    pub client: String,
    pub windows: Vec<PlanWindowUsage>,
    /// One line per window at or above the warning ratio, as `usage` words it.
    pub warnings: Vec<String>,
    /// Why the client cannot start, in the gateway's words; `None` while it can.
    pub unable_to_start: Option<String>,
}

/// One client's plan limit, read the way `usage` reads it: the shipped defaults
/// with this machine's override over them, and the machine-wide ledger's spend
/// in each window. A caller that runs a client itself asks this before it
/// plans, so it counts the client the way the gateway does.
pub fn read_plan_limit(runtime: &Runtime, client: &str) -> Result<PlanLimitReading, GatewayError> {
    let config = load_config(runtime)?;
    let entries = (LedgerStore { runtime }).read()?;
    let limits = config.clients.get(client).and_then(|known| known.plan_limits.as_deref()).unwrap_or(&[]);
    let windows = plan_windows(client, limits, &run_records(&entries), runtime.clock.now_ms(), config.settings.plan_warning_ratio);
    let unable_to_start =
        windows.iter().find(|window| window.state() == PlanState::Exhausted).map(|exhausted| format!("at 100% of {}", describe_window(exhausted)));
    Ok(PlanLimitReading { client: client.to_string(), warnings: warnings_of(client, &windows), windows, unable_to_start })
}
