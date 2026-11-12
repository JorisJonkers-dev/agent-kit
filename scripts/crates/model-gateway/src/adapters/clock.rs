use crate::ports::Clock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The system clock, in epoch milliseconds as `Date.now()` gives them.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis() as i64)
    }
}
