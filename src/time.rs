//! Minimal date/time helpers (no external crates). All instants are epoch milliseconds (UTC);
//! "local" values are computed with the OS time-zone offset.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(windows)]
pub fn local_offset_ms() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    let r = unsafe { GetTimeZoneInformation(&mut tz) };
    let mut bias = tz.Bias as i64;
    if r == 2 { bias += tz.DaylightBias as i64; } else if r == 1 { bias += tz.StandardBias as i64; }
    -bias * 60_000
}

#[cfg(not(windows))]
pub fn local_offset_ms() -> i64 {
    std::env::var("POCHI_TZ_MIN").ok().and_then(|v| v.parse::<i64>().ok()).unwrap_or(420) * 60_000
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    pub year: i32,
    pub month: u32, // 1..=12
    pub day: u32,   // 1..=31
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub weekday: u32, // 0 = Sunday
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

pub fn local(ms: i64) -> Local {
    let t = ms + local_offset_ms();
    let days = t.div_euclid(86_400_000);
    let rem = t.rem_euclid(86_400_000) / 1000;
    let (year, month, day) = civil_from_days(days);
    Local {
        year, month, day,
        hour: (rem / 3600) as u32,
        minute: ((rem % 3600) / 60) as u32,
        second: (rem % 60) as u32,
        weekday: ((days + 4).rem_euclid(7)) as u32,
    }
}

/// epoch ms of a local wall-clock time
pub fn from_local(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    let days = days_from_civil(year, month, day);
    days * 86_400_000 + (hour as i64 * 3600 + minute as i64 * 60) * 1000 - local_offset_ms()
}

pub fn day_key(ms: i64) -> String {
    let l = local(ms);
    format!("{:04}-{:02}-{:02}", l.year, l.month, l.day)
}

pub fn parse_day_key(k: &str) -> Option<(i32, u32, u32)> {
    let p: Vec<&str> = k.split('-').collect();
    if p.len() != 3 { return None; }
    Some((p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?))
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    (days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)) as u32
}

pub fn weekday_of(y: i32, m: u32, d: u32) -> u32 {
    ((days_from_civil(y, m, d) + 4).rem_euclid(7)) as u32
}

pub fn start_of_local_day(ms: i64) -> i64 {
    let l = local(ms);
    from_local(l.year, l.month, l.day, 0, 0)
}

/// "2026-10-27T08:15:00.000Z" -> epoch ms
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 { return None; }
    let y: i32 = s.get(0..4)?.parse().ok()?;
    let mo: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    let h: i64 = s.get(11..13)?.parse().ok()?;
    let mi: i64 = s.get(14..16)?.parse().ok()?;
    let se: i64 = s.get(17..19)?.parse().ok()?;
    let mut ms = 0i64;
    if let Some(rest) = s.get(19..) {
        if let Some(frac) = rest.strip_prefix('.') {
            let digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).take(3).collect();
            if !digits.is_empty() { ms = format!("{:0<3}", digits).parse().unwrap_or(0); }
        }
    }
    Some(days_from_civil(y, mo, d) * 86_400_000 + (h * 3600 + mi * 60 + se) * 1000 + ms)
}

pub fn to_iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let s = rem / 1000;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, m, d, s / 3600, (s % 3600) / 60, s % 60, rem % 1000)
}

pub const WD: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const WD_LONG: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
pub const MONTHS_LONG: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// (hh:mm, "AM"/"PM"/"")
pub fn clock(ms: i64, h12: bool) -> (String, &'static str) {
    let l = local(ms);
    if h12 {
        let h = if l.hour % 12 == 0 { 12 } else { l.hour % 12 };
        (format!("{:02}:{:02}", h, l.minute), if l.hour < 12 { "AM" } else { "PM" })
    } else {
        (format!("{:02}:{:02}", l.hour, l.minute), "")
    }
}

pub fn hm24(ms: i64) -> String {
    let l = local(ms);
    format!("{:02}:{:02}", l.hour, l.minute)
}

pub fn mmss(ms: i64) -> String {
    let s = (ms.max(0) + 999) / 1000;
    format!("{:02}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let t = parse_iso("2026-10-27T08:15:30.250Z").unwrap();
        assert_eq!(to_iso(t), "2026-10-27T08:15:30.250Z");
        let (y, m, d) = civil_from_days(days_from_civil(2024, 2, 29));
        assert_eq!((y, m, d), (2024, 2, 29));
        assert_eq!(weekday_of(2026, 10, 27), 2); // Tuesday
        assert_eq!(days_in_month(2024, 2), 29);
    }
}
