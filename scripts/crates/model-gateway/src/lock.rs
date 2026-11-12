//! An exclusive lock file. Two gateway processes taking the same lock take
//! turns, so what each does under it is never interleaved with the other's; a
//! lock left behind by a process that died is broken once it is old enough.

use crate::error::GatewayError;
use crate::ports::{Clock, Files, LockTimings};

/// The lock a writer takes, and what it guards, for the message of a writer that gave up waiting.
pub struct FileLock<'a> {
    name: &'a str,
    timings: LockTimings,
}

impl<'a> FileLock<'a> {
    pub fn new(name: &'a str, timings: LockTimings) -> Self {
        FileLock { name, timings }
    }

    /// Runs `action` holding the lock at `lock`, and releases it whatever the action does.
    pub fn run<T>(
        &self,
        lock: &str,
        files: &dyn Files,
        clock: &dyn Clock,
        action: impl FnOnce() -> Result<T, GatewayError>,
    ) -> Result<T, GatewayError> {
        self.acquire(lock, files, clock)?;
        let result = action();
        let _ = files.remove_file(lock);
        result
    }

    /// Takes the lock, waiting for whoever holds it; a lock that is too old to be alive is broken.
    fn acquire(&self, lock: &str, files: &dyn Files, clock: &dyn Clock) -> Result<(), GatewayError> {
        let deadline = clock.now_ms() + self.timings.timeout_ms;
        loop {
            if files.create_exclusive(lock).map_err(GatewayError::from_fs)? {
                return Ok(());
            }
            if self.stale(lock, files, clock) {
                let _ = files.remove_file(lock);
                continue;
            }
            if clock.now_ms() >= deadline {
                return Err(GatewayError::new(
                    "unexpected",
                    format!("{lock}: another gateway process held the {} lock for {} ms", self.name, self.timings.timeout_ms),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(self.timings.retry_ms));
        }
    }

    /// Whether the lock is older than a live writer could be; a lock that vanished meanwhile is not stale.
    fn stale(&self, lock: &str, files: &dyn Files, clock: &dyn Clock) -> bool {
        match files.modified_ms(lock) {
            Some(modified) => clock.now_ms() - modified > self.timings.stale_ms,
            None => false,
        }
    }
}
