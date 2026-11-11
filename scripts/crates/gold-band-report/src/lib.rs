//! The gate report: the one record a Gold Band shim leaves behind (#444),
//! shared by the run-start shim and the gate shim (#445).
//!
//! A report says which command ran for which run and attempt, the exit code
//! it earned, the SHA-256 of the output it printed, and when. A refused
//! check also names the check that failed. It is plain JSON with a fixed key
//! order, written atomically into a report directory (a temp file in that
//! directory, then a rename), so a reader never sees half a report.
//!
//! The clock is injected ([`Clock`]); the SHA-256 and the civil-date
//! conversion are written in std, since no hashing or time crate may join
//! the lockfile for this.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// The outside world a report reaches for: the current time, in whole
/// seconds since the Unix epoch.
pub trait Clock {
    /// Seconds since 1970-01-01T00:00:00Z.
    fn unix_seconds(&self) -> u64;
}

/// The system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

/// One gate report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The Gold Band run the report belongs to.
    pub run_id: String,
    /// The attempt number within the run (1 for the first).
    pub attempt: u32,
    /// The command line that ran.
    pub command: String,
    /// The exit code it earned: 0 passed, anything else did not.
    pub exit_code: i32,
    /// The SHA-256 of the output it printed, as 64 lowercase hex digits.
    pub output_hash: String,
    /// When the report was built, RFC 3339 in UTC (`2026-11-11T09:30:00Z`).
    pub timestamp: String,
    /// The check that failed, present only when the command was refused.
    pub failed_check: Option<String>,
}

impl Report {
    /// Builds a report: hashes `output` and stamps the clock's time.
    pub fn build(
        run_id: &str,
        attempt: u32,
        command: &str,
        exit_code: i32,
        output: &str,
        failed_check: Option<&str>,
        clock: &dyn Clock,
    ) -> Report {
        Report {
            run_id: run_id.to_string(),
            attempt,
            command: command.to_string(),
            exit_code,
            output_hash: sha256_hex(output.as_bytes()),
            timestamp: rfc3339(clock.unix_seconds()),
            failed_check: failed_check.map(str::to_string),
        }
    }

    /// The report as one line of JSON. The keys are always in the order
    /// run_id, attempt, command, exit_code, output_hash, timestamp and, only
    /// when set, failed_check.
    pub fn to_json(&self) -> String {
        let mut out = format!(
            "{{\"run_id\":{},\"attempt\":{},\"command\":{},\"exit_code\":{},\"output_hash\":{},\"timestamp\":{}",
            quoted(&self.run_id),
            self.attempt,
            quoted(&self.command),
            self.exit_code,
            quoted(&self.output_hash),
            quoted(&self.timestamp),
        );
        if let Some(check) = &self.failed_check {
            out.push_str(&format!(",\"failed_check\":{}", quoted(check)));
        }
        out.push('}');
        out
    }

    /// Parses a report, refusing a missing or mistyped field, an output hash
    /// that is not 64 lowercase hex digits and a timestamp that is not
    /// RFC 3339 UTC.
    pub fn from_json(text: &str) -> Result<Report, String> {
        let value: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        let string = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("{key} must be a string"))
        };
        let number = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_i64)
                .ok_or_else(|| format!("{key} must be an integer"))
        };
        let attempt = u32::try_from(number("attempt")?)
            .map_err(|_| "attempt must be a non-negative 32-bit integer".to_string())?;
        let exit_code = i32::try_from(number("exit_code")?)
            .map_err(|_| "exit_code must be a 32-bit integer".to_string())?;
        let output_hash = string("output_hash")?;
        if output_hash.len() != 64
            || !output_hash
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err("output_hash must be 64 lowercase hex digits".to_string());
        }
        let timestamp = string("timestamp")?;
        if parse_rfc3339(&timestamp).is_none() {
            return Err("timestamp must be RFC 3339 UTC like 2026-11-11T09:30:00Z".to_string());
        }
        let failed_check = match value.get("failed_check") {
            None => None,
            Some(v) => Some(
                v.as_str()
                    .map(str::to_string)
                    .ok_or("failed_check must be a string")?,
            ),
        };
        Ok(Report {
            run_id: string("run_id")?,
            attempt,
            command: string("command")?,
            exit_code,
            output_hash,
            timestamp,
            failed_check,
        })
    }
}

/// `text` as a JSON string literal.
fn quoted(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

/// Writes `report` to `<dir>/<file_name>` atomically: the directory is
/// created when missing, the JSON goes to a temp file beside the target and
/// is renamed over it. Returns the final path.
pub fn write(dir: &Path, file_name: &str, report: &Report) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let target = dir.join(file_name);
    let temp = dir.join(format!(".{file_name}.tmp-{}", std::process::id()));
    fs::write(&temp, format!("{}\n", report.to_json()))?;
    if let Err(e) = fs::rename(&temp, &target) {
        // Never leave the temp file behind next to a report that was not written.
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    Ok(target)
}

/// Reads and parses the report at `path`.
pub fn read(path: &Path) -> Result<Report, String> {
    let shown = path.display();
    let text = fs::read_to_string(path).map_err(|e| format!("{shown}: cannot read: {e}"))?;
    Report::from_json(&text).map_err(|e| format!("{shown}: {e}"))
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// The SHA-256 of `data` as 64 lowercase hex digits (FIPS 180-4).
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for start in (0..padded.len()).step_by(64) {
        let block = &padded[start..start + 64];
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            let at = i * 4;
            *word = u32::from_be_bytes([block[at], block[at + 1], block[at + 2], block[at + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (slot, add) in h.iter_mut().zip(v) {
            *slot = slot.wrapping_add(add);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

/// Seconds since the Unix epoch as RFC 3339 UTC, `2026-11-11T09:30:00Z`.
pub fn rfc3339(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let rest = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Parses RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`, with an optional fraction of
/// up to three digits, which is dropped) into seconds since the Unix epoch.
/// `None` for any other shape, an impossible date or a time before 1970.
pub fn parse_rfc3339(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    let (core, tail) = (b.get(..19)?, &b[19..]);
    let fraction_ok = match tail {
        [b'Z'] => true,
        [b'.', digits @ .., b'Z'] => {
            (1..=3).contains(&digits.len()) && digits.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    };
    let shape = b"0000-00-00T00:00:00";
    let shaped = core.iter().zip(shape).all(|(c, s)| match s {
        b'0' => c.is_ascii_digit(),
        _ => c == s,
    });
    if !fraction_ok || !shaped {
        return None;
    }
    let num = |from: usize, to: usize| text[from..to].parse::<i64>().ok();
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if civil_from_days(days) != (year, month, day) {
        return None;
    }
    u64::try_from(days * 86_400 + hour * 3600 + minute * 60 + second).ok()
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// (year, month, day) to days since 1970-01-01. A month or day out of range
/// gives a day count whose [`civil_from_days`] differs, which callers use to
/// refuse the date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests;
