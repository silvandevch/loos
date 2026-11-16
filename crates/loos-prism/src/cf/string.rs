//! CFString / NSString logic. A string is `{isa, flags, data, length, capacity}`: 8-bit characters
//! (ASCII, NUL-terminated; flag 0x08) or UTF-16 (flag 0x10), exactly as the compiler lays out
//! `@"..."` and `CFSTR` constants. All work is done on UTF-16 units, the unit of `length`.

use super::{rd, wr, F_OWNED, O_CAP, O_COUNT, O_DATA, O_FLAGS};
use crate::libsys::{Act, Call, Func};
use alloc::{string::String, vec, vec::Vec};

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

const F_UTF16: u64 = 0x10;
const F_8BIT: u64 = 0x08;

/// UTF-16 units `loc..loc + len` of string object `obj` (clamped to the string).
pub(crate) fn units_in(c: &mut Call, obj: u64, loc: u64, len: u64) -> Vec<u16> {
    let (flags, data, total) = (rd(c, obj + O_FLAGS), rd(c, obj + O_DATA), rd(c, obj + O_COUNT));
    let loc = loc.min(total);
    let len = len.min(total - loc) as usize;
    if flags & F_UTF16 != 0 {
        let b = c.mem.read_vec(data + loc * 2, len * 2).unwrap_or_default();
        b.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect()
    } else {
        c.mem.read_vec(data + loc, len).unwrap_or_default().into_iter().map(u16::from).collect()
    }
}

pub(crate) fn units(c: &mut Call, obj: u64) -> Vec<u16> {
    units_in(c, obj, 0, u64::MAX)
}

/// Make `text` the contents of string object `obj`: 8-bit when it is all ASCII, UTF-16 otherwise.
/// The object's buffer is reused when it is large enough.
pub(crate) fn store(c: &mut Call, obj: u64, text: &[u16]) {
    let flags = rd(c, obj + O_FLAGS);
    let wide = text.iter().any(|&u| u >= 0x80);
    let bytes: Vec<u8> = if wide { text.iter().flat_map(|u| u.to_le_bytes()).collect() } else { text.iter().map(|&u| u as u8).chain([0]).collect() };
    let owned = flags & F_OWNED != 0;
    let (mut data, mut cap) = (rd(c, obj + O_DATA), rd(c, obj + O_CAP));
    if !owned || cap < bytes.len() as u64 {
        if owned && data != 0 {
            c.st.heap.free(c.mem, data);
        }
        cap = (bytes.len() as u64).max(if owned { cap * 2 } else { 0 }).max(16);
        data = c.st.heap.alloc(c.mem, cap);
    }
    let _ = c.mem.write(data, &bytes);
    wr(c, obj + O_FLAGS, flags & !(F_UTF16 | F_8BIT) | if wide { F_UTF16 } else { F_8BIT } | F_OWNED);
    wr(c, obj + O_DATA, data);
    wr(c, obj + O_COUNT, text.len() as u64);
    wr(c, obj + O_CAP, cap);
}

// ---------------------------------------------------------------- encodings

#[derive(Clone, Copy, PartialEq)]
enum Enc {
    Ascii,
    Latin1,
    MacRoman,
    Utf8,
    /// big-endian? `None`: byte-order mark, else the host's order.
    Utf16(Option<bool>),
    Utf32(Option<bool>),
}

/// A CFStringEncoding or an NSStringEncoding (the two numberings do not overlap).
fn encoding(e: u64) -> Option<Enc> {
    Some(match e as u32 {
        1 | 0x0600 | 7 | 0x0bff => Enc::Ascii,
        4 | 0x0800_0100 => Enc::Utf8,
        5 | 0x0201 | 2 | 0x0b01 | 12 | 0x0500 => Enc::Latin1, // Latin-1, NeXTSTEP and Windows Latin-1 share ASCII + Latin-1 letters
        30 | 0 => Enc::MacRoman,
        10 | 0x0100 => Enc::Utf16(None),
        0x9000_0100 | 0x1000_0100 => Enc::Utf16(Some(true)),
        0x9400_0100 | 0x1400_0100 => Enc::Utf16(Some(false)),
        0x8c00_0100 | 0x0c00_0100 => Enc::Utf32(None),
        0x9800_0100 | 0x1800_0100 => Enc::Utf32(Some(true)),
        0x9c00_0100 | 0x1c00_0100 => Enc::Utf32(Some(false)),
        _ => return None,
    })
}

/// Mac OS Roman 0x80-0xFF.
const MAC_ROMAN: &str = "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»…\u{a0}ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{f8ff}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";

fn push_scalar(out: &mut Vec<u16>, ch: u32) -> bool {
    match char::from_u32(ch) {
        Some(ch) => {
            let mut b = [0u16; 2];
            out.extend_from_slice(ch.encode_utf16(&mut b));
            true
        }
        None => false,
    }
}

fn decode(bytes: &[u8], enc: Enc) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(bytes.len());
    match enc {
        Enc::Ascii => {
            if bytes.iter().any(|&b| b >= 0x80) {
                return None;
            }
            out.extend(bytes.iter().map(|&b| b as u16));
        }
        Enc::Latin1 => out.extend(bytes.iter().map(|&b| b as u16)),
        Enc::MacRoman => {
            let table: Vec<char> = MAC_ROMAN.chars().collect();
            for &b in bytes {
                if b < 0x80 { out.push(b as u16) } else { push_scalar(&mut out, table[b as usize - 0x80] as u32); }
            }
        }
        Enc::Utf8 => out.extend(core::str::from_utf8(bytes).ok()?.encode_utf16()),
        Enc::Utf16(order) => {
            let (big, body) = match (order, bytes) {
                (None, [0xfe, 0xff, rest @ ..]) => (true, rest),
                (None, [0xff, 0xfe, rest @ ..]) => (false, rest),
                (o, b) => (o.unwrap_or(false), b),
            };
            if body.len() % 2 != 0 {
                return None;
            }
            out.extend(body.chunks_exact(2).map(|p| if big { u16::from_be_bytes([p[0], p[1]]) } else { u16::from_le_bytes([p[0], p[1]]) }));
        }
        Enc::Utf32(order) => {
            let (big, body) = match (order, bytes) {
                (None, [0, 0, 0xfe, 0xff, rest @ ..]) => (true, rest),
                (None, [0xff, 0xfe, 0, 0, rest @ ..]) => (false, rest),
                (o, b) => (o.unwrap_or(false), b),
            };
            if body.len() % 4 != 0 {
                return None;
            }
            for p in body.chunks_exact(4) {
                let v = if big { u32::from_be_bytes([p[0], p[1], p[2], p[3]]) } else { u32::from_le_bytes([p[0], p[1], p[2], p[3]]) };
                if !push_scalar(&mut out, v) {
                    return None;
                }
            }
        }
    }
    Some(out)
}

/// Scalar values of UTF-16 text; an unpaired surrogate stays as its own value.
fn scalars(text: &[u16]) -> Vec<u32> {
    char::decode_utf16(text.iter().copied()).map(|r| r.map(|c| c as u32).unwrap_or_else(|e| e.unpaired_surrogate() as u32)).collect()
}

/// `text` in `enc`. A character the encoding lacks becomes `loss`, or fails the conversion.
fn encode(text: &[u16], enc: Enc, loss: Option<u8>) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    let mut lossy = |out: &mut Vec<u8>| loss.map(|l| out.push(l));
    match enc {
        Enc::Utf16(order) => {
            for u in text {
                out.extend(if order == Some(true) { u.to_be_bytes() } else { u.to_le_bytes() });
            }
        }
        _ => {
            let table: Vec<char> = if enc == Enc::MacRoman { MAC_ROMAN.chars().collect() } else { Vec::new() };
            for s in scalars(text) {
                match enc {
                    Enc::Ascii if s < 0x80 => out.push(s as u8),
                    Enc::Latin1 if s < 0x100 => out.push(s as u8),
                    Enc::MacRoman if s < 0x80 => out.push(s as u8),
                    Enc::MacRoman => match table.iter().position(|&t| t as u32 == s) {
                        Some(i) => out.push(0x80 + i as u8),
                        None => lossy(&mut out)?,
                    },
                    Enc::Utf8 => match char::from_u32(s) {
                        Some(ch) => out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes()),
                        None => out.extend_from_slice("\u{fffd}".as_bytes()),
                    },
                    Enc::Utf32(order) => out.extend(if order == Some(true) { s.to_be_bytes() } else { s.to_le_bytes() }),
                    _ => lossy(&mut out)?,
                }
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- comparison and search

const CASE_INSENSITIVE: u64 = 1;
const BACKWARDS: u64 = 4;
const ANCHORED: u64 = 8;
const NUMERIC: u64 = 64;
const DIACRITIC_INSENSITIVE: u64 = 128;

/// The characters comparison looks at: case-folded and stripped of combining marks on request.
fn folded(text: &[u16], opts: u64) -> Vec<u32> {
    let mut v = scalars(text);
    if opts & DIACRITIC_INSENSITIVE != 0 {
        v.retain(|s| !(0x300..0x370).contains(s));
    }
    if opts & CASE_INSENSITIVE != 0 {
        v = v.into_iter().flat_map(|s| char::from_u32(s).map(|c| c.to_lowercase().map(|l| l as u32).collect::<Vec<_>>()).unwrap_or_else(|| vec![s])).collect();
    }
    v
}

fn digit(s: u32) -> Option<u32> {
    (0x30..0x3a).contains(&s).then(|| s - 0x30)
}

fn compare(a: &[u16], b: &[u16], opts: u64) -> i64 {
    let (x, y) = (folded(a, opts), folded(b, opts));
    if opts & NUMERIC == 0 {
        return x.cmp(&y) as i64;
    }
    // Runs of digits compare by value ("file9" < "file10").
    let (mut i, mut j) = (0, 0);
    while i < x.len() && j < y.len() {
        if digit(x[i]).is_some() && digit(y[j]).is_some() {
            let run = |v: &[u32], from: usize| (from..v.len()).find(|&k| digit(v[k]).is_none()).unwrap_or(v.len());
            let (ei, ej) = (run(&x, i), run(&y, j));
            let strip = |v: &[u32], from: usize, to: usize| (from..to).find(|&k| v[k] != 0x30).unwrap_or(to);
            let (si, sj) = (strip(&x, i, ei), strip(&y, j, ej));
            let o = (ei - si).cmp(&(ej - sj)).then_with(|| x[si..ei].cmp(&y[sj..ej]));
            if o != core::cmp::Ordering::Equal {
                return o as i64;
            }
            i = ei;
            j = ej;
        } else if x[i] != y[j] {
            return x[i].cmp(&y[j]) as i64;
        } else {
            i += 1;
            j += 1;
        }
    }
    (x.len() - i).cmp(&(y.len() - j)) as i64
}

fn fold_unit(u: u16, opts: u64) -> u16 {
    if opts & CASE_INSENSITIVE == 0 {
        return u;
    }
    char::from_u32(u as u32).and_then(|c| { let mut l = c.to_lowercase(); let first = l.next()?; (l.next().is_none() && (first as u32) < 0x10000).then_some(first as u16) }).unwrap_or(u)
}

/// Where `needle` occurs in `hay` (unit offsets).
fn find(hay: &[u16], needle: &[u16], opts: u64) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    let at = |i: usize| (0..needle.len()).all(|k| fold_unit(hay[i + k], opts) == fold_unit(needle[k], opts));
    let last = hay.len() - needle.len();
    match (opts & BACKWARDS != 0, opts & ANCHORED != 0) {
        (false, true) => at(0).then_some(0),
        (true, true) => at(last).then_some(last),
        (false, false) => (0..=last).find(|&i| at(i)),
        (true, false) => (0..=last).rev().find(|&i| at(i)),
    }
}

fn hash(text: &[u16]) -> u64 {
    text.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &u| (h ^ u as u64).wrapping_mul(0x100_0000_01b3)) >> 1
}

fn mapped(text: &[u16], mode: u64) -> Vec<u16> {
    let s = String::from_utf16_lossy(text);
    let out: String = match mode {
        0 => s.to_uppercase(),
        1 => s.to_lowercase(),
        _ => {
            // Capitalized: the first letter of each word upper case, the rest lower case.
            let mut start = true;
            s.chars().flat_map(|c| {
                let r: Vec<char> = if start { c.to_uppercase().collect() } else { c.to_lowercase().collect() };
                start = !c.is_alphanumeric() && c != '\'';
                r
            }).collect()
        }
    };
    out.encode_utf16().collect()
}

/// Leading number in `text`, as `strtoll`/`strtod` read it after white space.
fn number_text(text: &[u16]) -> Vec<u8> {
    text.iter().skip_while(|&&u| u == 0x20 || (9..=13).contains(&u)).take_while(|&&u| u < 0x80).map(|&u| u as u8).collect()
}

pub static FUNCS: &[(&str, Func)] = &[
    // (string, bytes, count, encoding) → 1, or 0 when the bytes are not valid in the encoding
    f!("___prism_cf_str_init", |c| {
        let (obj, p, n, e) = (c.arg(), c.arg(), c.arg(), c.arg());
        let bytes = c.mem.read_vec(p, n as usize).unwrap_or_default();
        match encoding(e).and_then(|enc| decode(&bytes, enc)) {
            Some(text) => {
                store(c, obj, &text);
                c.ret(1)
            }
            None => c.ret(0),
        }
    }),
    f!("___prism_cf_str_init_chars", |c| {
        let (obj, p, n) = (c.arg(), c.arg(), c.arg());
        let b = c.mem.read_vec(p, n as usize * 2).unwrap_or_default();
        let text: Vec<u16> = b.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
        store(c, obj, &text);
        c.ret(1)
    }),
    // (string, loc, len, encoding, loss byte or -1, buffer, room) → bytes the range needs (-1:
    // cannot be converted); as many whole bytes as fit are written.
    f!("___prism_cf_str_encode", |c| {
        let (obj, loc, len, e, loss, buf, room) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg() as i64, c.arg(), c.arg());
        let text = units_in(c, obj, loc, len);
        match encoding(e).and_then(|enc| encode(&text, enc, (loss >= 0).then_some(loss as u8))) {
            Some(bytes) => {
                if buf != 0 {
                    let _ = c.mem.write(buf, &bytes[..bytes.len().min(room as usize)]);
                }
                c.ret(bytes.len() as u64)
            }
            None => c.ret(u64::MAX),
        }
    }),
    f!("___prism_cf_str_valid_encoding", |c| { let e = c.arg(); c.ret(encoding(e).is_some() as u64) }),
    // (string, loc, len, UTF-16 buffer)
    f!("___prism_cf_str_chars", |c| {
        let (obj, loc, len, buf) = (c.arg(), c.arg(), c.arg(), c.arg());
        let bytes: Vec<u8> = units_in(c, obj, loc, len).iter().flat_map(|u| u.to_le_bytes()).collect();
        let _ = c.mem.write(buf, &bytes);
        c.ret(bytes.len() as u64 / 2)
    }),
    // (a, loc, len, b, options) → -1, 0, 1: a[loc..loc+len] against all of b
    f!("___prism_cf_str_compare", |c| {
        let (a, loc, len, b, opts) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg());
        let (x, y) = (units_in(c, a, loc, len), units(c, b));
        c.ret(compare(&x, &y, opts) as u64)
    }),
    f!("___prism_cf_str_hash", |c| { let a = c.arg(); let x = units(c, a); c.ret(hash(&x)) }),
    // (hay, loc, len, needle, options, out[2]) → 1 and out = {location, length}
    f!("___prism_cf_str_find", |c| {
        let (a, loc, len, b, opts, out) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg(), c.arg());
        let (x, y) = (units_in(c, a, loc, len), units(c, b));
        match find(&x, &y, opts) {
            Some(i) => {
                super::wr(c, out, loc + i as u64);
                super::wr(c, out + 8, y.len() as u64);
                c.ret(1)
            }
            None => c.ret(0),
        }
    }),
    // (string, loc, len, other string or 0): replace the range with the other string's text
    f!("___prism_cf_str_splice", |c| {
        let (obj, loc, len, src) = (c.arg(), c.arg(), c.arg(), c.arg());
        let mut text = units(c, obj);
        let ins = if src == 0 { Vec::new() } else { units(c, src) };
        let loc = (loc as usize).min(text.len());
        let end = loc + (len as usize).min(text.len() - loc);
        text.splice(loc..end, ins);
        store(c, obj, &text);
        c.ret(1)
    }),
    // (destination, source, loc, len): destination = that part of source
    f!("___prism_cf_str_sub", |c| {
        let (dst, src, loc, len) = (c.arg(), c.arg(), c.arg(), c.arg());
        let text = units_in(c, src, loc, len);
        store(c, dst, &text);
        c.ret(1)
    }),
    // (destination, source, 0 upper / 1 lower / 2 capitalized)
    f!("___prism_cf_str_map", |c| {
        let (dst, src, mode) = (c.arg(), c.arg(), c.arg());
        let text = mapped(&units(c, src), mode);
        store(c, dst, &text);
        c.ret(1)
    }),
    f!("___prism_cf_str_i64", |c| {
        let a = c.arg();
        let s = number_text(&units(c, a));
        let neg = s.first() == Some(&b'-');
        let digits = s.iter().skip((neg || s.first() == Some(&b'+')) as usize).take_while(|b| b.is_ascii_digit());
        let v = digits.fold(0i64, |v, &d| v.saturating_mul(10).saturating_add((d - b'0') as i64));
        c.ret(if neg { -v } else { v } as u64)
    }),
    f!("___prism_cf_str_f64", |c| {
        let a = c.arg();
        let s = number_text(&units(c, a));
        c.abi.ret_f64(crate::wide::parse_double(&s).1);
        Act::Done
    }),
];
