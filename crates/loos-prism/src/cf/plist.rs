//! Property lists: reading XML, binary (`bplist00`) and OpenStep text, writing XML and binary.
//! The class layer and this code exchange a tree as a flat array of 32-byte records
//! `{kind, n, a, b}` in prefix order; a dictionary record is followed by its key and value
//! subtrees in turn, an array record by its elements.
//!
//! kinds: 1 dictionary (n entries), 2 array (n), 3 string (UTF-8 at a, n bytes), 4 integer (a),
//! 5 real (a = f64 bits), 6 boolean (a), 7 date (a = f64 bits, seconds since 2001), 8 data (at a,
//! n bytes), 9 UID (a).

use super::{bytes, rd, wr};
use crate::libsys::{Act, Call, Func};
use alloc::{format, string::String, vec, vec::Vec};

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum V {
    Dict(Vec<(Vec<u8>, V)>),
    Array(Vec<V>),
    Str(Vec<u8>),
    Int(i64),
    Real(f64),
    Bool(bool),
    Date(f64),
    Data(Vec<u8>),
    Uid(u64),
}

// ------------------------------------------------------------------ dates and base64

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}
const EPOCH_2001: i64 = 978_307_200;

fn parse_date(s: &str) -> Option<f64> {
    let b = s.trim().as_bytes();
    let num = |r: core::ops::Range<usize>| -> Option<i64> { core::str::from_utf8(b.get(r)?).ok()?.parse().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = if b.len() >= 19 { (num(11..13)?, num(14..16)?, num(17..19)?) } else { (0, 0, 0) };
    Some((days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se - EPOCH_2001) as f64)
}
fn format_date(t: f64) -> String {
    let s = libm_floor(t) as i64 + EPOCH_2001;
    let (y, m, d) = civil_from_days(s.div_euclid(86400));
    let r = s.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", r / 3600, r / 60 % 60, r % 60)
}
fn libm_floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
fn b64_encode(d: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    for ch in d.chunks(3) {
        let v = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            o.push(if i <= ch.len() { B64[(v >> (18 - 6 * i) & 63) as usize] } else { b'=' });
        }
    }
    o
}
fn b64_decode(s: &[u8]) -> Vec<u8> {
    let (mut o, mut acc, mut bits) = (Vec::new(), 0u32, 0);
    for &c in s {
        let Some(v) = B64.iter().position(|&x| x == c) else { continue };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            o.push((acc >> bits) as u8);
        }
    }
    o
}

// ------------------------------------------------------------------ XML

struct Xml<'a> {
    s: &'a [u8],
    i: usize,
}

fn entities(t: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if t[i] == b'&' {
            if let Some(e) = t[i..].iter().position(|&b| b == b';') {
                let name = &t[i + 1..i + e];
                let ch: Option<u32> = match name {
                    b"amp" => Some(38),
                    b"lt" => Some(60),
                    b"gt" => Some(62),
                    b"quot" => Some(34),
                    b"apos" => Some(39),
                    _ if name.starts_with(b"#x") => core::str::from_utf8(&name[2..]).ok().and_then(|h| u32::from_str_radix(h, 16).ok()),
                    _ if name.starts_with(b"#") => core::str::from_utf8(&name[1..]).ok().and_then(|h| h.parse().ok()),
                    _ => None,
                };
                if let Some(c) = ch.and_then(char::from_u32) {
                    let mut buf = [0; 4];
                    o.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    i += e + 1;
                    continue;
                }
            }
        }
        o.push(t[i]);
        i += 1;
    }
    o
}

impl Xml<'_> {
    /// The next tag: (name, closing, self-closing). Skips text, comments, `<?...?>` and `<!...>`.
    fn tag(&mut self) -> Option<(&[u8], bool, bool)> {
        loop {
            let lt = self.s[self.i..].iter().position(|&b| b == b'<')? + self.i;
            if self.s[lt..].starts_with(b"<!--") {
                self.i = lt + 4 + find(&self.s[lt + 4..], b"-->")? + 3;
                continue;
            }
            let gt = self.s[lt..].iter().position(|&b| b == b'>')? + lt;
            self.i = gt + 1;
            let inner = &self.s[lt + 1..gt];
            if inner.starts_with(b"?") || inner.starts_with(b"!") {
                continue;
            }
            let closing = inner.starts_with(b"/");
            let selfc = inner.ends_with(b"/");
            let body = &inner[closing as usize..inner.len() - selfc as usize];
            let name_end = body.iter().position(|b| b.is_ascii_whitespace()).unwrap_or(body.len());
            return Some((&self.s[lt + 1 + closing as usize..lt + 1 + closing as usize + name_end], closing, selfc));
        }
    }
    /// The text up to the closing tag `name`.
    fn text(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        let close = [b"</".as_slice(), name, b">"].concat();
        let e = find(&self.s[self.i..], &close)? + self.i;
        let t = entities(&self.s[self.i..e]);
        self.i = e + close.len();
        Some(t)
    }
    fn value(&mut self) -> Option<V> {
        let (name, closing, selfc) = self.tag()?;
        if closing {
            return None;
        }
        let name = name.to_vec();
        Some(match name.as_slice() {
            b"plist" => {
                if selfc {
                    return None;
                }
                self.value()?
            }
            b"dict" => {
                let mut d = Vec::new();
                if !selfc {
                    loop {
                        let save = self.i;
                        let (n, cl, sc) = self.tag()?;
                        if cl {
                            break;
                        }
                        if n != b"key" {
                            self.i = save;
                            return None;
                        }
                        let k = if sc { Vec::new() } else { self.text(b"key")? };
                        d.push((k, self.value()?));
                    }
                }
                V::Dict(d)
            }
            b"array" => {
                let mut a = Vec::new();
                if !selfc {
                    loop {
                        let save = self.i;
                        let (_, cl, _) = self.tag()?;
                        if cl {
                            break;
                        }
                        self.i = save;
                        a.push(self.value()?);
                    }
                }
                V::Array(a)
            }
            b"true" => V::Bool(true),
            b"false" => V::Bool(false),
            _ => {
                let t = if selfc { Vec::new() } else { self.text(&name)? };
                let st = String::from_utf8_lossy(&t);
                match name.as_slice() {
                    b"string" | b"key" => V::Str(t),
                    b"integer" => {
                        let s = st.trim();
                        V::Int(if let Some(h) = s.strip_prefix("0x") { i64::from_str_radix(h, 16).ok()? } else { s.parse::<i64>().or_else(|_| s.parse::<u64>().map(|u| u as i64)).ok()? })
                    }
                    b"real" => V::Real(match st.trim() {
                        "nan" => f64::NAN,
                        "+infinity" | "infinity" | "inf" => f64::INFINITY,
                        "-infinity" | "-inf" => f64::NEG_INFINITY,
                        s => s.parse().ok()?,
                    }),
                    b"date" => V::Date(parse_date(&st)?),
                    b"data" => V::Data(b64_decode(&t)),
                    _ => return None,
                }
            }
        })
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn xml_escape(o: &mut Vec<u8>, s: &[u8]) {
    for &b in s {
        match b {
            b'&' => o.extend_from_slice(b"&amp;"),
            b'<' => o.extend_from_slice(b"&lt;"),
            b'>' => o.extend_from_slice(b"&gt;"),
            _ => o.push(b),
        }
    }
}

fn real_text(f: f64) -> String {
    if f.is_nan() {
        "nan".into()
    } else if f.is_infinite() {
        if f > 0.0 { "+infinity".into() } else { "-infinity".into() }
    } else if f == libm_floor(f) && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

fn write_xml(o: &mut Vec<u8>, v: &V, depth: usize) {
    let tabs = |o: &mut Vec<u8>, n: usize| o.extend(core::iter::repeat_n(b'\t', n));
    tabs(o, depth);
    match v {
        V::Dict(d) if d.is_empty() => o.extend_from_slice(b"<dict/>\n"),
        V::Dict(d) => {
            o.extend_from_slice(b"<dict>\n");
            let mut sorted: Vec<&(Vec<u8>, V)> = d.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, val) in sorted {
                tabs(o, depth + 1);
                o.extend_from_slice(b"<key>");
                xml_escape(o, k);
                o.extend_from_slice(b"</key>\n");
                write_xml(o, val, depth + 1);
            }
            tabs(o, depth);
            o.extend_from_slice(b"</dict>\n");
        }
        V::Array(a) if a.is_empty() => o.extend_from_slice(b"<array/>\n"),
        V::Array(a) => {
            o.extend_from_slice(b"<array>\n");
            for x in a {
                write_xml(o, x, depth + 1);
            }
            tabs(o, depth);
            o.extend_from_slice(b"</array>\n");
        }
        V::Str(s) => {
            o.extend_from_slice(b"<string>");
            xml_escape(o, s);
            o.extend_from_slice(b"</string>\n");
        }
        V::Int(i) => o.extend_from_slice(format!("<integer>{i}</integer>\n").as_bytes()),
        V::Real(f) => o.extend_from_slice(format!("<real>{}</real>\n", real_text(*f)).as_bytes()),
        V::Bool(b) => o.extend_from_slice(if *b { b"<true/>\n" } else { b"<false/>\n" }),
        V::Date(t) => o.extend_from_slice(format!("<date>{}</date>\n", format_date(*t)).as_bytes()),
        V::Data(d) => {
            o.extend_from_slice(b"<data>\n");
            for line in b64_encode(d).chunks(76) {
                tabs(o, depth);
                o.extend_from_slice(line);
                o.push(b'\n');
            }
            tabs(o, depth);
            o.extend_from_slice(b"</data>\n");
        }
        V::Uid(u) => o.extend_from_slice(format!("<dict>\n\t<key>CF$UID</key>\n\t<integer>{u}</integer>\n</dict>\n").as_bytes()),
    }
}

pub(crate) fn to_xml(v: &V) -> Vec<u8> {
    let mut o = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n".to_vec();
    write_xml(&mut o, v, 0);
    o.extend_from_slice(b"</plist>\n");
    o
}

// ------------------------------------------------------------------ OpenStep text output

fn step_string(o: &mut Vec<u8>, s: &[u8]) {
    let bare = !s.is_empty() && s.iter().all(|b| b.is_ascii_alphanumeric() || b"_$+/:.-".contains(b));
    if bare {
        o.extend_from_slice(s);
        return;
    }
    o.push(b'"');
    for &b in s {
        match b {
            b'"' => o.extend_from_slice(b"\\\""),
            b'\\' => o.extend_from_slice(b"\\\\"),
            b'\n' => o.extend_from_slice(b"\\n"),
            b'\t' => o.extend_from_slice(b"\\t"),
            b'\r' => o.extend_from_slice(b"\\r"),
            _ => o.push(b),
        }
    }
    o.push(b'"');
}

/// The old NeXT text format. It has only strings, data, arrays and dictionaries: numbers,
/// booleans and dates are written as their text, as Apple's writer did.
fn write_step(o: &mut Vec<u8>, v: &V, depth: usize) {
    let pad = |o: &mut Vec<u8>, n: usize| o.extend(core::iter::repeat_n(b' ', 4 * n));
    match v {
        V::Dict(d) => {
            o.extend_from_slice(b"{\n");
            let mut sorted: Vec<&(Vec<u8>, V)> = d.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, x) in sorted {
                pad(o, depth + 1);
                step_string(o, k);
                o.extend_from_slice(b" = ");
                write_step(o, x, depth + 1);
                o.extend_from_slice(b";\n");
            }
            pad(o, depth);
            o.push(b'}');
        }
        V::Array(a) => {
            o.extend_from_slice(b"(\n");
            for (i, x) in a.iter().enumerate() {
                pad(o, depth + 1);
                write_step(o, x, depth + 1);
                if i + 1 < a.len() {
                    o.push(b',');
                }
                o.push(b'\n');
            }
            pad(o, depth);
            o.push(b')');
        }
        V::Str(s) => step_string(o, s),
        V::Data(d) => {
            o.push(b'<');
            for (i, b) in d.iter().enumerate() {
                if i > 0 && i % 4 == 0 {
                    o.push(b' ');
                }
                o.extend_from_slice(format!("{b:02x}").as_bytes());
            }
            o.push(b'>');
        }
        V::Int(i) => step_string(o, format!("{i}").as_bytes()),
        V::Real(f) => step_string(o, real_text(*f).as_bytes()),
        V::Bool(b) => step_string(o, if *b { b"1" } else { b"0" }),
        V::Date(t) => step_string(o, format_date(*t).as_bytes()),
        V::Uid(u) => step_string(o, format!("{u}").as_bytes()),
    }
}

pub(crate) fn to_openstep(v: &V) -> Vec<u8> {
    let mut o = Vec::new();
    write_step(&mut o, v, 0);
    o.push(b'\n');
    o
}

// ------------------------------------------------------------------ binary

fn be(b: &[u8]) -> u64 {
    b.iter().fold(0, |a, &x| a << 8 | x as u64)
}

struct Bin<'a> {
    s: &'a [u8],
    offsets: Vec<u64>,
    refsize: usize,
}

impl Bin<'_> {
    fn object(&self, n: u64, depth: usize) -> Option<V> {
        if depth > 512 {
            return None;
        }
        let off = *self.offsets.get(n as usize)? as usize;
        let m = *self.s.get(off)?;
        let (hi, lo) = (m >> 4, (m & 15) as usize);
        let count = |at: usize| -> Option<(usize, usize)> {
            if lo != 15 {
                return Some((lo, at));
            }
            let im = *self.s.get(at)?;
            let w = 1usize << (im & 15);
            Some((be(self.s.get(at + 1..at + 1 + w)?) as usize, at + 1 + w))
        };
        Some(match hi {
            0 => match m {
                8 => V::Bool(false),
                9 => V::Bool(true),
                _ => return None,
            },
            1 => {
                let w = 1usize << lo;
                let raw = be(self.s.get(off + 1..off + 1 + w)?);
                V::Int(if w < 8 { raw as i64 } else { raw as i64 })
            }
            2 => {
                let w = 1usize << lo;
                let raw = be(self.s.get(off + 1..off + 1 + w)?);
                V::Real(if w == 4 { f32::from_bits(raw as u32) as f64 } else { f64::from_bits(raw) })
            }
            3 => V::Date(f64::from_bits(be(self.s.get(off + 1..off + 9)?))),
            4 => {
                let (n, at) = count(off + 1)?;
                V::Data(self.s.get(at..at + n)?.to_vec())
            }
            5 => {
                let (n, at) = count(off + 1)?;
                V::Str(self.s.get(at..at + n)?.to_vec())
            }
            6 => {
                let (n, at) = count(off + 1)?;
                let u: Vec<u16> = self.s.get(at..at + 2 * n)?.chunks(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect();
                V::Str(String::from_utf16_lossy(&u).into_bytes())
            }
            8 => V::Uid(be(self.s.get(off + 1..off + 2 + lo)?)),
            0xa | 0xc => {
                let (n, at) = count(off + 1)?;
                let mut a = Vec::with_capacity(n.min(4096));
                for i in 0..n {
                    let r = be(self.s.get(at + i * self.refsize..at + (i + 1) * self.refsize)?);
                    a.push(self.object(r, depth + 1)?);
                }
                V::Array(a)
            }
            0xd => {
                let (n, at) = count(off + 1)?;
                let mut d = Vec::with_capacity(n.min(4096));
                for i in 0..n {
                    let kr = be(self.s.get(at + i * self.refsize..at + (i + 1) * self.refsize)?);
                    let vr = be(self.s.get(at + (n + i) * self.refsize..at + (n + i + 1) * self.refsize)?);
                    let V::Str(k) = self.object(kr, depth + 1)? else { return None };
                    d.push((k, self.object(vr, depth + 1)?));
                }
                V::Dict(d)
            }
            _ => return None,
        })
    }
}

fn from_binary(s: &[u8]) -> Option<V> {
    if s.len() < 40 {
        return None;
    }
    let t = &s[s.len() - 32..];
    let (osize, refsize) = (t[6] as usize, t[7] as usize);
    let (num, top, table) = (be(&t[8..16]) as usize, be(&t[16..24]), be(&t[24..32]) as usize);
    if osize == 0 || refsize == 0 || num > s.len() {
        return None;
    }
    let offsets = (0..num).map(|i| s.get(table + i * osize..table + (i + 1) * osize).map(be)).collect::<Option<Vec<_>>>()?;
    Bin { s, offsets, refsize }.object(top, 0)
}

fn int_bytes(i: i64) -> (u8, Vec<u8>) {
    if (0..=0xff).contains(&i) {
        (0, vec![i as u8])
    } else if (0..=0xffff).contains(&i) {
        (1, (i as u16).to_be_bytes().to_vec())
    } else if (0..=0xffff_ffff).contains(&i) {
        (2, (i as u32).to_be_bytes().to_vec())
    } else {
        (3, i.to_be_bytes().to_vec())
    }
}

fn to_binary(v: &V) -> Vec<u8> {
    // number the objects in prefix order
    fn count(v: &V) -> usize {
        1 + match v {
            V::Dict(d) => d.iter().map(|(_, x)| 1 + count(x)).sum(),
            V::Array(a) => a.iter().map(count).sum(),
            _ => 0,
        }
    }
    let total = count(v);
    let refsize = if total < 256 { 1 } else if total < 65536 { 2 } else { 4 };
    let mut out = b"bplist00".to_vec();
    let mut offsets = Vec::with_capacity(total);
    fn head(o: &mut Vec<u8>, kind: u8, n: usize) {
        if n < 15 {
            o.push(kind << 4 | n as u8);
        } else {
            o.push(kind << 4 | 15);
            let (p, b) = int_bytes(n as i64);
            o.push(0x10 | p);
            o.extend_from_slice(&b);
        }
    }
    fn emit(v: &V, out: &mut Vec<u8>, offsets: &mut Vec<u64>, refsize: usize) -> u64 {
        let me = offsets.len() as u64;
        offsets.push(0);
        let refb = |r: u64| r.to_be_bytes()[8 - refsize..].to_vec();
        match v {
            V::Dict(d) => {
                let mut kr = Vec::new();
                let mut vr = Vec::new();
                let mut body = Vec::new();
                for (k, x) in d {
                    kr.push(emit(&V::Str(k.clone()), &mut body, offsets, refsize));
                    vr.push(emit(x, &mut body, offsets, refsize));
                }
                // children were written into `body` with offsets relative to it; rebase them
                let start = out.len() as u64;
                let mut h = Vec::new();
                head(&mut h, 0xd, d.len());
                for r in kr.iter().chain(&vr) {
                    h.extend(refb(*r));
                }
                offsets[me as usize] = start;
                let base = start + h.len() as u64;
                for o in offsets[me as usize + 1..].iter_mut() {
                    *o += base;
                }
                out.extend(h);
                out.extend(body);
            }
            V::Array(a) => {
                let mut rs = Vec::new();
                let mut body = Vec::new();
                for x in a {
                    rs.push(emit(x, &mut body, offsets, refsize));
                }
                let start = out.len() as u64;
                let mut h = Vec::new();
                head(&mut h, 0xa, a.len());
                for r in &rs {
                    h.extend(refb(*r));
                }
                offsets[me as usize] = start;
                let base = start + h.len() as u64;
                for o in offsets[me as usize + 1..].iter_mut() {
                    *o += base;
                }
                out.extend(h);
                out.extend(body);
            }
            leaf => {
                offsets[me as usize] = out.len() as u64;
                match leaf {
                    V::Bool(b) => out.push(if *b { 9 } else { 8 }),
                    V::Int(i) => {
                        let (p, b) = int_bytes(*i);
                        out.push(0x10 | p);
                        out.extend(b);
                    }
                    V::Real(f) => {
                        out.push(0x23);
                        out.extend(f.to_bits().to_be_bytes());
                    }
                    V::Date(t) => {
                        out.push(0x33);
                        out.extend(t.to_bits().to_be_bytes());
                    }
                    V::Data(d) => {
                        head(out, 4, d.len());
                        out.extend(d);
                    }
                    V::Str(s) if s.is_ascii() => {
                        head(out, 5, s.len());
                        out.extend(s);
                    }
                    V::Str(s) => {
                        let u: Vec<u16> = String::from_utf8_lossy(s).encode_utf16().collect();
                        head(out, 6, u.len());
                        for x in u {
                            out.extend(x.to_be_bytes());
                        }
                    }
                    V::Uid(u) => {
                        let b = u.to_be_bytes();
                        let skip = b.iter().take_while(|&&x| x == 0).count().min(7);
                        out.push(0x80 | (7 - skip) as u8);
                        out.extend(&b[skip..]);
                    }
                    _ => {}
                }
            }
        }
        me
    }
    let mut body = Vec::new();
    emit(v, &mut body, &mut offsets, refsize);
    for o in offsets.iter_mut() {
        *o += 8;
    }
    out.extend(body);
    let table = out.len() as u64;
    let osize = if table < 256 { 1 } else if table < 65536 { 2 } else if table < 1 << 32 { 4 } else { 8 };
    for o in &offsets {
        out.extend(&o.to_be_bytes()[8 - osize..]);
    }
    out.extend([0u8; 6]);
    out.push(osize as u8);
    out.push(refsize as u8);
    out.extend((total as u64).to_be_bytes());
    out.extend(0u64.to_be_bytes());
    out.extend(table.to_be_bytes());
    out
}

// ------------------------------------------------------------------ OpenStep text

struct Step<'a> {
    s: &'a [u8],
    i: usize,
}

impl Step<'_> {
    fn ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.s[self.i..].starts_with(b"/*") {
                self.i = find(&self.s[self.i..], b"*/").map(|e| self.i + e + 2).unwrap_or(self.s.len());
            } else {
                return;
            }
        }
    }
    fn eat(&mut self, b: u8) -> bool {
        self.ws();
        if self.s.get(self.i) == Some(&b) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn string(&mut self) -> Option<Vec<u8>> {
        self.ws();
        if self.eat(b'"') {
            let mut o = Vec::new();
            while let Some(&c) = self.s.get(self.i) {
                self.i += 1;
                match c {
                    b'"' => return Some(o),
                    b'\\' => {
                        let e = *self.s.get(self.i)?;
                        self.i += 1;
                        o.push(match e {
                            b'n' => b'\n',
                            b't' => b'\t',
                            b'r' => b'\r',
                            _ => e,
                        });
                    }
                    _ => o.push(c),
                }
            }
            return None;
        }
        let st = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || b"_$+/:.-".contains(&self.s[self.i])) {
            self.i += 1;
        }
        (self.i > st).then(|| self.s[st..self.i].to_vec())
    }
    fn value(&mut self, depth: usize) -> Option<V> {
        if depth > 512 {
            return None;
        }
        self.ws();
        if self.eat(b'{') {
            let mut d = Vec::new();
            while !self.eat(b'}') {
                let k = self.string()?;
                if !self.eat(b'=') {
                    return None;
                }
                let v = self.value(depth + 1)?;
                d.push((k, v));
                if !self.eat(b';') {
                    return self.eat(b'}').then_some(V::Dict(d));
                }
            }
            Some(V::Dict(d))
        } else if self.eat(b'(') {
            let mut a = Vec::new();
            while !self.eat(b')') {
                a.push(self.value(depth + 1)?);
                if !self.eat(b',') {
                    return self.eat(b')').then_some(V::Array(a));
                }
            }
            Some(V::Array(a))
        } else if self.eat(b'<') {
            let e = self.s[self.i..].iter().position(|&b| b == b'>')? + self.i;
            let hex: Vec<u8> = self.s[self.i..e].iter().copied().filter(u8::is_ascii_hexdigit).collect();
            self.i = e + 1;
            Some(V::Data(hex.chunks(2).map(|p| u8::from_str_radix(core::str::from_utf8(p).unwrap_or("0"), 16).unwrap_or(0)).collect()))
        } else {
            self.string().map(V::Str)
        }
    }
}

pub(crate) fn parse(s: &[u8]) -> Option<(V, u64)> {
    if s.starts_with(b"bplist00") {
        return from_binary(s).map(|v| (v, 200));
    }
    let t = s.strip_prefix(b"\xef\xbb\xbf").unwrap_or(s);
    let first = t.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    if t[first..].starts_with(b"<") {
        return Xml { s: t, i: 0 }.value().map(|v| (v, 100));
    }
    let mut st = Step { s: t, i: 0 };
    let v = st.value(0)?;
    st.ws();
    // A strings file: `"a" = "b";` pairs without braces.
    if st.i < t.len() {
        let mut st = Step { s: t, i: 0 };
        let mut d = Vec::new();
        loop {
            st.ws();
            if st.i >= t.len() {
                return Some((V::Dict(d), 1));
            }
            let k = st.string()?;
            let v = if st.eat(b'=') { st.value(1)? } else { V::Str(k.clone()) };
            if !st.eat(b';') {
                return None;
            }
            d.push((k, v));
        }
    }
    Some((v, 1))
}

// ------------------------------------------------------------------ records

const REC: u64 = 32;

fn to_records(v: &V, recs: &mut Vec<[u64; 4]>, payload: &mut Vec<(usize, Vec<u8>)>) {
    let idx = recs.len();
    match v {
        V::Dict(d) => {
            recs.push([1, d.len() as u64, 0, 0]);
            for (k, x) in d {
                to_records(&V::Str(k.clone()), recs, payload);
                to_records(x, recs, payload);
            }
        }
        V::Array(a) => {
            recs.push([2, a.len() as u64, 0, 0]);
            for x in a {
                to_records(x, recs, payload);
            }
        }
        V::Str(s) => {
            recs.push([3, s.len() as u64, 0, 0]);
            payload.push((idx, s.clone()));
        }
        V::Int(i) => recs.push([4, 0, *i as u64, 0]),
        V::Real(f) => recs.push([5, 0, f.to_bits(), 0]),
        V::Bool(b) => recs.push([6, 0, *b as u64, 0]),
        V::Date(t) => recs.push([7, 0, t.to_bits(), 0]),
        V::Data(d) => {
            recs.push([8, d.len() as u64, 0, 0]);
            payload.push((idx, d.clone()));
        }
        V::Uid(u) => recs.push([9, 0, *u, 0]),
    }
}

fn from_records(c: &mut Call, base: u64, i: &mut u64, n: u64, depth: usize) -> Option<V> {
    if *i >= n || depth > 512 {
        return None;
    }
    let r = base + *i * REC;
    let (kind, cnt, a) = (rd(c, r), rd(c, r + 8), rd(c, r + 16));
    *i += 1;
    Some(match kind {
        1 => {
            let mut d = Vec::new();
            for _ in 0..cnt {
                let V::Str(k) = from_records(c, base, i, n, depth + 1)? else { return None };
                d.push((k, from_records(c, base, i, n, depth + 1)?));
            }
            V::Dict(d)
        }
        2 => {
            let mut v = Vec::new();
            for _ in 0..cnt {
                v.push(from_records(c, base, i, n, depth + 1)?);
            }
            V::Array(v)
        }
        3 => V::Str(bytes(c, a, cnt)),
        4 => V::Int(a as i64),
        5 => V::Real(f64::from_bits(a)),
        6 => V::Bool(a != 0),
        7 => V::Date(f64::from_bits(a)),
        8 => V::Data(bytes(c, a, cnt)),
        9 => V::Uid(a),
        _ => return None,
    })
}

/// The trap functions of this module.
pub static FUNCS: &[(&str, Func)] = &[
    // (bytes, length, out record count, out format) → malloc'ed records (strings and data
    // follow them in the same block), or NULL when the bytes are not a property list.
    f!("___prism_cf_plist_parse", |c| {
        let (p, n, out_n, out_fmt) = (c.arg(), c.arg(), c.arg(), c.arg());
        let s = bytes(c, p, n);
        let Some((v, fmt)) = parse(&s) else { return c.ret(0) };
        let (mut recs, mut payload) = (Vec::new(), Vec::new());
        to_records(&v, &mut recs, &mut payload);
        let extra: u64 = payload.iter().map(|(_, b)| b.len() as u64 + 1).sum();
        let block = c.st.heap.alloc(c.mem, recs.len() as u64 * REC + extra);
        let mut at = block + recs.len() as u64 * REC;
        for (idx, b) in payload {
            recs[idx][2] = at;
            let _ = c.mem.write(at, &b);
            let _ = c.mem.write(at + b.len() as u64, &[0]);
            at += b.len() as u64 + 1;
        }
        for (i, r) in recs.iter().enumerate() {
            for (j, w) in r.iter().enumerate() {
                wr(c, block + i as u64 * REC + j as u64 * 8, *w);
            }
        }
        if out_n != 0 {
            wr(c, out_n, recs.len() as u64);
        }
        if out_fmt != 0 {
            wr(c, out_fmt, fmt);
        }
        c.ret(block)
    }),
    // (records, count, format: 1 OpenStep / 100 XML / 200 binary, out length) → malloc'ed bytes, or NULL
    f!("___prism_cf_plist_write", |c| {
        let (recs, n, fmt, out_len) = (c.arg(), c.arg(), c.arg(), c.arg());
        let mut i = 0;
        let Some(v) = from_records(c, recs, &mut i, n, 0) else { return c.ret(0) };
        let out = match fmt {
            200 => to_binary(&v),
            1 => to_openstep(&v),
            _ => to_xml(&v),
        };
        let p = c.st.heap.alloc(c.mem, out.len() as u64 + 1);
        let _ = c.mem.write(p, &out);
        if out_len != 0 {
            wr(c, out_len, out.len() as u64);
        }
        c.ret(p)
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> V {
        V::Dict(vec![
            (b"name".to_vec(), V::Str("Prism é".as_bytes().to_vec())),
            (b"n".to_vec(), V::Int(-5)),
            (b"big".to_vec(), V::Int(1 << 40)),
            (b"pi".to_vec(), V::Real(3.25)),
            (b"ok".to_vec(), V::Bool(true)),
            (b"when".to_vec(), V::Date(86400.0 * 365.0)),
            (b"blob".to_vec(), V::Data(vec![0, 1, 2, 250, 251])),
            (b"list".to_vec(), V::Array((0..20).map(V::Int).collect())),
            (b"empty".to_vec(), V::Dict(vec![])),
        ])
    }

    fn sorted(v: V) -> V {
        match v {
            V::Dict(mut d) => {
                d.sort_by(|a, b| a.0.cmp(&b.0));
                V::Dict(d.into_iter().map(|(k, x)| (k, sorted(x))).collect())
            }
            V::Array(a) => V::Array(a.into_iter().map(sorted).collect()),
            x => x,
        }
    }

    #[test]
    fn round_trips() {
        let v = sample();
        let (x, f) = parse(&to_xml(&v)).unwrap();
        assert_eq!((sorted(x), f), (sorted(v.clone()), 100));
        let (b, f) = parse(&to_binary(&v)).unwrap();
        assert_eq!((b, f), (v.clone(), 200));
        assert_eq!(format_date(0.0), "2001-01-01T00:00:00Z");
        assert_eq!(parse_date("2026-11-16T12:30:05Z").map(format_date).unwrap(), "2026-11-16T12:30:05Z");
        let (o, _) = parse(b"{ a = (1, \"two\", <0aff>); b = { c = d; }; }").unwrap();
        assert_eq!(o, V::Dict(vec![
            (b"a".to_vec(), V::Array(vec![V::Str(b"1".to_vec()), V::Str(b"two".to_vec()), V::Data(vec![10, 255])])),
            (b"b".to_vec(), V::Dict(vec![(b"c".to_vec(), V::Str(b"d".to_vec()))])),
        ]));
        let (st, f) = parse(&to_openstep(&v)).unwrap();
        assert_eq!(f, 1);
        let V::Dict(d) = st else { panic!() };
        assert!(d.iter().any(|(k, x)| k == b"name" && *x == V::Str("Prism é".as_bytes().to_vec())));
        assert!(d.iter().any(|(k, x)| k == b"blob" && *x == V::Data(vec![0, 1, 2, 250, 251])));
        let (s, _) = parse(b"/* c */ \"hello\" = \"Hallo\";\n\"bye\" = \"Tsch\\u00fcss\";").unwrap();
        assert!(matches!(s, V::Dict(ref d) if d.len() == 2));
    }
}
