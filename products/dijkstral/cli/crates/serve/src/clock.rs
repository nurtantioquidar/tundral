//! UTC time stamps without a date library: the formats the work-memory files and `gh` use, and an
//! ISO 8601 reader as lenient as Python's `datetime.fromisoformat`.

use std::time::{SystemTime, UNIX_EPOCH};

/// A fixed "now" for tests: `DIJKSTRAL_NOW`, seconds since the Unix epoch. Not
/// `SOURCE_DATE_EPOCH`, which build shells export and which would skew real ages.
fn pinned() -> Option<f64> {
    std::env::var("DIJKSTRAL_NOW")
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()
}

/// Seconds since the Unix epoch, now.
pub fn now() -> f64 {
    if let Some(t) = pinned() {
        return t;
    }
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number from [`days_from_civil`].
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A moment as whole seconds since the Unix epoch and microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instant {
    pub secs: i64,
    pub micros: u32,
}

impl Instant {
    /// Now, from the system clock.
    pub fn now() -> Instant {
        if let Some(t) = pinned() {
            let micros = (t * 1e6).round() as i64;
            return Instant {
                secs: micros.div_euclid(1_000_000),
                micros: micros.rem_euclid(1_000_000) as u32,
            };
        }
        let t = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_micros() as i64,
            Err(e) => -(e.duration().as_micros() as i64),
        };
        Instant {
            secs: t.div_euclid(1_000_000),
            micros: t.rem_euclid(1_000_000) as u32,
        }
    }

    /// (year, month, day, hour, minute, second) in UTC.
    fn fields(self) -> (i64, u32, u32, u32, u32, u32) {
        let (y, m, d) = civil_from_days(self.secs.div_euclid(86_400));
        let sod = self.secs.rem_euclid(86_400) as u32;
        (y, m, d, sod / 3600, sod / 60 % 60, sod % 60)
    }

    /// `2026-10-15T15:56:41.434286+00:00`.
    pub fn iso_micros(self) -> String {
        let (y, mo, d, h, mi, s) = self.fields();
        let us = self.micros;
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{us:06}+00:00")
    }

    /// `20261015_155641`.
    pub fn compact_stamp(self) -> String {
        let (y, mo, d, h, mi, s) = self.fields();
        format!("{y:04}{mo:02}{d:02}_{h:02}{mi:02}{s:02}")
    }
}

/// An ISO 8601 date or date-time as Unix seconds. A time without an offset is UTC. Accepts what
/// `datetime.fromisoformat` accepts in practice: `YYYY-MM-DD` or `YYYYMMDD`, then optionally `T`
/// or a space and `HH[:MM[:SS[.ffffff]]]` (`,` also starts the fraction), then optionally `Z` or
/// `±HH[:MM[:SS]]`.
pub fn parse_iso(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let (date, rest) = if b.len() >= 10 && b[4] == b'-' && b[7] == b'-' {
        ((num(&b[0..4])?, num(&b[5..7])?, num(&b[8..10])?), &b[10..])
    } else if b.len() >= 8 && b[..8].iter().all(u8::is_ascii_digit) {
        ((num(&b[0..4])?, num(&b[4..6])?, num(&b[6..8])?), &b[8..])
    } else {
        return None;
    };
    let (y, m, d) = date;
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m as u32) {
        return None;
    }
    let mut t = days_from_civil(y, m as u32, d as u32) as f64 * 86_400.0;
    if rest.is_empty() {
        return Some(t);
    }
    if !matches!(rest[0], b'T' | b' ') {
        return None;
    }
    let rest = &rest[1..];
    let tz_at = rest
        .iter()
        .position(|&c| matches!(c, b'Z' | b'+' | b'-'))
        .unwrap_or(rest.len());
    let (time, tz) = rest.split_at(tz_at);
    t += parse_time(time)?;
    if !tz.is_empty() {
        t -= parse_offset(tz)?;
    }
    Some(t)
}

fn num(b: &[u8]) -> Option<i64> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(b).ok()?.parse().ok()
}

fn days_in_month(y: i64, m: u32) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn parse_time(b: &[u8]) -> Option<f64> {
    let (hms, frac) = match b.iter().position(|&c| c == b'.' || c == b',') {
        Some(i) => (&b[..i], Some(&b[i + 1..])),
        None => (b, None),
    };
    let parts: Vec<&[u8]> = if hms.contains(&b':') {
        hms.split(|&c| c == b':').collect()
    } else {
        hms.chunks(2).collect()
    };
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.len() != 2) {
        return None;
    }
    let h = num(parts[0])?;
    let m = parts.get(1).map_or(Some(0), |p| num(p))?;
    let s = parts.get(2).map_or(Some(0), |p| num(p))?;
    if h > 23 || m > 59 || s > 59 {
        return None;
    }
    let mut secs = (h * 3600 + m * 60 + s) as f64;
    if let Some(f) = frac {
        if parts.len() != 3 || f.is_empty() || f.len() > 9 || !f.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let digits = &f[..f.len().min(6)];
        secs += num(digits)? as f64 / 10f64.powi(digits.len() as i32);
    }
    Some(secs)
}

fn parse_offset(b: &[u8]) -> Option<f64> {
    if b == b"Z" {
        return Some(0.0);
    }
    let sign = match b.first()? {
        b'+' => 1.0,
        b'-' => -1.0,
        _ => return None,
    };
    let body = &b[1..];
    let parts: Vec<&[u8]> = if body.contains(&b':') {
        body.split(|&c| c == b':').collect()
    } else {
        body.chunks(2).collect()
    };
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.len() != 2) {
        return None;
    }
    let h = num(parts[0])?;
    let m = parts.get(1).map_or(Some(0), |p| num(p))?;
    let s = parts.get(2).map_or(Some(0), |p| num(p))?;
    if h > 23 || m > 59 || s > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60 + s) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips() {
        for z in [-800_000, -1, 0, 1, 20_000, 20_741, 60_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 10, 15), 20_741);
    }

    #[test]
    fn formats_utc() {
        let t = Instant {
            secs: 1_791_993_401,
            micros: 434_286,
        };
        assert_eq!(t.iso_micros(), "2026-10-14T15:56:41.434286+00:00");
        assert_eq!(t.compact_stamp(), "20261014_155641");
    }

    #[test]
    fn reads_iso_forms() {
        let base = parse_iso("2026-01-04").unwrap();
        assert_eq!(parse_iso("2026-01-04T00:00:00+00:00"), Some(base));
        assert_eq!(parse_iso("2026-01-04T00:00:00Z"), Some(base));
        assert_eq!(parse_iso("2026-01-04T00:00:00"), Some(base));
        assert_eq!(parse_iso("20260104"), Some(base));
        assert_eq!(parse_iso("2026-01-04T07:00:00+07:00"), Some(base));
        assert_eq!(parse_iso("2026-01-04 01:30"), Some(base + 5400.0));
        assert_eq!(parse_iso("2026-01-04T00:00:00.5"), Some(base + 0.5));
        for bad in [
            "",
            "garbage",
            "2026-13-01",
            "2026-02-30",
            "2026-01-04T25:00",
            "2026-01-04X",
        ] {
            assert_eq!(parse_iso(bad), None, "{bad}");
        }
    }
}
