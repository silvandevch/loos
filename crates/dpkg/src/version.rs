//! Debian version: `[epoch:]upstream[-revision]`.

use alloc::string::{String, ToString};
use core::cmp::Ordering;

/// A parsed Debian version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// Epoch (0 if absent).
    pub epoch: u32,
    /// Upstream part.
    pub upstream: String,
    /// Debian revision ("" if absent).
    pub revision: String,
}

impl Version {
    /// Parse; `None` if empty, epoch is not a number, or upstream is empty/doesn't start with a digit.
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let (epoch, rest) = match s.split_once(':') {
            Some((e, r)) => (e.parse().ok()?, r),
            None => (0, s),
        };
        let (upstream, revision) = match rest.rsplit_once('-') {
            Some((u, r)) => (u, r),
            None => (rest, ""),
        };
        if !upstream.chars().next()?.is_ascii_digit() {
            return None;
        }
        Some(Version { epoch, upstream: upstream.to_string(), revision: revision.to_string() })
    }
}

fn order(c: u8) -> i32 {
    // '~' sorts before everything (even end of string); letters before other symbols.
    match c {
        b'~' => -1,
        b'0'..=b'9' => 0,
        c if c.is_ascii_alphabetic() => c as i32,
        c => c as i32 + 256,
    }
}

/// dpkg's `verrevcmp`.
pub fn cmp_part(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        let mut first_diff = 0i32;
        while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit()) {
            let ac = a.get(i).map_or(0, |&c| order(c));
            let bc = b.get(j).map_or(0, |&c| order(c));
            if ac != bc {
                return ac.cmp(&bc);
            }
            i += 1;
            j += 1;
        }
        while a.get(i) == Some(&b'0') {
            i += 1;
        }
        while b.get(j) == Some(&b'0') {
            j += 1;
        }
        while i < a.len() && a[i].is_ascii_digit() && j < b.len() && b[j].is_ascii_digit() {
            if first_diff == 0 {
                first_diff = a[i] as i32 - b[j] as i32;
            }
            i += 1;
            j += 1;
        }
        if i < a.len() && a[i].is_ascii_digit() {
            return Ordering::Greater;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            return Ordering::Less;
        }
        if first_diff != 0 {
            return first_diff.cmp(&0);
        }
    }
    Ordering::Equal
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        self.epoch
            .cmp(&o.epoch)
            .then_with(|| cmp_part(&self.upstream, &o.upstream))
            .then_with(|| cmp_part(&self.revision, &o.revision))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
