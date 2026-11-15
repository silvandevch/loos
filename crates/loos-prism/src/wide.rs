//! Multibyte conversion (UTF-8), `strtod`, group lists and audit info.
//!
//! Text is UTF-8 in every locale (ASCII is unchanged, which keeps the "C" locale's behaviour
//! for plain text). An `mbstate_t` keeps the bytes of an incomplete character: byte 0 is their
//! count, bytes 1..4 the bytes.

#[allow(unused_imports)]
use alloc::{string::String, vec::Vec};
use crate::libsys::{Act, Call, Func};
use loos_lin::mem::UserMem;

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

const EILSEQ: i64 = 84;
/// `(size_t)-1`, `(size_t)-2`.
const BAD: u64 = u64::MAX;
const SHORT: u64 = u64::MAX - 1;

/// Decode one character from `b`: Ok((char, length)), Err(true) incomplete, Err(false) invalid.
pub(crate) fn decode(b: &[u8]) -> Result<(u32, usize), bool> {
    let c0 = *b.first().ok_or(true)?;
    let (n, init) = match c0 {
        0..=0x7f => return Ok((c0 as u32, 1)),
        0xc2..=0xdf => (2, c0 as u32 & 0x1f),
        0xe0..=0xef => (3, c0 as u32 & 0x0f),
        0xf0..=0xf4 => (4, c0 as u32 & 0x07),
        _ => return Err(false),
    };
    let mut v = init;
    for i in 1..n {
        let c = *b.get(i).ok_or(true)?;
        if c & 0xc0 != 0x80 {
            return Err(false);
        }
        v = v << 6 | (c as u32 & 0x3f);
    }
    let min = [0, 0, 0x80, 0x800, 0x10000][n];
    if v < min || v > 0x10_ffff || (0xd800..0xe000).contains(&v) {
        return Err(false);
    }
    Ok((v, n))
}

pub(crate) fn encode(c: u32) -> Option<Vec<u8>> {
    let ch = char::from_u32(c)?;
    let mut b = [0u8; 4];
    Some(ch.encode_utf8(&mut b).as_bytes().to_vec())
}

/// `mbrtowc(pwc, s, n, ps)` core; `ps` 0 means no state.
fn mbrtowc(c: &mut Call, pwc: u64, s: u64, n: u64, ps: u64) -> Act {
    if s == 0 {
        if ps != 0 {
            let _ = c.mem.write(ps, &[0u8; 8]);
        }
        return c.ret(0);
    }
    let mut pend = [0u8; 8];
    if ps != 0 {
        let _ = c.mem.read(ps, &mut pend);
    }
    let have = (pend[0] as usize).min(3);
    let mut bytes = pend[1..1 + have].to_vec();
    let take = n.min(4) as usize;
    let mut buf = [0u8; 4];
    let got = (0..take).take_while(|&i| c.mem.read(s + i as u64, &mut buf[i..i + 1]).is_ok()).count();
    bytes.extend_from_slice(&buf[..got]);
    match decode(&bytes) {
        Ok((w, len)) => {
            if pwc != 0 {
                let _ = c.mem.write_u32(pwc, w);
            }
            if ps != 0 {
                let _ = c.mem.write(ps, &[0u8; 8]);
            }
            c.ret(if w == 0 { 0 } else { (len - have) as u64 })
        }
        Err(true) => {
            if ps != 0 {
                let mut st = [0u8; 8];
                st[0] = bytes.len() as u8;
                st[1..1 + bytes.len()].copy_from_slice(&bytes);
                let _ = c.mem.write(ps, &st);
            }
            c.ret(SHORT)
        }
        Err(false) => {
            c.set_errno(EILSEQ);
            c.ret(BAD)
        }
    }
}

/// Convert a NUL-terminated multibyte string at `*src` (or `src` itself when `indirect` is false)
/// into at most `len` wide characters at `dst` (0: just count).
fn mbs_to_wcs(c: &mut Call, dst: u64, src: u64, len: u64, indirect: bool) -> Act {
    let mut p = if indirect { c.mem.read_u64(src).unwrap_or(0) } else { src };
    let mut n = 0u64;
    loop {
        if dst != 0 && n >= len {
            break;
        }
        let mut b = [0u8; 4];
        for (i, x) in b.iter_mut().enumerate() {
            let mut one = [0u8];
            if c.mem.read(p + i as u64, &mut one).is_err() {
                break;
            }
            *x = one[0];
            if one[0] == 0 {
                break;
            }
        }
        match decode(&b) {
            Ok((0, _)) => {
                if dst != 0 {
                    let _ = c.mem.write_u32(dst + n * 4, 0);
                    if indirect {
                        let _ = c.mem.write_u64(src, 0);
                    }
                }
                return c.ret(n);
            }
            Ok((w, l)) => {
                if dst != 0 {
                    let _ = c.mem.write_u32(dst + n * 4, w);
                }
                p += l as u64;
                n += 1;
            }
            Err(_) => {
                if dst != 0 && indirect {
                    let _ = c.mem.write_u64(src, p);
                }
                c.set_errno(EILSEQ);
                return c.ret(BAD);
            }
        }
    }
    if indirect {
        let _ = c.mem.write_u64(src, p);
    }
    c.ret(n)
}

fn wcs_to_mbs(c: &mut Call, dst: u64, src: u64, len: u64, indirect: bool) -> Act {
    let mut p = if indirect { c.mem.read_u64(src).unwrap_or(0) } else { src };
    let mut n = 0u64;
    loop {
        let w = c.mem.read_u32(p).unwrap_or(0);
        if w == 0 {
            if dst != 0 && n < len {
                let _ = c.mem.write(dst + n, &[0]);
                if indirect {
                    let _ = c.mem.write_u64(src, 0);
                }
            }
            return c.ret(n);
        }
        let Some(b) = encode(w) else {
            if dst != 0 && indirect {
                let _ = c.mem.write_u64(src, p);
            }
            c.set_errno(EILSEQ);
            return c.ret(BAD);
        };
        if dst != 0 {
            if n + b.len() as u64 > len {
                break;
            }
            let _ = c.mem.write(dst + n, &b);
        }
        n += b.len() as u64;
        p += 4;
    }
    if indirect {
        let _ = c.mem.write_u64(src, p);
    }
    c.ret(n)
}

fn wcrtomb(c: &mut Call, s: u64, w: u64) -> Act {
    if s == 0 {
        return c.ret(1);
    }
    match encode(w as u32) {
        Some(b) => {
            let _ = c.mem.write(s, &b);
            c.ret(b.len() as u64)
        }
        None => {
            c.set_errno(EILSEQ);
            c.ret(BAD)
        }
    }
}

// ---------------------------------------------------------------- strtod

/// Length of the longest prefix of `s` (after leading space) that is a decimal or special
/// floating-point number, and its value.
pub(crate) fn parse_double(s: &[u8]) -> (usize, f64) {
    let start = s.iter().take_while(|b| b" \t\n\x0b\x0c\r".contains(b)).count();
    let t = &s[start..];
    let mut i = 0;
    let neg = matches!(t.first(), Some(b'-'));
    if matches!(t.first(), Some(b'-' | b'+')) {
        i = 1;
    }
    let lower: Vec<u8> = t[i..t.len().min(i + 8)].iter().map(|b| b.to_ascii_lowercase()).collect();
    let sign = if neg { -1.0 } else { 1.0 };
    if lower.starts_with(b"infinity") {
        return (start + i + 8, sign * f64::INFINITY);
    }
    if lower.starts_with(b"inf") {
        return (start + i + 3, sign * f64::INFINITY);
    }
    if lower.starts_with(b"nan") {
        return (start + i + 3, f64::NAN);
    }
    if lower.starts_with(b"0x") {
        return parse_hex(t, i, neg).map(|(l, v)| (start + l, v)).unwrap_or((0, 0.0));
    }
    let digits = |from: usize| t[from..].iter().take_while(|b| b.is_ascii_digit()).count();
    let int = digits(i);
    let mut j = i + int;
    let mut frac = 0;
    if t.get(j) == Some(&b'.') {
        frac = digits(j + 1);
        if int + frac > 0 {
            j += 1 + frac;
        }
    }
    if int + frac == 0 {
        return (0, 0.0);
    }
    if matches!(t.get(j), Some(b'e' | b'E')) {
        let mut k = j + 1;
        if matches!(t.get(k), Some(b'-' | b'+')) {
            k += 1;
        }
        let e = digits(k);
        if e > 0 {
            j = k + e;
        }
    }
    let text = core::str::from_utf8(&t[..j]).unwrap_or("0");
    let text = text.strip_prefix('+').unwrap_or(text);
    (start + j, text.parse::<f64>().unwrap_or(0.0))
}

fn parse_hex(t: &[u8], i: usize, neg: bool) -> Option<(usize, f64)> {
    let mut j = i + 2;
    let (mut m, mut exp, mut any) = (0u64, 0i32, false);
    let mut frac = false;
    while let Some(&b) = t.get(j) {
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            b'.' if !frac => {
                frac = true;
                j += 1;
                continue;
            }
            _ => break,
        };
        any = true;
        if m >> 59 == 0 {
            m = m << 4 | d as u64;
            if frac {
                exp -= 4;
            }
        } else if !frac {
            exp += 4;
        }
        j += 1;
    }
    if !any {
        return Some((i + 1, if neg { -0.0 } else { 0.0 })); // just the "0"
    }
    if matches!(t.get(j), Some(b'p' | b'P')) {
        let mut k = j + 1;
        let en = matches!(t.get(k), Some(b'-'));
        if matches!(t.get(k), Some(b'-' | b'+')) {
            k += 1;
        }
        let n = t[k..].iter().take_while(|b| b.is_ascii_digit()).count();
        if n > 0 {
            let e: i32 = core::str::from_utf8(&t[k..k + n]).ok()?.parse().unwrap_or(10000);
            exp += if en { -e } else { e };
            j = k + n;
        }
    }
    let mut v = m as f64;
    let mut e = exp;
    while e > 0 {
        v *= 2.0;
        e -= 1;
    }
    while e < 0 {
        v /= 2.0;
        e += 1;
    }
    Some((j, if neg { -v } else { v }))
}

fn strtod(c: &mut Call, single: bool) -> Act {
    let (p, end) = (c.arg(), c.arg());
    let s = c.cstr(p);
    let (n, mut v) = parse_double(&s);
    if single {
        v = v as f32 as f64;
    }
    if v.is_infinite() && n > 0 && !s.iter().any(|b| b.eq_ignore_ascii_case(&b'i')) {
        c.set_errno(34); // ERANGE
    }
    if end != 0 {
        let _ = c.mem.write_u64(end, p + n as u64);
    }
    if single {
        // float results are in the low 32 bits of xmm0 / s0.
        c.abi.ret_f64(f64::from_bits((v as f32).to_bits() as u64));
    } else {
        c.abi.ret_f64(v);
    }
    Act::Done
}

fn zero(mem: &mut UserMem, p: u64, n: u64) {
    let _ = mem.write(p, &alloc::vec![0u8; n as usize]);
}

pub static FUNCS: &[(&str, Func)] = &[
    f!("_mbrtowc", |c| { let (w, s, n, ps) = (c.arg(), c.arg(), c.arg(), c.arg()); mbrtowc(c, w, s, n, ps) }),
    f!("_mbtowc", |c| { let (w, s, n) = (c.arg(), c.arg(), c.arg()); mbrtowc(c, w, s, n, 0) }),
    f!("_mbrlen", |c| { let (s, n, ps) = (c.arg(), c.arg(), c.arg()); mbrtowc(c, 0, s, n, ps) }),
    f!("_mblen", |c| { let (s, n) = (c.arg(), c.arg()); mbrtowc(c, 0, s, n, 0) }),
    f!("_mbsinit", |c| { let ps = c.arg(); let v = if ps == 0 { 1 } else { (c.mem.read_u32(ps).unwrap_or(0) & 0xff == 0) as u64 }; c.ret(v) }),
    f!("_wcrtomb", |c| { let (s, w) = (c.arg(), c.arg()); wcrtomb(c, s, w) }),
    f!("_wctomb", |c| { let (s, w) = (c.arg(), c.arg()); if s == 0 { c.ret(0) } else { wcrtomb(c, s, w) } }),
    f!("_btowc", |c| { let b = c.arg() as i32; c.ret(if (0..0x80).contains(&b) { b as u64 } else { u32::MAX as u64 }) }),
    f!("_wctob", |c| { let w = c.arg() as u32; c.ret(if w < 0x80 { w as u64 } else { u64::MAX }) }),
    f!("_mbsrtowcs", |c| { let (d, s, n) = (c.arg(), c.arg(), c.arg()); mbs_to_wcs(c, d, s, n, true) }),
    f!("_mbstowcs", |c| { let (d, s, n) = (c.arg(), c.arg(), c.arg()); mbs_to_wcs(c, d, s, n, false) }),
    f!("_wcsrtombs", |c| { let (d, s, n) = (c.arg(), c.arg(), c.arg()); wcs_to_mbs(c, d, s, n, true) }),
    f!("_wcstombs", |c| { let (d, s, n) = (c.arg(), c.arg(), c.arg()); wcs_to_mbs(c, d, s, n, false) }),
    f!("_strtod", |c| strtod(c, false)),
    f!("_strtof", |c| strtod(c, true)),
    f!("_atof", |c| {
        let p = c.arg();
        let s = c.cstr(p);
        let v = parse_double(&s).1;
        c.abi.ret_f64(v);
        Act::Done
    }),
    f!("_getgrouplist_2", |c| {
        let (_name, base, out) = (c.arg(), c.arg(), c.arg());
        let a = c.st.heap.alloc(c.mem, 8);
        let _ = c.mem.write_u32(a, base as u32);
        let _ = c.mem.write_u32(a + 4, 20);
        let _ = c.mem.write_u64(out, a);
        c.ret(if base as u32 == 20 { 1 } else { 2 })
    }),
    f!("_getgrouplist", |c| {
        let (_name, base, groups, np) = (c.arg(), c.arg(), c.arg(), c.arg());
        let cap = c.mem.read_u32(np).unwrap_or(0);
        let list: &[u32] = if base as u32 == 20 { &[20] } else { &[base as u32, 20] };
        for (i, g) in list.iter().enumerate().take(cap as usize) {
            let _ = c.mem.write_u32(groups + i as u64 * 4, *g);
        }
        let _ = c.mem.write_u32(np, list.len() as u32);
        c.ret(if (cap as usize) < list.len() { u64::MAX } else { 0 })
    }),
    // BSM auditing is off: no audit user, session or terminal.
    f!("_getaudit_addr", |c| { let (p, n) = (c.arg(), c.arg()); zero(c.mem, p, n.min(512)); c.ret(0) }),
    f!("_getauid", |c| { let p = c.arg(); let _ = c.mem.write_u32(p, u32::MAX); c.ret(0) }),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_and_doubles() {
        assert_eq!(decode("é".as_bytes()), Ok((0xe9, 2)));
        assert_eq!(decode(&[0xe4, 0xb8]), Err(true));
        assert_eq!(decode(&[0xff]), Err(false));
        assert_eq!(parse_double(b"  3.25e2xyz"), (8, 325.0));
        assert_eq!(parse_double(b"-.5"), (3, -0.5));
        assert_eq!(parse_double(b"0x1.8p1"), (7, 3.0));
        assert_eq!(parse_double(b"abc").0, 0);
        assert!(parse_double(b"-inf").1.is_infinite());
        assert_eq!(parse_double(b"1e"), (1, 1.0));
    }
}
