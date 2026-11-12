//! The timestamps the workflows file holds: `Date.parse` for the forms it
//! accepts, and `toISOString` for the ones the gateway writes.

/// How long a workflow lives when nobody ends it.
pub const WORKFLOW_TTL_MS: i64 = 24 * 60 * 60 * 1000;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// `Date.parse` for the ISO forms the file may hold: `YYYY-MM-DD`,
/// `YYYY-MM-DDTHH:MM`, `...:SS`, fractional seconds of any length, `Z` or a
/// `+HH:MM`/`-HH:MM` offset, and a `+YYYYYY` expanded year. A date-time
/// without a zone is UTC. Anything else is `None`.
pub fn parse(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let mut at = 0;
    let year = year(bytes, &mut at)?;
    expect(bytes, &mut at, b'-')?;
    let month = number(bytes, &mut at, 2)?;
    expect(bytes, &mut at, b'-')?;
    let day = number(bytes, &mut at, 2)?;
    let (mut hour, mut minute, mut second, mut millis, mut offset) = (0, 0, 0, 0, 0);
    if at < bytes.len() {
        if !matches!(bytes[at], b'T' | b't' | b' ') {
            return None;
        }
        at += 1;
        hour = number(bytes, &mut at, 2)?;
        expect(bytes, &mut at, b':')?;
        minute = number(bytes, &mut at, 2)?;
        if bytes.get(at) == Some(&b':') {
            at += 1;
            second = number(bytes, &mut at, 2)?;
            if bytes.get(at) == Some(&b'.') {
                at += 1;
                millis = fraction(bytes, &mut at)?;
            }
        }
        if at < bytes.len() {
            offset = zone(bytes, &mut at)?;
        }
    }
    if at != bytes.len() || !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * DAY_MS + hour * 3_600_000 + minute * 60_000 + second * 1000 + millis - offset)
}

/// `Date.prototype.toISOString`: `YYYY-MM-DDTHH:MM:SS.mmmZ`, with an expanded year outside 0..9999.
pub fn to_iso(ms: i64) -> String {
    let (year, month, day) = civil_from_days(ms.div_euclid(DAY_MS));
    let rest = ms.rem_euclid(DAY_MS);
    let year_text = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", -year)
    } else {
        format!("+{year:06}")
    };
    format!(
        "{year_text}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest % 3_600_000 / 60_000,
        rest % 60_000 / 1000,
        rest % 1000
    )
}

/// The year, four digits, or six after a `+` or `-`.
fn year(bytes: &[u8], at: &mut usize) -> Option<i64> {
    let (sign, signed) = match bytes.get(*at) {
        Some(b'+') => {
            *at += 1;
            (1, true)
        }
        Some(b'-') => {
            *at += 1;
            (-1, true)
        }
        _ => (1, false),
    };
    let start = *at;
    while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
        *at += 1;
    }
    if *at - start != if signed { 6 } else { 4 } {
        return None;
    }
    let digits = std::str::from_utf8(&bytes[start..*at]).ok()?;
    Some(sign * digits.parse::<i64>().ok()?)
}

/// Exactly `width` ASCII digits.
fn number(bytes: &[u8], at: &mut usize, width: usize) -> Option<i64> {
    let start = *at;
    while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
        *at += 1;
    }
    if *at - start != width {
        return None;
    }
    std::str::from_utf8(&bytes[start..*at]).ok()?.parse().ok()
}

fn expect(bytes: &[u8], at: &mut usize, byte: u8) -> Option<()> {
    if bytes.get(*at) != Some(&byte) {
        return None;
    }
    *at += 1;
    Some(())
}

/// The milliseconds of a fractional second: the first three digits, padded, as `Date.parse` truncates them.
fn fraction(bytes: &[u8], at: &mut usize) -> Option<i64> {
    let start = *at;
    while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
        *at += 1;
    }
    if *at == start {
        return None;
    }
    let mut millis = 0;
    for index in 0..3 {
        let digit = bytes.get(start + index).filter(|byte| byte.is_ascii_digit()).map_or(0, |byte| i64::from(byte - b'0'));
        millis = millis * 10 + digit;
    }
    Some(millis)
}

/// The zone, as the milliseconds it takes off the wall clock: `Z`, or `+HH:MM` / `-HH:MM`.
fn zone(bytes: &[u8], at: &mut usize) -> Option<i64> {
    match bytes.get(*at) {
        Some(b'Z' | b'z') => {
            *at += 1;
            Some(0)
        }
        Some(b'+' | b'-') => {
            let sign = if bytes[*at] == b'-' { -1 } else { 1 };
            *at += 1;
            let hours = number(bytes, at, 2)?;
            expect(bytes, at, b':')?;
            let minutes = number(bytes, at, 2)?;
            Some(sign * (hours * 60 + minutes) * 60_000)
        }
        _ => None,
    }
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days from 1970-01-01 to `year-month-day`, as Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil date `days` after 1970-01-01, as Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 { shifted } else { shifted - 146_096 } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 { month_prime + 3 } else { month_prime - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}
