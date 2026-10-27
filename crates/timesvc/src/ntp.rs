//! SNTPv4 (RFC 5905 §14) packets and offset/delay math; fixed-point NTP timestamps.

use alloc::vec::Vec;

/// Seconds between 1900-01-01 (NTP era 0) and 1970-01-01.
pub const NTP_UNIX_DELTA: u64 = 2_208_988_800;

/// 64-bit NTP timestamp (32.32 fixed point).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ts(pub u64);

impl Ts {
    /// From unix time in nanoseconds.
    pub fn from_unix_ns(ns: i128) -> Ts {
        let secs = (ns.div_euclid(1_000_000_000) + NTP_UNIX_DELTA as i128) as u64 & 0xFFFF_FFFF;
        let frac = ((ns.rem_euclid(1_000_000_000) as u128) << 32) / 1_000_000_000;
        Ts((secs << 32) | frac as u64)
    }
    /// To unix nanoseconds, assuming the timestamp is in era 0 or 1 closest to `hint_unix_s`.
    pub fn to_unix_ns(self, hint_unix_s: i64) -> i128 {
        let secs = (self.0 >> 32) as i128;
        let frac = ((self.0 & 0xFFFF_FFFF) as u128 * 1_000_000_000 >> 32) as i128;
        let mut unix = secs - NTP_UNIX_DELTA as i128;
        // Resolve era rollover (every 2^32 s) by choosing the value nearest the hint.
        let era = 1i128 << 32;
        while unix + era / 2 < hint_unix_s as i128 {
            unix += era;
        }
        unix * 1_000_000_000 + frac
    }
}

/// Parse/validate failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// Not 48 bytes.
    Length,
    /// Mode is not server.
    Mode,
    /// Version outside 3..=4.
    Version,
    /// Stratum 0 (kiss-o'-death) or >15 / alarm leap.
    Unsynchronized,
    /// Origin timestamp does not echo our transmit time (spoof/replay).
    OriginMismatch,
    /// Transmit timestamp zero.
    NoTransmit,
}

/// A decoded server reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    /// Leap indicator.
    pub leap: u8,
    /// Stratum.
    pub stratum: u8,
    /// Root delay (seconds, 16.16).
    pub root_delay: u32,
    /// Root dispersion (seconds, 16.16).
    pub root_disp: u32,
    /// Server receive time (T2).
    pub rx: Ts,
    /// Server transmit time (T3).
    pub tx: Ts,
}

/// Build a client request; `t1` is our transmit time (also sent so the reply must echo it).
pub fn request(t1: Ts) -> [u8; 48] {
    let mut p = [0u8; 48];
    p[0] = (4 << 3) | 3; // LI=0 VN=4 mode=client
    p[40..48].copy_from_slice(&t1.0.to_be_bytes());
    p
}

/// Decode and validate a reply to a request sent with transmit time `t1`.
pub fn parse_reply(p: &[u8], t1: Ts) -> Result<Reply, Reject> {
    if p.len() < 48 {
        return Err(Reject::Length);
    }
    let (li, vn, mode) = (p[0] >> 6, (p[0] >> 3) & 7, p[0] & 7);
    if mode != 4 {
        return Err(Reject::Mode);
    }
    if !(3..=4).contains(&vn) {
        return Err(Reject::Version);
    }
    if li == 3 || p[1] == 0 || p[1] > 15 {
        return Err(Reject::Unsynchronized);
    }
    let be64 = |o: usize| u64::from_be_bytes(p[o..o + 8].try_into().unwrap());
    if Ts(be64(24)) != t1 {
        return Err(Reject::OriginMismatch);
    }
    let tx = Ts(be64(40));
    if tx.0 == 0 {
        return Err(Reject::NoTransmit);
    }
    Ok(Reply {
        leap: li,
        stratum: p[1],
        root_delay: u32::from_be_bytes(p[4..8].try_into().unwrap()),
        root_disp: u32::from_be_bytes(p[8..12].try_into().unwrap()),
        rx: Ts(be64(32)),
        tx,
    })
}

/// One measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Clock offset (server - local), ns.
    pub offset_ns: i128,
    /// Round-trip delay, ns.
    pub delay_ns: i128,
    /// Server stratum.
    pub stratum: u8,
}

/// Standard on-wire calculation; `t1`/`t4` are local unix ns (send/receive).
pub fn measure(r: &Reply, t1_ns: i128, t4_ns: i128) -> Sample {
    let hint = (t1_ns / 1_000_000_000) as i64;
    let t2 = r.rx.to_unix_ns(hint);
    let t3 = r.tx.to_unix_ns(hint);
    Sample { offset_ns: ((t2 - t1_ns) + (t3 - t4_ns)) / 2, delay_ns: (t4_ns - t1_ns) - (t3 - t2), stratum: r.stratum }
}

/// Pick the best sample from several servers: drop negative-delay/over-`max_delay_ns` ones,
/// take the lowest delay (ties: lowest stratum). A step is needed above `step_threshold_ns`.
pub fn select(samples: &[Sample], max_delay_ns: i128) -> Option<Sample> {
    samples.iter().copied().filter(|s| s.delay_ns >= 0 && s.delay_ns <= max_delay_ns).min_by_key(|s| (s.delay_ns, s.stratum))
}

/// What to do with the local clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Within tolerance.
    None,
    /// Gradually slew by this many ppm (clamped to ±500).
    Slew(i32),
    /// Jump by this many ns.
    Step(i128),
}

/// Discipline policy: step if |offset| > 128 ms (ntpd default), ignore under 1 ms, otherwise
/// slew so the error is removed over ~`over_secs`.
pub fn discipline(offset_ns: i128, over_secs: i128) -> Action {
    let a = offset_ns.abs();
    if a > 128_000_000 {
        Action::Step(offset_ns)
    } else if a < 1_000_000 {
        Action::None
    } else {
        let ppm = (offset_ns * 1_000_000 / (over_secs.max(1) * 1_000_000_000)) as i32;
        Action::Slew(ppm.clamp(-500, 500))
    }
}

/// Poll interval after `n` consecutive good syncs: 64 s doubling up to 1024 s.
pub fn poll_interval(n: u32) -> u32 {
    64u32 << n.min(4)
}

/// Build a fake server reply (used by tests and the loopback self-test).
pub fn server_reply(req: &[u8; 48], rx: Ts, tx: Ts, stratum: u8) -> Vec<u8> {
    let mut p = [0u8; 48].to_vec();
    p[0] = (4 << 3) | 4;
    p[1] = stratum;
    p[24..32].copy_from_slice(&req[40..48]);
    p[32..40].copy_from_slice(&rx.0.to_be_bytes());
    p[40..48].copy_from_slice(&tx.0.to_be_bytes());
    p
}
