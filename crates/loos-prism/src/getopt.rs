//! `getopt`, `getopt_long`, `getopt_long_only`. The globals (`optind`, `optarg`, ...) live in the
//! libSystem data page so programs read them directly. `getopt_long` reorders `argv` so options
//! come first, as macOS does; plain `getopt` stops at the first operand.

#[allow(unused_imports)]
use alloc::{format, string::String, vec::Vec};
use crate::libsys::{read_cbytes, Act, Call, Func, Post, D_OPTARG, D_OPTERR, D_OPTIND, D_OPTOPT, D_OPTPLACE, D_OPTRESET};
use crate::loader::SHIM_DATA;

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

struct Opts {
    /// Short option letters with their `:` markers (prefix flags removed).
    short: Vec<u8>,
    /// Reorder `argv` (getopt_long without a `+` prefix).
    permute: bool,
    /// `-` prefix: operands are returned as option 1.
    in_order: bool,
    /// Leading `:`: report a missing argument as ':' and print nothing.
    colon: bool,
}

/// Option argument kind of short option `ch`: None (unknown), Some(0) none, Some(1) required, Some(2) optional.
fn short_kind(o: &Opts, ch: u8) -> Option<u8> {
    if ch == b':' {
        return None;
    }
    let i = o.short.iter().position(|&b| b == ch)?;
    Some(match (o.short.get(i + 1), o.short.get(i + 2)) {
        (Some(b':'), Some(b':')) => 2,
        (Some(b':'), _) => 1,
        _ => 0,
    })
}

struct LongOpt {
    name: Vec<u8>,
    has_arg: u32,
    flag: u64,
    val: u32,
}

fn long_opts(c: &mut Call, p: u64) -> Vec<LongOpt> {
    let mut v = Vec::new();
    while p != 0 && v.len() < 4096 {
        let at = p + v.len() as u64 * 32;
        let name = c.mem.read_u64(at).unwrap_or(0);
        if name == 0 {
            break;
        }
        let n = read_cbytes(c.mem, name, 256);
        let has_arg = c.mem.read_u32(at + 8).unwrap_or(0);
        let flag = c.mem.read_u64(at + 16).unwrap_or(0);
        let val = c.mem.read_u32(at + 24).unwrap_or(0);
        v.push(LongOpt { name: n, has_arg, flag, val });
    }
    v
}

/// Index of the option `name` matches: exact, or the only one it abbreviates. Err(true): ambiguous.
fn find_long(list: &[LongOpt], name: &[u8]) -> Result<usize, bool> {
    if let Some(i) = list.iter().position(|o| o.name == name) {
        return Ok(i);
    }
    let mut hits = list.iter().enumerate().filter(|(_, o)| o.name.starts_with(name));
    match (hits.next(), hits.next()) {
        (Some((i, _)), None) => Ok(i),
        (Some(_), Some(_)) => Err(true),
        _ => Err(false),
    }
}

struct G<'a, 'b> {
    c: &'a mut Call<'b>,
    argc: u64,
    argv: u64,
}

impl G<'_, '_> {
    fn get(&mut self, at: u64) -> u64 {
        self.c.mem.read_u32(SHIM_DATA + at).unwrap_or(0) as u64
    }
    fn set(&mut self, at: u64, v: u64) {
        let _ = self.c.mem.write_u32(SHIM_DATA + at, v as u32);
    }
    fn set64(&mut self, at: u64, v: u64) {
        let _ = self.c.mem.write_u64(SHIM_DATA + at, v);
    }
    fn arg(&mut self, i: u64) -> u64 {
        self.c.mem.read_u64(self.argv + i * 8).unwrap_or(0)
    }
    fn arg_bytes(&mut self, i: u64) -> Vec<u8> {
        let p = self.arg(i);
        read_cbytes(self.c.mem, p, 4096)
    }
    /// Return `r`, printing `msg` (prefixed with the program name) when opterr is set.
    fn done(&mut self, r: u64, msg: Option<String>, quiet: bool) -> Act {
        match msg {
            Some(m) if !quiet && self.get(D_OPTERR) != 0 => {
                let pp = self.c.mem.read_u64(SHIM_DATA + 0x38).unwrap_or(0);
                let prog = read_cbytes(self.c.mem, pp, 256);
                let base = prog.iter().rposition(|&b| b == b'/').map(|i| i + 1).unwrap_or(0);
                let mut line = prog[base..].to_vec();
                line.extend(b": ");
                line.extend(m.as_bytes());
                line.push(b'\n');
                self.c.out(2, &line, Post::Value(r))
            }
            _ => self.c.ret(r),
        }
    }
}

/// How many argv slots option element `s` occupies (2 when its argument is the next element).
fn span(o: &Opts, longs: &[LongOpt], s: &[u8]) -> u64 {
    if s == b"--" {
        return 1;
    }
    if let Some(name) = s.strip_prefix(b"--") {
        if name.contains(&b'=') {
            return 1;
        }
        return match find_long(longs, name) {
            Ok(i) if longs[i].has_arg == 1 => 2,
            _ => 1,
        };
    }
    for (k, &ch) in s.iter().enumerate().skip(1) {
        match short_kind(o, ch) {
            Some(1) => return if k + 1 == s.len() { 2 } else { 1 },
            Some(2) | None => return 1,
            _ => {}
        }
    }
    1
}

fn is_opt(s: &[u8]) -> bool {
    s.len() > 1 && s[0] == b'-'
}

fn getopt(c: &mut Call, long: bool, only: bool) -> Act {
    let (argc, argv, optstr) = (c.arg(), c.arg(), c.arg());
    let (lp, idxp) = if long { (c.arg(), c.arg()) } else { (0, 0) };
    let raw = c.cstr(optstr);
    let longs = long_opts(c, lp);
    let mut short = raw.as_slice();
    let (mut permute, mut in_order) = (long, false);
    match short.first() {
        Some(b'+') => { permute = false; short = &short[1..]; }
        Some(b'-') => { in_order = true; short = &short[1..]; }
        _ => {}
    }
    let colon = short.first() == Some(&b':');
    let o = Opts { short: short.to_vec(), permute, in_order, colon };
    let mut g = G { c, argc: argc & 0xffff_ffff, argv };

    if g.get(D_OPTRESET) != 0 || g.get(D_OPTIND) == 0 {
        g.set(D_OPTRESET, 0);
        g.set(D_OPTIND, 1);
        g.set64(D_OPTPLACE, 0);
    }
    let mut place = g.c.mem.read_u64(SHIM_DATA + D_OPTPLACE).unwrap_or(0);
    if place != 0 && read_cbytes(g.c.mem, place, 1).is_empty() {
        place = 0;
    }
    if place == 0 {
        let mut i = g.get(D_OPTIND);
        if i >= g.argc {
            return g.c.ret(u64::MAX);
        }
        let mut s = g.arg_bytes(i);
        if !is_opt(&s) {
            if o.in_order {
                let a = g.arg(i);
                g.set64(D_OPTARG, a);
                g.set(D_OPTIND, i + 1);
                return g.c.ret(1);
            }
            if !o.permute {
                return g.c.ret(u64::MAX);
            }
            // Bring the next option (and its separate argument) in front of the operands.
            let Some(j) = (i + 1..g.argc).find(|&j| { let b = g.arg_bytes(j); is_opt(&b) }) else { return g.c.ret(u64::MAX) };
            let sj = g.arg_bytes(j);
            let n = span(&o, &longs, &sj).min(g.argc - j);
            let mut slots: Vec<u64> = (i..j + n).map(|k| g.arg(k)).collect();
            slots.rotate_right(n as usize);
            for (k, v) in slots.iter().enumerate() {
                let _ = g.c.mem.write_u64(argv + (i + k as u64) * 8, *v);
            }
            s = g.arg_bytes(i);
        }
        if s == b"--" {
            g.set(D_OPTIND, i + 1);
            return g.c.ret(u64::MAX);
        }
        let as_long = if let Some(rest) = s.strip_prefix(b"--") {
            Some(rest.to_vec())
        } else if only && long {
            let name: Vec<u8> = s[1..].iter().take_while(|&&b| b != b'=').copied().collect();
            let single_short = name.len() == 1 && short_kind(&o, name[0]).is_some();
            (!single_short && find_long(&longs, &name).is_ok()).then(|| s[1..].to_vec())
        } else {
            None
        };
        if let Some(body) = as_long.filter(|_| long) {
            i += 1;
            g.set(D_OPTIND, i);
            let eq = body.iter().position(|&b| b == b'=');
            let name = &body[..eq.unwrap_or(body.len())];
            let shown = String::from_utf8_lossy(name).into_owned();
            let k = match find_long(&longs, name) {
                Ok(k) => k,
                Err(amb) => {
                    g.set(D_OPTOPT, 0);
                    let m = if amb { format!("option `--{shown}' is ambiguous") } else { format!("unrecognized option `--{shown}'") };
                    return g.done(b'?' as u64, Some(m), o.colon);
                }
            };
            let lo = &longs[k];
            let (flag, val, has_arg, full) = (lo.flag, lo.val as u64, lo.has_arg, String::from_utf8_lossy(&lo.name).into_owned());
            let optval = if flag != 0 { 0 } else { val };
            let mut optarg = 0;
            if let Some(e) = eq {
                if has_arg == 0 {
                    g.set(D_OPTOPT, optval);
                    return g.done(b'?' as u64, Some(format!("option `--{full}' doesn't allow an argument")), o.colon);
                }
                let prefix = if s.starts_with(b"--") { 2 } else { 1 };
                optarg = g.arg(i - 1) + prefix + e as u64 + 1; // just after '='
            } else if has_arg == 1 {
                if i >= g.argc {
                    g.set(D_OPTOPT, optval);
                    let r = if o.colon { b':' } else { b'?' };
                    return g.done(r as u64, Some(format!("option `--{full}' requires an argument")), o.colon);
                }
                optarg = g.arg(i);
                g.set(D_OPTIND, i + 1);
            }
            g.set64(D_OPTARG, optarg);
            if idxp != 0 {
                let _ = g.c.mem.write_u32(idxp, k as u32);
            }
            if flag != 0 {
                let _ = g.c.mem.write_u32(flag, val as u32);
                return g.c.ret(0);
            }
            return g.c.ret(val);
        }
        place = g.arg(i) + 1;
    }
    // One short option from the current cluster.
    let ch = read_cbytes(g.c.mem, place, 1).first().copied().unwrap_or(0);
    place += 1;
    let rest_empty = read_cbytes(g.c.mem, place, 1).is_empty();
    let i = g.get(D_OPTIND);
    g.set(D_OPTOPT, ch as u64);
    let next = |g: &mut G, i: u64| {
        g.set(D_OPTIND, i);
        g.set64(D_OPTPLACE, 0);
    };
    match short_kind(&o, ch) {
        None => {
            if rest_empty {
                next(&mut g, i + 1);
            } else {
                g.set64(D_OPTPLACE, place);
            }
            g.done(b'?' as u64, Some(format!("illegal option -- {}", ch as char)), o.colon)
        }
        Some(0) => {
            g.set64(D_OPTARG, 0);
            if rest_empty {
                next(&mut g, i + 1);
            } else {
                g.set64(D_OPTPLACE, place);
            }
            g.c.ret(ch as u64)
        }
        Some(2) => {
            g.set64(D_OPTARG, if rest_empty { 0 } else { place });
            next(&mut g, i + 1);
            g.c.ret(ch as u64)
        }
        _ => {
            if !rest_empty {
                g.set64(D_OPTARG, place);
                next(&mut g, i + 1);
            } else if i + 1 >= g.argc {
                next(&mut g, i + 1);
                let r = if o.colon { b':' } else { b'?' };
                return g.done(r as u64, Some(format!("option requires an argument -- {}", ch as char)), o.colon);
            } else {
                let a = g.arg(i + 1);
                g.set64(D_OPTARG, a);
                next(&mut g, i + 2);
            }
            g.c.ret(ch as u64)
        }
    }
}

pub static FUNCS: &[(&str, Func)] = &[
    f!("_getopt", |c| getopt(c, false, false)),
    f!("_getopt_long", |c| getopt(c, true, false)),
    f!("_getopt_long_only", |c| getopt(c, true, true)),
];
