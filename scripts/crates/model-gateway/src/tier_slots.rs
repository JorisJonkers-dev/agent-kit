//! How many runs one tier takes at once. A `run --tier` is a process of its own, so the limit is held on disk: a
//! tier has `limit` slot files under the state directory, and a run takes a slot by holding an exclusive OS lock
//! (`flock`) on one of them for as long as it runs. The lock belongs to the open file, so the system releases it when
//! the run ends however it ends, a killed run included: there is no staleness to guess and no file to delete, so a run
//! can never take over a slot that is still held, and a run never frees a slot that is not its own. The slot files are
//! left in place, empty.

use crate::error::GatewayError;
use crate::ports::Runtime;
use crate::state_file::state_dir;
use crate::text::join;
use std::fs::{File, OpenOptions, TryLockError};
use std::time::{Duration, Instant};

/// What a run needs to take its tier's slot.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotPlan {
    pub tier: String,
    pub limit: usize,
    /// How long to wait for a free slot, in seconds, before the run is refused.
    pub wait_s: f64,
}

/// A slot a run holds: dropping it closes the file, which releases the lock, and nothing else.
pub struct TierSlot {
    _lock: File,
}

/// Takes a slot of the tier, waiting for one when all are taken; refused once `wait_s` has passed.
pub fn acquire(runtime: &Runtime, plan: &SlotPlan) -> Result<TierSlot, GatewayError> {
    let dir = join(&[&state_dir(&runtime.env)?, "tier-slots"]);
    runtime.files.mkdir_all(&dir).map_err(GatewayError::from_fs)?;
    let deadline = Instant::now() + Duration::from_secs_f64(plan.wait_s);
    loop {
        for number in 0..plan.limit {
            let path = join(&[&dir, &format!("{}.{number}", plan.tier)]);
            let file = OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|error| GatewayError::new("unexpected", format!("{path}: {error}")))?;
            let lock = |error: TryLockError| match error { TryLockError::WouldBlock => Ok(false), TryLockError::Error(error) => Err(GatewayError::new("unexpected", format!("{path}: cannot lock the slot: {error}"))) };
            if file.try_lock().map(|()| true).or_else(lock)? {
                return Ok(TierSlot { _lock: file });
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
