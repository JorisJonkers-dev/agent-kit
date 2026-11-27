//! The lease record a claim stores in a git commit message, with the
//! UTC time format and the clocks it needs. This module is deliberately
//! std-only, with no dependency of any kind.

use std::time::{SystemTime, UNIX_EPOCH};

/// One device lease: the device, the run holding it, the issue it answers,
/// and the moment it expires in Unix seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Lease {
    pub device: String,
    pub run: String,
    pub issue: u64,
    pub expires: u64,
}

/// Formats Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` in UTC, computed with the
/// civil-from-days algorithm rather than a date crate.
pub fn format_time(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let within_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        within_day / 3600,
        (within_day / 60) % 60,
        within_day % 60
    )
}

/// Converts days since 1970-01-01 into a proleptic Gregorian date. The
/// era-based steps are Howard Hinnant's civil-from-days algorithm; shifting
/// the year boundary to March keeps the leap-day correction in one place.
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u64, day as u64)
}

/// Parses exactly `YYYY-MM-DDTHH:MM:SSZ` into Unix seconds, or returns
/// `None` for any other string, including an impossible date.
pub fn parse_time(s: &str) -> Option<u64> {
    let bytes = s.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let year = digits(bytes, 0, 4)? as i64;
    let month = digits(bytes, 5, 2)?;
    let day = digits(bytes, 8, 2)?;
    let hour = digits(bytes, 11, 2)?;
    let minute = digits(bytes, 14, 2)?;
    let second = digits(bytes, 17, 2)?;
    if month == 0
        || month > 12
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return None;
    }
    Some(days as u64 * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Reads exactly `len` ASCII digits from `bytes`, or returns `None`.
fn digits(bytes: &[u8], start: usize, len: usize) -> Option<u64> {
    let mut value = 0;
    for &byte in &bytes[start..start + len] {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u64::from(byte - b'0');
    }
    Some(value)
}

/// The last day of a month in a proleptic Gregorian year.
fn days_in_month(year: i64, month: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => if is_leap(year) { 29 } else { 28 },
    }
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Converts a proleptic Gregorian date into days since 1970-01-01. This is
/// the exact inverse of [`civil_from_days`] for the dates it accepts.
fn days_from_civil(year: i64, month: u64, day: u64) -> i64 {
    let shifted_year = year - i64::from(month <= 2);
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year - era * 400;
    let month_number = month as i64;
    let shifted_month = month_number + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

impl Lease {
    /// The commit message body that records this lease.
    pub fn message(&self) -> String {
        format!(
            "claim lease\n\ndevice: {}\nrun: {}\nissue: {}\nlease-expires: {}\n",
            self.device,
            self.run,
            self.issue,
            format_time(self.expires)
        )
    }

    /// Reads a lease from a commit message. Fields may appear in any order;
    /// every other line is ignored. The error names the offending field.
    pub fn parse(message: &str) -> Result<Lease, String> {
        let mut device = None;
        let mut run = None;
        let mut issue = None;
        let mut expires = None;
        for line in message.lines() {
            if let Some(value) = line.strip_prefix("device: ") {
                device = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("run: ") {
                run = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("issue: ") {
                issue = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("malformed issue: {value}"))?,
                );
            } else if let Some(value) = line.strip_prefix("lease-expires: ") {
                expires = Some(
                    parse_time(value)
                        .ok_or_else(|| format!("malformed lease-expires: {value}"))?,
                );
            }
        }
        Ok(Lease {
            device: device.ok_or_else(|| "missing device".to_string())?,
            run: run.ok_or_else(|| "missing run".to_string())?,
            issue: issue.ok_or_else(|| "missing issue".to_string())?,
            expires: expires.ok_or_else(|| "missing lease-expires".to_string())?,
        })
    }

    /// Whether the lease has run out at `now` (Unix seconds).
    pub fn expired(&self, now: u64) -> bool {
        now >= self.expires
    }
}

/// A source of the current time, so tests can supply a fixed instant.
pub trait Clock {
    fn now(&self) -> u64;
}

/// The clock backed by the operating system.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

/// A clock fixed at one instant; useful to tests in other modules.
pub struct FixedClock(pub u64);

impl Clock for FixedClock {
    fn now(&self) -> u64 {
        self.0
    }
}
