use crate::ports::Clock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs() as i64)
    }
}
