use crate::ports::Clock;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// The moment the first reading was taken, which every later reading is measured from.
static STARTED: OnceLock<Instant> = OnceLock::new();

/// The system clock, in epoch milliseconds as `Date.now()` gives them.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis() as i64)
    }

    fn monotonic_ms(&self) -> f64 {
        STARTED.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
    }
}
