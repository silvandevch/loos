//! Small formatting helpers shared by the shell apps (C `util.c`).
#![allow(missing_docs)]

fn tail(out: &mut [u8], pos: usize) -> &mut [u8] {
    let i = pos.min(out.len());
    &mut out[i..]
}

fn put(out: &mut [u8], s: &[u8]) -> usize {
    if out.is_empty() {
        return s.len();
    }
    let n = s.len().min(out.len() - 1);
    out[..n].copy_from_slice(&s[..n]);
    out[n] = 0;
    s.len()
}

fn put_u64(out: &mut [u8], mut v: u64) -> usize {
    let mut tmp = [0u8; 20];
    let mut n = 0;
    if v == 0 {
        tmp[0] = b'0';
        n = 1;
    }
    while v != 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    for i in 0..n / 2 {
        tmp.swap(i, n - 1 - i);
    }
    put(out, &tmp[..n])
}

fn put_pad(out: &mut [u8], v: u64, width: usize) -> usize {
    let mut tmp = [0u8; 20];
    let mut n = 0;
    let mut v = v;
    if v == 0 {
        tmp[0] = b'0';
        n = 1;
    }
    while v != 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    let total = width.max(n);
    let mut written = 0;
    while written < total.saturating_sub(n) {
        if written < out.len().saturating_sub(1) {
            out[written] = b'0';
        }
        written += 1;
    }
    for i in 0..n {
        if written < out.len().saturating_sub(1) {
            out[written] = tmp[n - 1 - i];
        }
        written += 1;
    }
    written
}

pub fn size(b: u64, out: &mut [u8]) {
    const UNIT: [&[u8]; 5] = [b"B", b"KB", b"MB", b"GB", b"TB"];
    let mut u = 0;
    let mut whole = b;
    let mut rem = 0u64;
    while whole >= 1024 && u < 4 {
        rem = whole % 1024;
        whole /= 1024;
        u += 1;
    }
    if u == 0 {
        let pos = put_u64(out, b);
        put(tail(out, pos), b" B");
    } else {
        let mut pos = put_u64(out, whole);
        pos += put(tail(out, pos), b".");
        pos += put_u64(tail(out, pos), rem * 10 / 1024);
        pos += put(tail(out, pos), b" ");
        put(tail(out, pos), UNIT[u]);
    }
}

pub fn time(t: i64, out: &mut [u8]) {
    let mut days = t / 86400;
    let mut rem = t % 86400;
    if rem < 0 {
        rem += 86400;
        days -= 1;
    }
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    let mut pos = put_pad(out, y as u64, 4);
    pos += put(tail(out, pos), b"-");
    pos += put_pad(tail(out, pos), m, 2);
    pos += put(tail(out, pos), b"-");
    pos += put_pad(tail(out, pos), d, 2);
    pos += put(tail(out, pos), b" ");
    let hour = (rem / 3600) as u64;
    let min = (rem % 3600 / 60) as u64;
    pos += put_pad(tail(out, pos), hour, 2);
    pos += put(tail(out, pos), b":");
    put_pad(tail(out, pos), min, 2);
    if !out.is_empty() {
        let n = out.len() - 1;
        out[n] = 0;
    }
}

pub fn pct10(x10: u32, out: &mut [u8]) {
    let mut pos = put_u64(out, (x10 / 10) as u64);
    pos += put(tail(out, pos), b".");
    pos += put_u64(tail(out, pos), (x10 % 10) as u64);
    put(tail(out, pos), b"%");
}

#[no_mangle]
pub unsafe extern "C" fn fmt_size(bytes: u64, out: *mut core::ffi::c_char, cap: usize) {
    if out.is_null() || cap == 0 {
        return;
    }
    // SAFETY: caller provides cap writable bytes.
    size(bytes, unsafe { core::slice::from_raw_parts_mut(out as *mut u8, cap) });
}

#[no_mangle]
pub unsafe extern "C" fn fmt_time(t: i64, out: *mut core::ffi::c_char, cap: usize) {
    if out.is_null() || cap == 0 {
        return;
    }
    // SAFETY: caller provides cap writable bytes.
    time(t, unsafe { core::slice::from_raw_parts_mut(out as *mut u8, cap) });
}

#[no_mangle]
pub unsafe extern "C" fn fmt_pct10(x10: u32, out: *mut core::ffi::c_char, cap: usize) {
    if out.is_null() || cap == 0 {
        return;
    }
    // SAFETY: caller provides cap writable bytes.
    pct10(x10, unsafe { core::slice::from_raw_parts_mut(out as *mut u8, cap) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        let mut b = [0u8; 32];
        size(812, &mut b);
        assert_eq!(&b[..5], b"812 B");
        assert_eq!(b[5], 0);
        size(1536 * 1024, &mut b);
        assert_eq!(&b[..6], b"1.5 MB");
        size(1023, &mut b);
        assert_eq!(&b[..6], b"1023 B");
    }

    #[test]
    fn times() {
        let mut b = [0u8; 32];
        time(0, &mut b);
        assert_eq!(&b[..16], b"1970-01-01 00:00");
        time(1_766_000_000, &mut b);
        assert_eq!(&b[..16], b"2025-12-17 19:33");
    }

    #[test]
    fn percentages() {
        let mut b = [0u8; 16];
        pct10(123, &mut b);
        assert_eq!(&b[..5], b"12.3%");
        pct10(1000, &mut b);
        assert_eq!(&b[..5], b"100.0");
    }
}
