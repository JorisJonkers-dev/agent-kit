//! Whether one client can start now: what `usage` reports as its availability,
//! asking only that client's own status command, so a delegation never waits on
//! the status of clients it does not use.

use crate::caps::run_records;
use crate::config::{ClientConfig, GatewayConfig};
use crate::error::GatewayError;
use crate::ledger_file::LedgerStore;
use crate::ports::Runtime;
use crate::status;
use crate::usage::{self, Availability, ClientReading};

/// What the gateway finds out about each of these clients without sending a prompt: PATH, and the client's own status command.
pub fn read_clients<'a>(runtime: &Runtime, clients: impl Iterator<Item = (&'a String, &'a ClientConfig)>) -> Vec<ClientReading> {
    clients
        .map(|(id, client)| {
            let installed = runtime.process.resolve(&client.binary).is_some();
            let status = if installed { status::read_status(runtime, id, status::STATUS_TIMEOUT_MS) } else { None };
            ClientReading { client: id.clone(), installed, status }
        })
        .collect()
}

/// Whether one client can start now. A client the gateway does not know has no availability.
pub fn client_availability(runtime: &Runtime, config: &GatewayConfig, client: &str) -> Result<Option<Availability>, GatewayError> {
    let Some((id, known)) = config.clients.get_key_value(client) else {
        return Ok(None);
    };
    let readings = read_clients(runtime, std::iter::once((id, known)));
    let entries = (LedgerStore { runtime }).read()?;
    let (_, availability) =
        usage::client_standing(id, known, readings.first(), &run_records(&entries), runtime.clock.now_ms(), &config.settings);
    Ok(Some(availability))
}
