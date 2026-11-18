//! How many runs one tier takes at once. A `run --tier` is a process of its own, so the limit is held on disk, by
//! the same exclusive-create the ledger's lock uses: a tier has `limit` slot files under the state directory, a run
//! creates one before it starts and removes it when it ends, and a run that finds them all taken waits for one.
//! A slot older than `stale_s` belongs to a run that died, and is broken.

use crate::error::GatewayError;
use crate::ports::Runtime;
use crate::state_file::state_dir;
use crate::text::join;
use std::time::{Duration, Instant};

/// What a run needs to take its tier's slot.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotPlan {
    pub tier: String,
    pub limit: usize,
    /// How long to wait for a free slot, in seconds, before the run is refused.
    pub wait_s: f64,
    /// How old a slot is before it is taken to belong to a run that died, in seconds.
    pub stale_s: f64,
}

/// A slot a run holds: dropping it gives the slot back.
pub struct TierSlot<'a> {
    runtime: &'a Runtime,
    path: String,
}

impl Drop for TierSlot<'_> {
    fn drop(&mut self) {
        let _ = self.runtime.files.remove_file(&self.path);
    }
}

/// Takes a slot of the tier, waiting for one when all are taken; refused once `wait_s` has passed.
pub fn acquire<'a>(runtime: &'a Runtime, plan: &SlotPlan) -> Result<TierSlot<'a>, GatewayError> {
    let dir = join(&[&state_dir(&runtime.env)?, "tier-slots"]);
    runtime.files.mkdir_all(&dir).map_err(GatewayError::from_fs)?;
    let deadline = Instant::now() + Duration::from_secs_f64(plan.wait_s);
    loop {
        for number in 0..plan.limit {
            let path = join(&[&dir, &format!("{}.{number}", plan.tier)]);
            if take(runtime, &path, plan.stale_s)? {
                return Ok(TierSlot { runtime, path });
            }
        }
        if Instant::now() >= deadline {
            return Err(GatewayError::new(
                "tier-busy",
                format!("{} already runs {} at once, and none was free for {} s: wait for one to end, or raise settings.quota.parallel", plan.tier, plan.limit, plan.wait_s),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Creates the slot file, breaking one that is too old to be alive.
fn take(runtime: &Runtime, path: &str, stale_s: f64) -> Result<bool, GatewayError> {
    let files = runtime.files.as_ref();
    if files.create_exclusive(path).map_err(GatewayError::from_fs)? {
        return Ok(true);
    }
    let old = files.modified_ms(path).is_some_and(|modified| (runtime.clock.now_ms() - modified) as f64 > stale_s * 1000.0);
    if !old {
        return Ok(false);
    }
    let _ = files.remove_file(path);
    files.create_exclusive(path).map_err(GatewayError::from_fs)
}
