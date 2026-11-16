//! Smaller libc pieces: `strtonum`, wide-string helpers, `fnmatch`, `strptime`, `fmod`,
//! `fmtcheck`, `unvis`, `devname`, `mkfifo` and `sys_signame`.

#[allow(unused_imports)]
use alloc::{vec, vec::Vec};
use crate::libsys::{Act, Call, Func, Post};
use crate::loader::SHIM_DATA;
use crate::posix::ret_str;

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

/// `sys_signame`: 32 pointers, then the names.
pub(crate) const D_SIGNAME: u64 = 0x7800;
const SIGNAMES: [&str; 32] = [
    "Signal 0", "hup", "int", "quit", "ill", "trap", "abrt", "emt", "fpe", "kill", "bus", "segv", "sys", "pipe", "alrm", "term",
    "urg", "stop", "tstp", "cont", "chld", "ttin", "ttou", "io", "xcpu", "xfsz", "vtalrm", "prof", "winch", "info", "usr1", "usr2",
];

pub fn init_data(d: &mut [u8]) {
    let base = D_SIGNAME as usize;
    let mut at = base + 32 * 8;
    for (i, n) in SIGNAMES.iter().enumerate() {
        d[base + i * 8..base + i * 8 + 8].copy_from_slice(&(SHIM_DATA + at as u64).to_le_bytes());
        d[at..at + n.len()].copy_from_slice(n.as_bytes());
        at += n.len() + 1;
    }
}

// ---------------------------------------------------------------- fnmatch

const FNM_NOESCAPE: u64 = 1;
const FNM_PATHNAME: u64 = 2;
const FNM_PERIOD: u64 = 4;
const FNM_LEADING_DIR: u64 = 8;
const FNM_CASEFOLD: u64 = 16;

/// Does `s` match the shell pattern `p`?
pub(crate) fn fnmatch(p: &[u8], s: &[u8], flags: u64) -> bool {
    fn lower(b: u8, f: u64) -> u8 {
        if f & FNM_CASEFOLD != 0 { b.to_ascii_lowercase() } else { b }
    }
    fn leading_period(s: &[u8], si: usize, f: u64) -> bool {
        f & FNM_PERIOD != 0 && s.get(si) == Some(&b'.') && (si == 0 || (f & FNM_PATHNAME != 0 && s[si - 1] == b'/'))
    }
    /// `[...]` at p[pi]: Some((matched, end index)) or None when there is no closing `]`.
    fn class(p: &[u8], mut pi: usize, c: u8, f: u64) -> Option<(bool, usize)> {
        pi += 1;
        let neg = matches!(p.get(pi), Some(b'!' | b'^'));
        if neg {
            pi += 1;
        }
        let mut ok = false;
        let mut first = true;
        loop {
            let mut a = *p.get(pi)?;
            if a == b']' && !first {
                return Some((ok != neg, pi + 1));
            }
            first = false;
            if a == b'\\' && f & FNM_NOESCAPE == 0 {
                pi += 1;
                a = *p.get(pi)?;
            }
            if p.get(pi + 1) == Some(&b'-') && p.get(pi + 2).is_some_and(|&b| b != b']') {
                let mut z = p[pi + 2];
                let mut skip = 3;
                if z == b'\\' && f & FNM_NOESCAPE == 0 {
                    z = *p.get(pi + 3)?;
                    skip = 4;
                }
                if lower(a, f) <= lower(c, f) && lower(c, f) <= lower(z, f) {
                    ok = true;
                }
                pi += skip;
            } else {
                if lower(a, f) == lower(c, f) {
                    ok = true;
                }
                pi += 1;
            }
        }
    }
    fn go(p: &[u8], mut pi: usize, s: &[u8], mut si: usize, f: u64) -> bool {
        while pi < p.len() {
            match p[pi] {
                b'*' => {
                    while p.get(pi) == Some(&b'*') {
                        pi += 1;
                    }
                    if leading_period(s, si, f) {
                        return false;
                    }
                    if pi == p.len() {
                        return f & FNM_PATHNAME == 0 || f & FNM_LEADING_DIR != 0 || !s[si..].contains(&b'/');
                    }
                    for k in si..=s.len() {
                        if go(p, pi, s, k, f) {
                            return true;
                        }
                        if k < s.len() && s[k] == b'/' && f & FNM_PATHNAME != 0 {
                            break;
                        }
                    }
                    return false;
                }
                b'?' => {
                    let Some(&c) = s.get(si) else { return false };
                    if (c == b'/' && f & FNM_PATHNAME != 0) || leading_period(s, si, f) {
                        return false;
                    }
                    pi += 1;
                    si += 1;
                }
                b'[' => {
                    let Some(&c) = s.get(si) else { return false };
                    if (c == b'/' && f & FNM_PATHNAME != 0) || leading_period(s, si, f) {
                        return false;
                    }
                    match class(p, pi, c, f) {
                        Some((true, end)) => {
                            pi = end;
                            si += 1;
                        }
                        Some((false, _)) => return false,
                        None if c == b'[' => {
                            pi += 1;
                            si += 1;
                        }
                        None => return false,
                    }
                }
                mut c => {
                    if c == b'\\' && f & FNM_NOESCAPE == 0 && pi + 1 < p.len() {
                        pi += 1;
                        c = p[pi];
                    }
                    if s.get(si).map(|&b| lower(b, f)) != Some(lower(c, f)) {
                        return false;
                    }
                    pi += 1;
                    si += 1;
                }
            }
        }
        si == s.len() || (f & FNM_LEADING_DIR != 0 && s[si] == b'/')
    }
    go(p, 0, s, 0, flags)
}

// ---------------------------------------------------------------- strptime

const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
const DAYS: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];

/// Parse `s` by `fmt` into the `struct tm` fields (sec, min, hour, mday, mon, year, wday, yday);
/// returns the number of bytes consumed, or None.
pub(crate) fn strptime(s: &[u8], fmt: &[u8], tm: &mut [i32; 9]) -> Option<usize> {
    let mut si = 0;
    let mut fi = 0;
    let mut pm: Option<bool> = None;
    let skip_space = |si: &mut usize| {
        while s.get(*si).is_some_and(|b| b.is_ascii_whitespace()) {
            *si += 1;
        }
    };
    let num = |si: &mut usize, max: usize| -> Option<i32> {
        let neg = s.get(*si) == Some(&b'-');
        let start = *si + neg as usize;
        let n = s[start.min(s.len())..].iter().take(max).take_while(|b| b.is_ascii_digit()).count();
        if n == 0 {
            return None;
        }
        let v: i32 = core::str::from_utf8(&s[start..start + n]).ok()?.parse().ok()?;
        *si = start + n;
        Some(if neg { -v } else { v })
    };
    let name = |si: &mut usize, list: &[&str]| -> Option<i32> {
        let rest: Vec<u8> = s[*si..].iter().take(12).map(|b| b.to_ascii_lowercase()).collect();
        for (i, n) in list.iter().enumerate() {
            if rest.starts_with(n.as_bytes()) {
                *si += n.len();
                return Some(i as i32);
            }
        }
        for (i, n) in list.iter().enumerate() {
            if rest.starts_with(&n.as_bytes()[..3]) {
                *si += 3;
                return Some(i as i32);
            }
        }
        None
    };
    while fi < fmt.len() {
        let c = fmt[fi];
        if c.is_ascii_whitespace() {
            skip_space(&mut si);
            fi += 1;
            continue;
        }
        if c != b'%' {
            if s.get(si) != Some(&c) {
                return None;
            }
            si += 1;
            fi += 1;
            continue;
        }
        fi += 1;
        let mut conv = *fmt.get(fi)?;
        if conv == b'E' || conv == b'O' {
            fi += 1;
            conv = *fmt.get(fi)?;
        }
        fi += 1;
        if conv != b'%' && conv != b'n' && conv != b't' {
            skip_space(&mut si);
        }
        match conv {
            b'%' => {
                if s.get(si) != Some(&b'%') {
                    return None;
                }
                si += 1;
            }
            b'n' | b't' => skip_space(&mut si),
            b'Y' => tm[5] = num(&mut si, 4)? - 1900,
            b'C' => tm[5] = num(&mut si, 2)? * 100 - 1900 + tm[5].rem_euclid(100),
            b'y' => {
                let y = num(&mut si, 2)?;
                tm[5] = if y < 69 { y + 100 } else { y };
            }
            b'm' => tm[4] = num(&mut si, 2)? - 1,
            b'd' | b'e' => tm[3] = num(&mut si, 2)?,
            b'H' | b'k' => tm[2] = num(&mut si, 2)?,
            b'I' | b'l' => tm[2] = num(&mut si, 2)? % 12,
            b'M' => tm[1] = num(&mut si, 2)?,
            b'S' => tm[0] = num(&mut si, 2)?,
            b'j' => tm[7] = num(&mut si, 3)? - 1,
            b'w' => tm[6] = num(&mut si, 1)?,
            b'u' => tm[6] = num(&mut si, 1)? % 7,
            b'b' | b'B' | b'h' => tm[4] = name(&mut si, &MONTHS)?,
            b'a' | b'A' => tm[6] = name(&mut si, &DAYS)?,
            b'p' => {
                let r: Vec<u8> = s[si..].iter().take(2).map(|b| b.to_ascii_lowercase()).collect();
                pm = Some(match r.as_slice() {
                    b"am" => false,
                    b"pm" => true,
                    _ => return None,
                });
                si += 2;
            }
            b'T' => si += strptime(&s[si..], b"%H:%M:%S", tm)?,
            b'R' => si += strptime(&s[si..], b"%H:%M", tm)?,
            b'D' => si += strptime(&s[si..], b"%m/%d/%y", tm)?,
            b'F' => si += strptime(&s[si..], b"%Y-%m-%d", tm)?,
            b'r' => si += strptime(&s[si..], b"%I:%M:%S %p", tm)?,
            b'c' => si += strptime(&s[si..], b"%a %b %e %H:%M:%S %Y", tm)?,
            b'x' => si += strptime(&s[si..], b"%m/%d/%y", tm)?,
            b'X' => si += strptime(&s[si..], b"%H:%M:%S", tm)?,
            b'z' => {
                let sign = match s.get(si) {
                    Some(b'+') => 1,
                    Some(b'-') => -1,
                    Some(b'Z') => {
                        si += 1;
                        continue;
                    }
                    _ => return None,
                };
                si += 1;
                num(&mut si, 4)?;
                let _ = sign;
            }
            b'Z' => {
                while s.get(si).is_some_and(|b| b.is_ascii_alphabetic()) {
                    si += 1;
                }
            }
            b's' => {
                let t = num(&mut si, 20)? as i64;
                let days = t.div_euclid(86400);
                let r = t.rem_euclid(86400);
                tm[0] = (r % 60) as i32;
                tm[1] = (r / 60 % 60) as i32;
                tm[2] = (r / 3600) as i32;
                let (y, m, d) = civil(days);
                tm[5] = y - 1900;
                tm[4] = m - 1;
                tm[3] = d;
                tm[6] = (days + 4).rem_euclid(7) as i32;
            }
            _ => return None,
        }
    }
    if let Some(p) = pm {
        tm[2] = tm[2] % 12 + if p { 12 } else { 0 };
    }
    Some(si)
}

/// Days since 1970-01-01 → (year, month 1-12, day).
fn civil(z: i64) -> (i32, i32, i32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((yoe + era * 400 + (m <= 2) as i64) as i32, m as i32, d as i32)
}

/// Exact floating-point remainder (sign of `x`), by long division on the significands.
pub(crate) fn fmod(x: f64, y: f64) -> f64 {
    if y == 0.0 || x.is_infinite() || x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let (ax, ay) = (x.abs(), y.abs());
    if ax < ay || y.is_infinite() {
        return x;
    }
    // (significand with the implicit bit, unbiased exponent), normalizing subnormals.
    let split = |v: f64| {
        let b = v.to_bits();
        let e = (b >> 52 & 0x7ff) as i32;
        let m = b & ((1 << 52) - 1);
        if e == 0 {
            let sh = m.leading_zeros() as i32 - 11;
            (m << sh, -1022 - sh)
        } else {
            (m | 1 << 52, e - 1023)
        }
    };
    let (mut mx, ex) = split(ax);
    let (my, ey) = split(ay);
    for _ in 0..ex - ey {
        if mx >= my {
            mx -= my;
        }
        mx <<= 1;
    }
    if mx >= my {
        mx -= my;
    }
    if mx == 0 {
        return 0.0 * x;
    }
    // mx · 2^(ey-52) is exact; build it from the bits.
    let sh = mx.leading_zeros() as i32 - 11;
    mx <<= sh;
    let e = ey - sh;
    let r = if e >= -1022 {
        f64::from_bits(((e + 1023) as u64) << 52 | (mx & ((1 << 52) - 1)))
    } else {
        f64::from_bits(mx >> (-1022 - e))
    };
    if x < 0.0 { -r } else { r }
}

// ---------------------------------------------------------------- fmtcheck, unvis

/// The conversions of a printf format (length modifiers + conversion character).
fn conversions(f: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < f.len() {
        if f[i] != b'%' {
            i += 1;
            continue;
        }
        i += 1;
        if f.get(i) == Some(&b'%') {
            i += 1;
            continue;
        }
        while f.get(i).is_some_and(|b| b"#0- +'".contains(b)) {
            i += 1;
        }
        let mut spec = Vec::new();
        for _ in 0..2 {
            // width, then precision: `*` takes an int argument
            if f.get(i) == Some(&b'.') {
                i += 1;
            }
            if f.get(i) == Some(&b'*') {
                out.push(b"d".to_vec());
                i += 1;
            }
            while f.get(i).is_some_and(|b| b.is_ascii_digit()) {
                i += 1;
            }
        }
        while f.get(i).is_some_and(|b| b"hlLqjzt".contains(b)) {
            spec.push(f[i]);
            i += 1;
        }
        let c = *f.get(i)?;
        spec.push(match c {
            b'i' => b'd',
            b'o' | b'x' | b'X' => b'u',
            b'e' | b'E' | b'g' | b'G' | b'a' | b'A' | b'F' => b'f',
            c => c,
        });
        out.push(spec);
        i += 1;
    }
    Some(out)
}

/// Decode a `vis(3)` encoded string; None on a malformed escape.
pub(crate) fn unvis(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'\\' {
            out.push(s[i]);
            i += 1;
            continue;
        }
        let c = *s.get(i + 1)?;
        i += 2;
        match c {
            b'0'..=b'7' => {
                let mut v = (c - b'0') as u32;
                for _ in 0..2 {
                    match s.get(i) {
                        Some(&d @ b'0'..=b'7') => {
                            v = v * 8 + (d - b'0') as u32;
                            i += 1;
                        }
                        _ => break,
                    }
                }
                out.push(v as u8);
            }
            b'M' => {
                let k = *s.get(i)?;
                let d = *s.get(i + 1)?;
                i += 2;
                out.push(match k {
                    b'-' => d | 0x80,
                    b'^' => (if d == b'?' { 0x7f } else { d & 0x1f }) | 0x80,
                    _ => return None,
                });
            }
            b'^' => {
                let d = *s.get(i)?;
                i += 1;
                out.push(if d == b'?' { 0x7f } else { d & 0x1f });
            }
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(8),
            b'a' => out.push(7),
            b'v' => out.push(11),
            b'f' => out.push(12),
            b's' => out.push(b' '),
            b'E' => out.push(0x1b),
            b'\n' | b'$' => {}
            c => out.push(c),
        }
    }
    Some(out)
}

fn read_tm(c: &mut Call, p: u64) -> [i32; 9] {
    let mut t = [0i32; 9];
    for (i, v) in t.iter_mut().enumerate() {
        *v = c.mem.read_u32(p + i as u64 * 4).unwrap_or(0) as i32;
    }
    t
}

fn wlen(c: &mut Call, p: u64) -> u64 {
    let mut n = 0;
    while c.mem.read_u32(p + n * 4).unwrap_or(0) != 0 {
        n += 1;
    }
    n
}

fn wcmp(c: &mut Call, a: u64, b: u64) -> Act {
    let mut i = 0;
    loop {
        let (x, y) = (c.mem.read_u32(a + i * 4).unwrap_or(0) as i32, c.mem.read_u32(b + i * 4).unwrap_or(0) as i32);
        if x != y {
            return c.ret(if x < y { u64::MAX } else { 1 });
        }
        if x == 0 {
            return c.ret(0);
        }
        i += 1;
    }
}

fn image_field(c: &mut Call, f: impl Fn(&(u64, u64, alloc::string::String, u64)) -> u64) -> Act {
    let i = c.arg() as u32 as usize;
    let v = c.st.images.get(i).map(&f).unwrap_or(0);
    c.ret(v)
}

/// The address `dlsym` gives for `name` in a system library: Prism's C runtime, a libSystem data
/// cell, or the stub of a function Prism implements (bound now if the program did not import it).
fn system_symbol(c: &mut Call, name: &str) -> Option<u64> {
    if let Some(a) = c.st.runtime.and_then(|r| crate::loader::symbol_in(&c.st.loaded[r..=r], 0, name)) {
        return Some(a);
    }
    if let Some(off) = crate::libsys::data_symbol(name) {
        return Some(SHIM_DATA + off);
    }
    let i = match c.st.stubs.iter().position(|s| s == name) {
        Some(i) => i,
        None => {
            if !crate::libsys::State::implements(name) {
                return None;
            }
            let mut stubs = c.st.stubs.clone();
            stubs.push(name.into());
            crate::loader::map_stubs(c.mem, stubs.len() - 1, stubs.len()).ok()?;
            c.st.set_stubs(stubs);
            c.st.stubs.len() - 1
        }
    };
    Some(crate::loader::SHIM_BASE + i as u64 * crate::loader::STUB_SIZE)
}

pub static FUNCS: &[(&str, Func)] = &[
    // dlopen: the image is mapped here; libc/dyld.c then runs its initializers.
    f!("___prism_dlopen", |c| {
        let p = c.arg();
        if p == 0 {
            return c.ret(crate::loader::HANDLE_BASE); // the main program
        }
        let path = alloc::string::String::from_utf8_lossy(&c.cstr(p)).into_owned();
        Act::Dlopen(path)
    }),
    f!("___prism_dl_inits", |c| {
        let (buf, max) = (c.arg(), c.arg() as usize);
        let n = c.st.dl_inits.len().min(max);
        for (k, f) in c.st.dl_inits.drain(..n).enumerate() {
            let _ = c.mem.write_u64(buf + k as u64 * 8, f);
        }
        c.ret(n as u64)
    }),
    f!("___prism_dlsym", |c| {
        use crate::loader::{symbol_in, HANDLE_BASE, HANDLE_SYSTEM};
        let (h, p) = (c.arg(), c.arg());
        let mut name = alloc::string::String::from("_");
        name += &alloc::string::String::from_utf8_lossy(&c.cstr(p));
        let n = c.st.loaded.len();
        let found = match h as i64 {
            // RTLD_NEXT, RTLD_DEFAULT, RTLD_SELF: every image in load order, then the system.
            -3..=-1 => (0..n).filter(|&i| Some(i) != c.st.runtime).find_map(|i| symbol_in(&c.st.loaded, i, &name)).or_else(|| system_symbol(c, &name)),
            -5 => symbol_in(&c.st.loaded, 0, &name), // RTLD_MAIN_ONLY
            _ if h == HANDLE_SYSTEM => system_symbol(c, &name),
            _ if h >= HANDLE_BASE && ((h - HANDLE_BASE) as usize) < n => symbol_in(&c.st.loaded, (h - HANDLE_BASE) as usize, &name).or_else(|| system_symbol(c, &name)),
            _ => None,
        };
        c.ret(found.unwrap_or(0))
    }),
    f!("_dladdr", |c| {
        let (addr, info) = (c.arg(), c.arg());
        let hit = c.st.loaded.iter().position(|im| im.macho.segments.iter().any(|s| s.vmsize > 0 && addr.wrapping_sub(s.vmaddr.wrapping_add(im.slide)) < s.vmsize));
        let Some(i) = hit else { return c.ret(0) };
        let Some(im) = c.st.images.get(i).cloned() else { return c.ret(0) };
        let mut name = im.3;
        if name == 0 {
            let mut s = im.2.into_bytes();
            s.push(0);
            name = c.st.heap.alloc(c.mem, s.len() as u64);
            let _ = c.mem.write(name, &s);
            c.st.images[i].3 = name;
        }
        // Dl_info: {dli_fname, dli_fbase, dli_sname, dli_saddr}
        for (k, v) in [name, im.0, 0, 0].into_iter().enumerate() {
            let _ = c.mem.write_u64(info + k as u64 * 8, v);
        }
        c.ret(1)
    }),
    f!("__dyld_image_count", |c| { let n = c.st.images.len() as u64; c.ret(n) }),
    f!("__dyld_get_image_header", |c| image_field(c, |im| im.0)),
    f!("__dyld_get_image_vmaddr_slide", |c| image_field(c, |im| im.1)),
    f!("__dyld_get_image_name", |c| {
        let i = c.arg() as u32 as usize;
        let Some(im) = c.st.images.get(i).cloned() else { return c.ret(0) };
        if im.3 != 0 {
            return c.ret(im.3);
        }
        let mut s = im.2.into_bytes();
        s.push(0);
        let p = c.st.heap.alloc(c.mem, s.len() as u64);
        let _ = c.mem.write(p, &s);
        c.st.images[i].3 = p;
        c.ret(p)
    }),
    f!("__NSGetMachExecuteHeader", |c| { let h = c.st.images.first().map(|im| im.0).unwrap_or(0); c.ret(h) }),
    f!("_strtonum", |c| {
        let (p, lo, hi, errp) = (c.arg(), c.arg() as i64, c.arg() as i64, c.arg());
        let s = c.cstr(p);
        let t = core::str::from_utf8(&s).unwrap_or("").trim_start();
        let r: Result<i64, &[u8]> = if lo > hi {
            Err(b"invalid\0")
        } else {
            match t.parse::<i64>() {
                Ok(v) if v < lo => Err(b"too small\0"),
                Ok(v) if v > hi => Err(b"too large\0"),
                Ok(v) => Ok(v),
                Err(_) if !t.is_empty() && t.trim_start_matches(['-', '+']).bytes().all(|b| b.is_ascii_digit()) => Err(if t.starts_with('-') { b"too small\0" } else { b"too large\0" }),
                Err(_) => Err(b"invalid\0"),
            }
        };
        match r {
            Ok(v) => {
                if errp != 0 {
                    let _ = c.mem.write_u64(errp, 0);
                }
                c.ret(v as u64)
            }
            Err(msg) => {
                let e = if msg[0] == b'i' { 22 } else { 34 };
                c.set_errno(e);
                if errp != 0 {
                    let m = SHIM_DATA + crate::libsys::D_STATIC + 0x400 + msg[1] as u64 % 4 * 16;
                    let _ = c.mem.write(m, msg);
                    let _ = c.mem.write_u64(errp, m);
                }
                c.ret(0)
            }
        }
    }),
    f!("_wcslen", |c| { let p = c.arg(); let n = wlen(c, p); c.ret(n) }),
    f!("_wcscmp", |c| { let (a, b) = (c.arg(), c.arg()); wcmp(c, a, b) }),
    f!("_wcscoll", |c| { let (a, b) = (c.arg(), c.arg()); wcmp(c, a, b) }),
    f!("_wcschr", |c| {
        let (p, w) = (c.arg(), c.arg() as u32);
        let mut i = 0;
        loop {
            let x = c.mem.read_u32(p + i * 4).unwrap_or(0);
            if x == w {
                return c.ret(p + i * 4);
            }
            if x == 0 {
                return c.ret(0);
            }
            i += 1;
        }
    }),
    f!("_fnmatch", |c| {
        let (p, s, fl) = (c.arg(), c.arg(), c.arg());
        let (p, s) = (c.cstr(p), c.cstr(s));
        c.ret(if fnmatch(&p, &s, fl) { 0 } else { 1 })
    }),
    f!("_strptime", |c| {
        let (s, f, tmp) = (c.arg(), c.arg(), c.arg());
        let (sb, fb) = (c.cstr(s), c.cstr(f));
        let mut tm = read_tm(c, tmp);
        match strptime(&sb, &fb, &mut tm) {
            Some(n) => {
                for (i, v) in tm.iter().enumerate() {
                    let _ = c.mem.write_u32(tmp + i as u64 * 4, *v as u32);
                }
                c.ret(s + n as u64)
            }
            None => c.ret(0),
        }
    }),
    f!("_fmod", |c| {
        let (x, y) = (c.abi.float(c.mem), c.abi.float(c.mem));
        let r = fmod(x, y);
        c.abi.ret_f64(r);
        Act::Done
    }),
    f!("_fmtcheck", |c| {
        let (user, def) = (c.arg(), c.arg());
        let (u, d) = (c.cstr(user), c.cstr(def));
        let ok = matches!((conversions(&u), conversions(&d)), (Some(a), Some(b)) if a == b);
        c.ret(if ok { user } else { def })
    }),
    f!("_strunvis", |c| {
        let (dst, src) = (c.arg(), c.arg());
        let s = c.cstr(src);
        match unvis(&s) {
            Some(mut v) => {
                let n = v.len();
                v.push(0);
                let _ = c.mem.write(dst, &v);
                c.ret(n as u64)
            }
            None => c.ret(u64::MAX),
        }
    }),
    f!("_devname", |c| { let (_d, _t) = (c.arg(), c.arg()); ret_str(c, "??") }),
    f!("_mkfifo", |c| { let (p, m) = (c.arg(), c.arg()); Act::Linux(133, [p, 0o010000 | (m & 0o7777), 0, 0, 0, 0], Post::Libc) }),
    f!("_sethostname", |c| { c.set_errno(1); c.ret(u64::MAX) }),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn globs_dates_vis() {
        assert!(fnmatch(b"*.c", b"main.c", 0));
        assert!(!fnmatch(b"*.c", b"main.h", 0));
        assert!(fnmatch(b"a[!x-z]c", b"abc", 0));
        assert!(!fnmatch(b"*", b"a/b", FNM_PATHNAME));
        assert!(!fnmatch(b"*", b".hidden", FNM_PERIOD));
        assert!(fnmatch(b"A?C", b"abc", FNM_CASEFOLD));
        assert!(fnmatch(b"a\\*", b"a*", 0));
        let mut tm = [0; 9];
        assert_eq!(strptime(b"2026-11-15 13:04:05", b"%Y-%m-%d %H:%M:%S", &mut tm), Some(19));
        assert_eq!(&tm[..6], &[5, 4, 13, 15, 10, 126]);
        let mut tm = [0; 9];
        assert!(strptime(b"Nov 3 2:05pm", b"%b %d %I:%M%p", &mut tm).is_some());
        assert_eq!((tm[4], tm[3], tm[2]), (10, 3, 14));
        assert_eq!(unvis(b"a\\040b\\M-a\\^A"), Some(vec![b'a', 32, b'b', 0xe1, 1]));
        assert_eq!(conversions(b"%5.2f %ld"), conversions(b"%f %ld"));
        assert_ne!(conversions(b"%s"), conversions(b"%d"));
        assert_eq!(fmod(7.5, 2.0), 1.5);
        assert_eq!(fmod(-7.5, 2.0), -1.5);
        assert_eq!(fmod(1e300, 3.0), 1e300 % 3.0);
        assert_eq!(fmod(0.3, 0.1), 0.3 % 0.1);
    }
}
