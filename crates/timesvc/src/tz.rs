//! POSIX TZ strings (`CET-1CEST,M3.5.0,M10.5.0/3`) and local-time conversion.

use crate::calendar::{days_from_civil, days_in_month, from_unix, DateTime};
use alloc::string::{String, ToString};

/// A DST transition rule `Mm.w.d/time`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Month 1-12.
    pub month: u32,
    /// Week 1-5 (5 = last).
    pub week: u32,
    /// Weekday 0 = Sunday.
    pub wday: u32,
    /// Local wall-clock seconds after midnight (default 7200).
    pub secs: i32,
}

/// Parsed zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    /// Standard abbreviation.
    pub std_name: String,
    /// Seconds east of UTC in standard time.
    pub std_off: i32,
    /// DST abbreviation, offset (east) and start/end rules.
    pub dst: Option<(String, i32, Rule, Rule)>,
}

fn name(s: &str) -> Option<(&str, &str)> {
    if let Some(r) = s.strip_prefix('<') {
        let e = r.find('>')?;
        return Some((&r[..e], &r[e + 1..]));
    }
    let e = s.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(s.len());
    (e >= 3).then(|| (&s[..e], &s[e..]))
}

/// `[+-]hh[:mm[:ss]]` -> seconds, returns rest.
fn hms(s: &str) -> Option<(i32, &str)> {
    let (sign, s) = match s.as_bytes().first()? {
        b'-' => (-1, &s[1..]),
        b'+' => (1, &s[1..]),
        _ => (1, s),
    };
    let end = s.find(|c: char| !c.is_ascii_digit() && c != ':').unwrap_or(s.len());
    let mut total = 0i32;
    let mut mult = 3600;
    for (i, p) in s[..end].split(':').enumerate() {
        if i > 2 {
            return None;
        }
        total += p.parse::<i32>().ok()? * mult;
        mult /= 60;
    }
    (end > 0).then_some((sign * total, &s[end..]))
}

fn rule(s: &str) -> Option<Rule> {
    let r = s.strip_prefix('M')?;
    let (spec, time) = match r.split_once('/') {
        Some((a, t)) => (a, Some(t)),
        None => (r, None),
    };
    let mut it = spec.split('.');
    let (month, week, wday): (u32, u32, u32) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if it.next().is_some() || !(1..=12).contains(&month) || !(1..=5).contains(&week) || wday > 6 {
        return None;
    }
    let secs = match time {
        Some(t) => {
            let (v, rest) = hms(t)?;
            if !rest.is_empty() {
                return None;
            }
            v
        }
        None => 7200,
    };
    Some(Rule { month, week, wday, secs })
}

impl Zone {
    /// Parse a POSIX TZ string. Only the `M` rule form is supported.
    pub fn parse(s: &str) -> Option<Zone> {
        let (std_name, rest) = name(s)?;
        let (west, rest) = hms(rest)?;
        let std_off = -west;
        if rest.is_empty() {
            return Some(Zone { std_name: std_name.to_string(), std_off, dst: None });
        }
        let (dst_name, rest) = name(rest)?;
        let (dst_off, rest) = if rest.starts_with(',') { (std_off + 3600, rest) } else { let (w, r) = hms(rest)?; (-w, r) };
        let rest = rest.strip_prefix(',')?;
        let (a, b) = rest.split_once(',')?;
        Some(Zone { std_name: std_name.to_string(), std_off, dst: Some((dst_name.to_string(), dst_off, rule(a)?, rule(b)?)) })
    }

    /// Unix time of the transition in `year`, given the offset in force *before* it.
    fn transition(r: &Rule, year: i64, off_before: i32) -> i64 {
        let first = days_from_civil(year, r.month, 1);
        let wd_first = (first + 4).rem_euclid(7) as u32;
        let mut day = 1 + (r.wday + 7 - wd_first) % 7 + (r.week - 1) * 7;
        while day > days_in_month(year, r.month) {
            day -= 7;
        }
        (days_from_civil(year, r.month, day)) * 86_400 + r.secs as i64 - off_before as i64
    }

    /// Offset east of UTC and DST flag at unix time `t`.
    pub fn offset_at(&self, t: i64) -> (i32, bool) {
        let Some((_, dst_off, start, end)) = &self.dst else { return (self.std_off, false) };
        let year = from_unix(t + self.std_off as i64).year;
        let s = Self::transition(start, year, self.std_off);
        let e = Self::transition(end, year, *dst_off);
        let in_dst = if s < e { t >= s && t < e } else { t >= s || t < e }; // southern hemisphere
        if in_dst { (*dst_off, true) } else { (self.std_off, false) }
    }

    /// Local broken-down time.
    pub fn local(&self, t: i64) -> (DateTime, i32, bool) {
        let (off, dst) = self.offset_at(t);
        (from_unix(t + off as i64), off, dst)
    }

    /// Abbreviation in force at `t`.
    pub fn abbrev(&self, t: i64) -> &str {
        match (&self.dst, self.offset_at(t).1) {
            (Some((n, ..)), true) => n,
            _ => &self.std_name,
        }
    }
}
