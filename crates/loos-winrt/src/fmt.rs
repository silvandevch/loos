//! printf-family formatting engine (the C runtime's `%` conversions) over an abstract argument source.
//!
//! Output is a vector of code points: narrow strings are bytes mapped 1:1 to U+0000..U+00FF, wide strings are UTF-16
//! units; the caller encodes the result back. Floating point is converted with exact binary-to-decimal expansion.

use alloc::string::String;
use alloc::vec::Vec;

/// Where `%s`/`%S` strings and varargs come from.
pub trait Args {
    /// Next 64-bit vararg slot (integers, pointers, and doubles as raw bits).
    fn next(&mut self) -> u64;
    /// Reads a NUL-terminated string at `ptr` (`wide`: UTF-16), at most `max` characters.
    fn string(&mut self, ptr: u64, wide: bool, max: usize) -> Vec<u32>;
}

#[derive(Default, Clone, Copy)]
struct Spec {
    minus: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
    width: usize,
    prec: Option<usize>,
}

fn pad(out: &mut Vec<u32>, body: &[u32], sp: &Spec, zero_ok: bool, sign_len: usize) {
    let len = body.len();
    if len >= sp.width {
        out.extend_from_slice(body);
    } else if sp.minus {
        out.extend_from_slice(body);
        out.extend(core::iter::repeat(0x20).take(sp.width - len));
    } else if sp.zero && zero_ok {
        out.extend_from_slice(&body[..sign_len]);
        out.extend(core::iter::repeat(0x30).take(sp.width - len));
        out.extend_from_slice(&body[sign_len..]);
    } else {
        out.extend(core::iter::repeat(0x20).take(sp.width - len));
        out.extend_from_slice(body);
    }
}

fn s2v(s: &str) -> Vec<u32> {
    s.chars().map(|c| c as u32).collect()
}

fn utoa(mut v: u64, base: u64, upper: bool) -> Vec<u32> {
    if v == 0 {
        return alloc::vec![0x30];
    }
    let mut d = Vec::new();
    while v != 0 {
        let r = (v % base) as u32;
        d.push(if r < 10 { 0x30 + r } else { (if upper { 0x41 } else { 0x61 }) + r - 10 });
        v /= base;
    }
    d.reverse();
    d
}

/// Exact decimal digits of a finite, positive double: `(digits, point)` meaning `0.d1d2d3... * 10^point`... expressed as
/// the integer-part length: value = digits[..point] . digits[point..]. Leading zeros are not produced for the integer part.
fn decimal(a: f64) -> (Vec<u8>, usize) {
    let bits = a.to_bits();
    let (e, frac) = (((bits >> 52) & 0x7ff) as i32, bits & ((1 << 52) - 1));
    let (m, exp) = if e == 0 { (frac, -1074) } else { (frac | 1 << 52, e - 1075) };
    let mut digits: Vec<u8> = Vec::new();
    let mut int_len;
    if exp >= 0 {
        // Integer: m << exp; beyond u128 range approximate through repeated doubling in decimal.
        if exp <= 74 {
            let v = u128::from(m) << exp;
            digits = v.to_string_digits();
        } else {
            let mut dec: Vec<u8> = m.to_string_digits_u64();
            for _ in 0..exp {
                let mut carry = 0;
                for d in dec.iter_mut().rev() {
                    let x = *d * 2 + carry;
                    *d = x % 10;
                    carry = x / 10;
                }
                if carry > 0 {
                    dec.insert(0, carry);
                }
            }
            digits = dec;
        }
        int_len = digits.len();
    } else {
        let k = (-exp) as u32;
        if k > 120 {
            // Too small for exact u128 expansion: integer part 0, fraction digits from a scaled estimate.
            digits.push(0);
            int_len = 1;
            let mut x = a;
            for _ in 0..60 {
                x *= 10.0;
                let d = x as u32 % 10;
                digits.push(d as u8);
                x -= (x as u64) as f64;
            }
        } else {
            let (ip, mut r) = (u128::from(m) >> k, u128::from(m) & ((1u128 << k) - 1));
            digits = ip.to_string_digits();
            int_len = digits.len();
            for _ in 0..(k + 1).min(80) {
                r *= 10;
                digits.push((r >> k) as u8);
                r &= (1u128 << k) - 1;
                if r == 0 {
                    break;
                }
            }
        }
    }
    if int_len == 0 {
        int_len = 1;
        digits.insert(0, 0);
    }
    (digits, int_len)
}

trait Digits {
    fn to_string_digits(self) -> Vec<u8>;
}
impl Digits for u128 {
    fn to_string_digits(mut self) -> Vec<u8> {
        if self == 0 {
            return alloc::vec![0];
        }
        let mut d = Vec::new();
        while self != 0 {
            d.push((self % 10) as u8);
            self /= 10;
        }
        d.reverse();
        d
    }
}
trait DigitsU64 {
    fn to_string_digits_u64(self) -> Vec<u8>;
}
impl DigitsU64 for u64 {
    fn to_string_digits_u64(self) -> Vec<u8> {
        u128::from(self).to_string_digits()
    }
}

/// Rounds a digit string at `keep` digits (half away from zero); returns whether a carry added a leading digit.
fn round_at(d: &mut Vec<u8>, keep: usize) -> bool {
    if keep >= d.len() {
        return false;
    }
    let up = d[keep] >= 5;
    d.truncate(keep);
    if !up {
        return false;
    }
    for x in d.iter_mut().rev() {
        if *x == 9 {
            *x = 0;
        } else {
            *x += 1;
            return false;
        }
    }
    d.insert(0, 1);
    true
}

/// Formats a double for `%f %e %g %a`-style conversions (`conv` lower case; `upper` for the letters).
pub fn float(v: f64, conv: u8, prec: Option<usize>, alt: bool, upper: bool) -> String {
    let a = if v.is_sign_negative() { -v } else { v };
    if v != v {
        return String::from(if upper { "NAN" } else { "nan" });
    }
    if a == f64::INFINITY {
        return String::from(if upper { "INF" } else { "inf" });
    }
    let prec = prec.unwrap_or(6);
    let (mut digits, mut int_len) = if a == 0.0 { (alloc::vec![0u8], 1usize) } else { decimal(a) };
    let mut s = String::new();
    let letter = |c: char| if upper { c.to_ascii_uppercase() } else { c };
    // Decimal exponent of the first significant digit.
    let first_nz = digits.iter().position(|d| *d != 0);
    let exp10 = match first_nz {
        Some(p) => int_len as i32 - 1 - p as i32,
        None => 0,
    };
    let mut emit_e = |s: &mut String, digs: &[u8], e: i32, prec: usize, alt: bool| {
        s.push((b'0' + digs.first().copied().unwrap_or(0)) as char);
        if prec > 0 || alt {
            s.push('.');
        }
        for i in 0..prec {
            s.push((b'0' + digs.get(1 + i).copied().unwrap_or(0)) as char);
        }
        s.push(letter('e'));
        s.push(if e < 0 { '-' } else { '+' });
        let ae = e.unsigned_abs();
        if ae < 10 {
            s.push('0');
        }
        for c in alloc::format!("{ae}").chars() {
            s.push(c);
        }
    };
    let sig_for_e = |digits: &Vec<u8>, first: usize, n: usize| -> (Vec<u8>, bool) {
        let mut sig: Vec<u8> = digits[first..].to_vec();
        let carry = round_at(&mut sig, n);
        (sig, carry)
    };
    match conv {
        b'f' => {
            // Round at int_len + prec digits.
            let keep = int_len + prec;
            while digits.len() < keep {
                digits.push(0);
            }
            if round_at(&mut digits, keep) {
                int_len += 1;
            }
            for (i, d) in digits.iter().enumerate() {
                if i == int_len {
                    s.push('.');
                }
                s.push((b'0' + d) as char);
            }
            if prec == 0 && alt {
                s.push('.');
            }
        }
        b'e' => {
            let first = first_nz.unwrap_or(0);
            let (sig, carry) = sig_for_e(&digits, first, prec + 1);
            let e = if first_nz.is_none() { 0 } else { exp10 + i32::from(carry) };
            let sig = if first_nz.is_none() { alloc::vec![0] } else { sig };
            emit_e(&mut s, &sig, e, prec, alt);
        }
        _ => {
            // %g: precision P (0 -> 1); use %e when exp < -4 or exp >= P, trimming trailing zeros unless `alt`.
            let p = if prec == 0 { 1 } else { prec };
            let first = first_nz.unwrap_or(0);
            let (mut sig, carry) = sig_for_e(&digits, first, p);
            let e = if first_nz.is_none() { 0 } else { exp10 + i32::from(carry) };
            if first_nz.is_none() {
                sig = alloc::vec![0];
            }
            while sig.len() < p {
                sig.push(0);
            }
            if e < -4 || e >= p as i32 {
                let mut n = p - 1;
                if !alt {
                    while n > 0 && sig.get(n).copied().unwrap_or(0) == 0 {
                        n -= 1;
                    }
                }
                emit_e(&mut s, &sig, e, n, alt);
            } else if e >= 0 {
                let ip = e as usize + 1;
                let mut frac_n = p.saturating_sub(ip);
                if !alt {
                    while frac_n > 0 && sig.get(ip + frac_n - 1).copied().unwrap_or(0) == 0 {
                        frac_n -= 1;
                    }
                }
                for i in 0..ip {
                    s.push((b'0' + sig.get(i).copied().unwrap_or(0)) as char);
                }
                if frac_n > 0 || alt {
                    s.push('.');
                }
                for i in 0..frac_n {
                    s.push((b'0' + sig.get(ip + i).copied().unwrap_or(0)) as char);
                }
            } else {
                let zeros = (-e - 1) as usize;
                let mut frac_n = p + zeros;
                if !alt {
                    while frac_n > zeros && sig.get(frac_n - zeros - 1).copied().unwrap_or(0) == 0 {
                        frac_n -= 1;
                    }
                }
                s.push('0');
                s.push('.');
                for _ in 0..zeros {
                    s.push('0');
                }
                for i in 0..frac_n - zeros {
                    s.push((b'0' + sig.get(i).copied().unwrap_or(0)) as char);
                }
            }
        }
    }
    s
}

/// Formats `fmt` (code points) with arguments from `args`; `wide_fmt` selects how `%s` defaults (narrow vs wide) and
/// swaps the meaning of `%s`/`%ls` as the wide printf family does.
pub fn format(fmt: &[u32], wide_fmt: bool, args: &mut dyn Args) -> Vec<u32> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != 0x25 {
            out.push(c);
            continue;
        }
        let mut sp = Spec::default();
        // flags
        while let Some(&f) = fmt.get(i) {
            match f as u8 as char {
                '-' => sp.minus = true,
                '+' => sp.plus = true,
                ' ' => sp.space = true,
                '#' => sp.alt = true,
                '0' => sp.zero = true,
                _ => break,
            }
            if f > 0x7f {
                break;
            }
            i += 1;
        }
        // width
        if fmt.get(i) == Some(&0x2a) {
            let w = args.next() as i32;
            if w < 0 {
                sp.minus = true;
            }
            sp.width = w.unsigned_abs() as usize;
            i += 1;
        } else {
            while let Some(d) = fmt.get(i).and_then(|c| char::from_u32(*c)).and_then(|c| c.to_digit(10)) {
                sp.width = sp.width * 10 + d as usize;
                i += 1;
            }
        }
        sp.width = sp.width.min(4096);
        // precision
        if fmt.get(i) == Some(&0x2e) {
            i += 1;
            if fmt.get(i) == Some(&0x2a) {
                let p = args.next() as i32;
                sp.prec = if p < 0 { None } else { Some(p as usize) };
                i += 1;
            } else {
                let mut p = 0usize;
                while let Some(d) = fmt.get(i).and_then(|c| char::from_u32(*c)).and_then(|c| c.to_digit(10)) {
                    p = p * 10 + d as usize;
                    i += 1;
                }
                sp.prec = Some(p.min(4096));
            }
        }
        // length
        let (mut long, mut half, mut size64, mut wide_mod) = (false, 0u8, false, false);
        loop {
            match fmt.get(i).copied().and_then(char::from_u32) {
                Some('l') => {
                    if long {
                        size64 = true;
                    }
                    long = true;
                    wide_mod = true;
                }
                Some('h') => half += 1,
                Some('z' | 'j' | 't') => size64 = true,
                Some('L') => {}
                Some('w') => wide_mod = true,
                Some('I') => {
                    // I, I32, I64
                    if fmt.get(i + 1) == Some(&0x36) && fmt.get(i + 2) == Some(&0x34) {
                        size64 = true;
                        i += 2;
                    } else if fmt.get(i + 1) == Some(&0x33) && fmt.get(i + 2) == Some(&0x32) {
                        i += 2;
                    } else {
                        size64 = true;
                    }
                }
                _ => break,
            }
            i += 1;
        }
        let Some(&cv) = fmt.get(i) else { break };
        i += 1;
        let conv = char::from_u32(cv).unwrap_or('?');
        match conv {
            '%' => out.push(0x25),
            'c' | 'C' => {
                let ch = args.next() as u32;
                let ch = if conv == 'c' && half == 0 && !wide_mod && !wide_fmt || conv == 'c' && half > 0 { ch & 0xff } else { ch & 0xffff };
                pad(&mut out, &[ch], &sp, false, 0);
            }
            's' | 'S' => {
                let p = args.next();
                // %s: narrow in narrow printf, wide in wide printf; %S/%ls/%hs the opposite (or explicit).
                let wide = match (conv, long || wide_mod, half > 0) {
                    (_, true, _) => true,
                    (_, _, true) => false,
                    ('s', _, _) => wide_fmt,
                    _ => !wide_fmt,
                };
                let body: Vec<u32> = if p == 0 { s2v("(null)") } else { args.string(p, wide, sp.prec.unwrap_or(usize::MAX)) };
                let body = match sp.prec {
                    Some(n) if body.len() > n => body[..n].to_vec(),
                    _ => body,
                };
                pad(&mut out, &body, &sp, false, 0);
            }
            'd' | 'i' => {
                let raw = args.next();
                let v: i64 = if size64 { raw as i64 } else if half == 1 { i64::from(raw as i16) } else if half >= 2 { i64::from(raw as i8) } else { i64::from(raw as i32) };
                let mut digs = utoa(v.unsigned_abs(), 10, false);
                if let Some(p) = sp.prec {
                    while digs.len() < p {
                        digs.insert(0, 0x30);
                    }
                    if p == 0 && v == 0 {
                        digs.clear();
                    }
                }
                let mut body = Vec::new();
                if v < 0 {
                    body.push(0x2d);
                } else if sp.plus {
                    body.push(0x2b);
                } else if sp.space {
                    body.push(0x20);
                }
                let sl = body.len();
                body.extend(digs);
                pad(&mut out, &body, &sp, sp.prec.is_none(), sl);
            }
            'u' | 'x' | 'X' | 'o' => {
                let raw = args.next();
                let v: u64 = if size64 { raw } else if half == 1 { u64::from(raw as u16) } else if half >= 2 { u64::from(raw as u8) } else { u64::from(raw as u32) };
                let base = match conv { 'u' => 10, 'o' => 8, _ => 16 };
                let mut digs = utoa(v, base, conv == 'X');
                if let Some(p) = sp.prec {
                    while digs.len() < p {
                        digs.insert(0, 0x30);
                    }
                    if p == 0 && v == 0 {
                        digs.clear();
                    }
                }
                let mut body = Vec::new();
                if sp.alt && v != 0 {
                    match conv {
                        'x' => body.extend([0x30, 0x78]),
                        'X' => body.extend([0x30, 0x58]),
                        'o' if digs.first() != Some(&0x30) => body.push(0x30),
                        _ => {}
                    }
                }
                let sl = body.len();
                body.extend(digs);
                pad(&mut out, &body, &sp, sp.prec.is_none(), sl);
            }
            'p' | 'P' => {
                let v = args.next();
                let mut digs = utoa(v, 16, conv == 'P');
                while digs.len() < 16 {
                    digs.insert(0, 0x30);
                }
                pad(&mut out, &digs, &sp, false, 0);
            }
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                let v = f64::from_bits(args.next());
                let lower = conv.to_ascii_lowercase() as u8;
                let body_s = float(v, lower, sp.prec, sp.alt, conv.is_ascii_uppercase());
                let mut body = Vec::new();
                if v.is_sign_negative() && v == v {
                    body.push(0x2d);
                } else if sp.plus {
                    body.push(0x2b);
                } else if sp.space {
                    body.push(0x20);
                }
                let sl = body.len();
                body.extend(s2v(&body_s));
                pad(&mut out, &body, &sp, v.is_finite(), sl);
            }
            'n' => {
                let _ = args.next(); // %n is not supported (it writes through a pointer): consume and ignore
            }
            other => {
                out.push(0x25);
                out.push(other as u32);
            }
        }
    }
    out
}
