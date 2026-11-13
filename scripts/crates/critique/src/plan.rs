//! A client's plan limit, read by the model gateway's own code: the shipped defaults with this machine's override
//! over them, and the machine-wide ledger's spend in each window. The node runner's `adapters/plan`.

use crate::error::CritiqueError;
use crate::runtime::Runtime;
use model_gateway::plan::{read_plan_limit, PlanLimitReading};

/// What the gateway says of one client's plan limit. The gateway's own error is not one of the runner's, so the command line reports it as `unexpected`, with its message.
pub fn plan_limit(runtime: &Runtime, client: &str) -> Result<PlanLimitReading, CritiqueError> {
    read_plan_limit(&runtime.gateway, client).map_err(|error| CritiqueError::unexpected(error.message))
}
