//! The grant a run is held to: the live grant that covers a workflow and
//! client now, which clients a live grant covers, and the `no-grant` refusal.
//! Every read settles the reservations whose runs died first, so a dead run's
//! reservation never makes a grant look spent for the run that asks next.

use crate::error::GatewayError;
use crate::grants::{self, Grant};
use crate::grants_file::GrantsStore;
use crate::ledger_file::{settle_stale, LedgerStore};
use crate::ports::Runtime;
use crate::state_file::Workflows;
use std::collections::BTreeSet;

pub struct GrantGuard<'a> {
    pub runtime: &'a Runtime,
}

impl GrantGuard<'_> {
    /// The live grant that covers a workflow and client now, or `None` when every grant of it lapsed.
    pub fn live(&self, workflow: &str, client: &str, now_ms: i64) -> Result<Option<Grant>, GatewayError> {
        settle_stale(self.runtime, now_ms)?;
        let records = (Workflows { runtime: self.runtime }).read()?;
        let record = records.iter().find(|candidate| candidate.name == workflow);
        let grants = (GrantsStore { runtime: self.runtime }).read()?;
        let entries = (LedgerStore { runtime: self.runtime }).read()?;
        Ok(grants::live_grant(&grants, workflow, client, &entries, record, now_ms))
    }

    /// The clients a live grant covers for a workflow now, which is what lets their models into the fall-through pool.
    pub fn live_clients(&self, workflow: &str, now_ms: i64) -> Result<BTreeSet<String>, GatewayError> {
        settle_stale(self.runtime, now_ms)?;
        let records = (Workflows { runtime: self.runtime }).read()?;
        let record = records.iter().find(|candidate| candidate.name == workflow);
        let entries = (LedgerStore { runtime: self.runtime }).read()?;
        let mut clients = BTreeSet::new();
        for grant in (GrantsStore { runtime: self.runtime }).read()? {
            if grant.workflow != workflow {
                continue;
            }
            if grants::grant_lapse(&grant, &grants::grant_room(&entries, &grant), record, now_ms).is_some() {
                continue;
            }
            clients.insert(grant.client);
        }
        Ok(clients)
    }

    /// The live grant that covers a workflow and client, or a `no-grant` refusal naming the call that would add one.
    pub fn require(&self, workflow: &str, client: &str, now_ms: i64) -> Result<Grant, GatewayError> {
        self.live(workflow, client, now_ms)?.ok_or_else(|| GatewayError::new("no-grant", grants::no_grant_message(workflow, client)))
    }
}
