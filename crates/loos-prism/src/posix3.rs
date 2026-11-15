//! Paths, temporary files, `sysctl` and `select`.

#[allow(unused_imports)]
use alloc::{string::String, vec, vec::Vec};
use crate::darwin::dirfd;
use crate::posix::ret_str;
use crate::libsys::{copy_out, D_PROGNAME, next_rand, read_cbytes, Act, Call, Func, Post, State, Then};
use crate::loader::{SCRATCH, SHIM_DATA};
use loos_lin::mem::UserMem;

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

pub(crate) const K_REALPATH: u8 = 20;
pub(crate) const K_MKSTEMP: u8 = 21;
pub(crate) const K_MKDTEMP: u8 = 22;
pub(crate) const K_REALPATH_CHECK: u8 = 23;

const PATH_MAX: u64 = 1024;
/// Where realpath stages the working directory (second half of the scratch buffer).
const CWD_STAGE: u64 = SCRATCH + 0x8000;

/// Lexically normalise `p` (absolute) : drop `.`, resolve `..`, collapse slashes.
fn normalize(p: &[u8]) -> Vec<u8> {
    let mut parts: Vec<&[u8]> = Vec::new();
    for s in p.split(|&b| b == b'/') {
        match s {
            b"" | b"." => {}
            b".." => { parts.pop(); }
            s => parts.push(s),
        }
    }
    let mut out = Vec::new();
    for s in parts {
        out.push(b'/');
        out.extend_from_slice(s);
    }
    if out.is_empty() { out.push(b'/'); }
    out
}

/// `realpath(path, resolved)`: ctx = [path, resolved (0: allocate)].
fn realpath(c: &mut Call) -> Act {
    let (p, r) = (c.arg(), c.arg());
    if p == 0 { c.set_errno(22); return c.ret(0); }
    // getcwd first (cheap, also used for absolute paths so there is one path through cont).
    Act::Linux(79, [CWD_STAGE, PATH_MAX, 0, 0, 0, 0], Post::Cont(K_REALPATH, [p, r, 0]))
}

/// Fill the trailing `X`s of the template at `t` with random letters; false if there are none.
fn fill_x(mem: &mut UserMem, st: &mut State, t: u64) -> bool {
    let s = read_cbytes(mem, t, PATH_MAX as usize);
    let n = s.iter().rev().take_while(|&&b| b == b'X').count();
    if n == 0 { return false; }
    const SET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    for i in s.len() - n..s.len() {
        let ch = SET[(next_rand(st) % SET.len() as u64) as usize];
        let _ = mem.write(t + i as u64, &[ch]);
    }
    true
}

fn mktemp_step(mem: &mut UserMem, st: &mut State, t: u64, dir: bool, tries: u64) -> Then {
    if tries > 100 || !fill_x(mem, st, t) { return Then::Fail(22, if dir { 0 } else { u64::MAX }); }
    if dir {
        Then::Linux(83, [t, 0o700, 0, 0, 0, 0], Post::Cont(K_MKDTEMP, [t, tries + 1, 0]))
    } else {
        Then::Linux(2, [t, 0x42 | 0x80 | 0x80000, 0o600, 0, 0, 0], Post::Cont(K_MKSTEMP, [t, tries + 1, 0]))
    }
}

/// Continuations of this module (called from `posix::cont` before its generic error rule).
pub fn cont(id: u8, ctx: [u64; 3], v: i64, mem: &mut UserMem, st: &mut State) -> Option<Then> {
    Some(match id {
        K_MKSTEMP | K_MKDTEMP if v == -17 => mktemp_step(mem, st, ctx[0], id == K_MKDTEMP, ctx[1]),
        K_MKDTEMP if v < 0 => Then::Fail(-v, 0),
        K_MKDTEMP => Then::Ret(ctx[0]),
        K_MKSTEMP if v < 0 => Then::Fail(-v, u64::MAX),
        K_MKSTEMP => Then::Ret(v as u64),
        K_REALPATH if v < 0 => Then::Fail(-v, 0),
        K_REALPATH => {
            let path = read_cbytes(mem, ctx[0], PATH_MAX as usize);
            let mut full = if path.first() == Some(&b'/') { Vec::new() } else { read_cbytes(mem, CWD_STAGE, PATH_MAX as usize) };
            full.push(b'/');
            full.extend_from_slice(&path);
            let mut n = normalize(&full);
            if n.len() as u64 >= PATH_MAX { return Some(Then::Fail(36, 0)); }
            let out = if ctx[1] != 0 { ctx[1] } else { st.heap.alloc(mem, PATH_MAX) };
            if out == 0 { return Some(Then::Fail(12, 0)); }
            n.push(0);
            let _ = mem.write(out, &n);
            // The path must exist (POSIX): check with access(F_OK).
            Then::Linux(21, [out, 0, 0, 0, 0, 0], Post::Cont(K_REALPATH_CHECK, [out, ctx[1], 0]))
        }
        K_REALPATH_CHECK if v < 0 => {
            if ctx[1] == 0 { st.heap.free(mem, ctx[0]); }
            Then::Fail(-v, 0)
        }
        K_REALPATH_CHECK => Then::Ret(ctx[0]),
        _ => return None,
    })
}


// ---------------------------------------------------------------- modes, names, misc

/// Apply a `chmod`-style mode string (octal or symbolic, `umask` 022) to `old`; None if invalid.
pub(crate) fn apply_mode(spec: &[u8], old: u32) -> Option<u32> {
    if !spec.is_empty() && spec.iter().all(|b| (b'0'..=b'7').contains(b)) {
        let v = spec.iter().fold(0u32, |a, b| a * 8 + (b - b'0') as u32);
        return (v <= 0o7777).then_some(v);
    }
    let umask = 0o022;
    let mut m = old & 0o7777;
    for clause in spec.split(|&b| b == b',') {
        let mut i = 0;
        let mut who = 0u32;
        while i < clause.len() && b"ugoa".contains(&clause[i]) {
            who |= match clause[i] { b'u' => 0o4700, b'g' => 0o2070, b'o' => 0o0007, _ => 0o7777 };
            i += 1;
        }
        let explicit = who != 0;
        if !explicit { who = 0o7777; }
        if i >= clause.len() { return None; }
        while i < clause.len() {
            let op = clause[i];
            if !b"+-=".contains(&op) { return None; }
            i += 1;
            let mut bits = 0u32;
            while i < clause.len() && !b"+-=".contains(&clause[i]) {
                bits |= match clause[i] {
                    b'r' => 0o444, b'w' => 0o222, b'x' => 0o111,
                    b'X' => if m & 0o111 != 0 || old & 0o170000 == 0o040000 { 0o111 } else { 0 },
                    b's' => 0o6000, b't' => 0o1000,
                    b'u' => { let u = (m >> 6) & 7; u * 0o111 }
                    b'g' => { let g = (m >> 3) & 7; g * 0o111 }
                    b'o' => { let o = m & 7; o * 0o111 }
                    _ => return None,
                };
                i += 1;
            }
            let mut eff = bits & who;
            if !explicit { eff &= !umask | 0o7000; }
            match op {
                b'+' => m |= eff,
                b'-' => m &= !eff,
                _ => m = (m & !(who & if explicit { 0o7777 } else { !umask & 0o7777 })) | eff,
            }
        }
    }
    Some(m)
}

/// `wcwidth`: 0 for combining marks, 2 for East Asian wide, -1 for controls.
pub(crate) fn wcwidth(c: u32) -> i32 {
    match c {
        0 => 0,
        1..=0x1f | 0x7f..=0x9f => -1,
        0x300..=0x36f | 0x200b..=0x200f | 0xfe00..=0xfe0f | 0x20d0..=0x20ff => 0,
        0x1100..=0x115f | 0x2e80..=0x303e | 0x3041..=0x33ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xa000..=0xa4cf
        | 0xac00..=0xd7a3 | 0xf900..=0xfaff | 0xfe30..=0xfe4f | 0xff00..=0xff60 | 0xffe0..=0xffe6
        | 0x1f300..=0x1f64f | 0x1f900..=0x1f9ff | 0x20000..=0x3fffd => 2,
        _ => 1,
    }
}

fn expand_number(s: &[u8]) -> Option<u64> {
    let d = s.iter().take_while(|b| b.is_ascii_digit()).count();
    if d == 0 { return None; }
    let n: u64 = core::str::from_utf8(&s[..d]).ok()?.parse().ok()?;
    let shift = match s.get(d).map(|b| b.to_ascii_lowercase()) {
        None => 0, Some(b'b') => 0, Some(b'k') => 10, Some(b'm') => 20, Some(b'g') => 30,
        Some(b't') => 40, Some(b'p') => 50, Some(b'e') => 60, _ => return None,
    };
    if s.len() > d + 1 + (s.get(d + 1).map(|b| b.eq_ignore_ascii_case(&b'b')).unwrap_or(false) as usize) { return None; }
    n.checked_mul(1u64.checked_shl(shift)?)
}

fn name_of(id: u64, users: bool) -> &'static str {
    match (id, users) {
        (0, true) => "root", (501, true) => "user",
        (0, false) => "wheel", (20, false) => "staff",
        _ => "",
    }
}

/// Darwin `*at` flags → Linux.
fn at_flags(f: u64) -> u64 {
    [(0x10, 0x200), (0x20, 0x100), (0x40, 0x400), (0x80, 0x200)].iter().filter(|(d, _)| f & d != 0).fold(0, |a, (_, l)| a | l)
}

fn utime_ns(v: u64) -> u64 {
    match v as i64 { -1 => (1 << 30) - 1, -2 => (1 << 30) - 2, _ => v }
}

// ---------------------------------------------------------------- sysctl

enum Val { Int(u32), Quad(u64), Str(&'static [u8]) }

fn by_name(name: &[u8]) -> Option<Val> {
    Some(match name {
        b"hw.ncpu" | b"hw.activecpu" | b"hw.logicalcpu" | b"hw.physicalcpu" | b"hw.logicalcpu_max" | b"hw.physicalcpu_max" => Val::Int(1),
        b"hw.memsize" => Val::Quad(1 << 30),
        b"hw.pagesize" => Val::Quad(loos_lin::mem::PAGE),
        b"hw.cachelinesize" => Val::Quad(64),
        b"hw.byteorder" => Val::Int(1234),
        b"hw.machine" => Val::Str(b"x86_64"),
        b"hw.model" => Val::Str(b"LoOS"),
        b"hw.optional.arm64" => Val::Int(0),
        b"kern.ostype" => Val::Str(b"Darwin"),
        b"kern.osrelease" => Val::Str(b"24.6.0"),
        b"kern.osproductversion" => Val::Str(b"15.8.2"),
        b"kern.version" => Val::Str(b"Darwin Kernel Version 24.6.0: LoOS Prism"),
        b"kern.hostname" => Val::Str(b"loos"),
        b"kern.argmax" => Val::Int(1 << 20),
        b"kern.maxfilesperproc" | b"kern.maxfiles" => Val::Int(10240),
        b"kern.osversion" => Val::Str(b"24G0"),
        b"sysctl.proc_translated" => Val::Int(0),
        _ => return None,
    })
}

/// MIB → name for the numeric `sysctl` (CTL_KERN 1, CTL_HW 6).
fn mib_name(a: u32, b: u32) -> Option<&'static [u8]> {
    Some(match (a, b) {
        (1, 1) => b"kern.ostype",
        (1, 2) => b"kern.osrelease",
        (1, 4) => b"kern.version",
        (1, 8) => b"kern.argmax",
        (1, 10) => b"kern.hostname",
        (1, 29) => b"kern.maxfilesperproc",
        (6, 1) => b"hw.machine",
        (6, 2) => b"hw.model",
        (6, 3) => b"hw.ncpu",
        (6, 4) => b"hw.byteorder",
        (6, 7) => b"hw.pagesize",
        (6, 24) => b"hw.memsize",
        (6, 25) => b"hw.availcpu",
        _ => return None,
    })
}

fn answer(c: &mut Call, v: Option<Val>, old: u64, oldlen: u64) -> Act {
    let Some(v) = v else { c.set_errno(2); return c.ret(u64::MAX) };
    let mut bytes = match v {
        Val::Int(i) => i.to_le_bytes().to_vec(),
        Val::Quad(q) => q.to_le_bytes().to_vec(),
        Val::Str(s) => s.to_vec(),
    };
    if matches!(v, Val::Str(_)) { bytes.push(0); }
    let have = if oldlen != 0 { c.mem.read_u64(oldlen).unwrap_or(0) } else { 0 };
    if old != 0 {
        // An int asked for as 8 bytes (or the reverse) is common; widen/narrow like xnu does for ints.
        if have < bytes.len() as u64 && !(bytes.len() == 8 && have == 4) {
            c.set_errno(12);
            return c.ret(u64::MAX);
        }
        let n = bytes.len().min(have as usize);
        let _ = c.mem.write(old, &bytes[..n]);
        if oldlen != 0 { let _ = c.mem.write_u64(oldlen, n as u64); }
    } else if oldlen != 0 {
        let _ = c.mem.write_u64(oldlen, bytes.len() as u64);
    }
    c.ret(0)
}

fn pathconf(n: u64) -> u64 {
    match n { 1 => 32767, 2 | 3 => 1024, 4 => 255, 5 => 1024, 6 => 512, _ => u64::MAX }
}

fn user_group(c: &mut Call, id: u64, nouser: u64, users: bool) -> Act {
    let n = name_of(id & 0xffff_ffff, users);
    if !n.is_empty() { return ret_str(c, n) }
    if nouser != 0 { return c.ret(0) }
    let s = alloc::format!("{}", id as u32);
    ret_str(c, &s)
}

pub static FUNCS: &[(&str, Func)] = &[
    f!("_setmode", |c| {
        let p = c.arg();
        let s = c.cstr(p);
        if apply_mode(&s, 0).is_none() { c.set_errno(22); return c.ret(0) }
        let s = String::from_utf8_lossy(&s).into_owned();
        ret_str(c, &s)
    }),
    f!("_getmode", |c| {
        let (set, m) = (c.arg(), c.arg());
        let s = c.cstr(set);
        let r = apply_mode(&s, m as u32).unwrap_or(m as u32 & 0o7777);
        c.ret(r as u64)
    }),
    f!("_wcwidth", |c| { let w = c.arg() as u32; c.ret(wcwidth(w) as i64 as u64) }),
    f!("_wcswidth", |c| {
        let (p, n) = (c.arg(), c.arg());
        let mut t: i64 = 0;
        for i in 0..n {
            let ch = c.mem.read_u32(p + i * 4).unwrap_or(0);
            if ch == 0 { break }
            let w = wcwidth(ch);
            if w < 0 { return c.ret(u64::MAX) }
            t += w as i64;
        }
        c.ret(t as u64)
    }),
    f!("_getgroups", |c| { let (n, l) = (c.arg(), c.arg()); if n as i32 > 0 { let _ = c.mem.write_u32(l, 20); } c.ret(1) }),
    f!("_utimensat", |c| {
        let (fd, p, ts, fl) = (c.arg(), c.arg(), c.arg(), c.arg());
        let mut t = 0;
        if ts != 0 {
            for i in 0..2 {
                let s = c.mem.read_u64(ts + i * 16).unwrap_or(0);
                let n = c.mem.read_u64(ts + i * 16 + 8).unwrap_or(0);
                let _ = c.mem.write_u64(SCRATCH + i * 16, s);
                let _ = c.mem.write_u64(SCRATCH + i * 16 + 8, utime_ns(n));
            }
            t = SCRATCH;
        }
        Act::Linux(280, [dirfd(fd), p, t, at_flags(fl), 0, 0], Post::Libc)
    }),
    f!("_faccessat", |c| { let (fd, p, m, fl) = (c.arg(), c.arg(), c.arg(), c.arg()); Act::Linux(439, [dirfd(fd), p, m, at_flags(fl), 0, 0], Post::Libc) }),
    f!("_linkat", |c| { let (a, p, b, q, fl) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg()); Act::Linux(265, [dirfd(a), p, dirfd(b), q, at_flags(fl), 0], Post::Libc) }),
    f!("_getpriority", |c| { let _ = c; c.ret(0) }),
    f!("_setpriority", |c| { let (w, i, p) = (c.arg(), c.arg(), c.arg()); Act::Linux(141, [w, i, p, 0, 0, 0], Post::Libc) }),
    f!("_chflags", |c| { let (_, f) = (c.arg(), c.arg()); if f == 0 { c.ret(0) } else { c.set_errno(95); c.ret(u64::MAX) } }),
    f!("_lchflags", |c| { let (_, f) = (c.arg(), c.arg()); if f == 0 { c.ret(0) } else { c.set_errno(95); c.ret(u64::MAX) } }),
    f!("_fchflags", |c| { let (_, f) = (c.arg(), c.arg()); if f == 0 { c.ret(0) } else { c.set_errno(95); c.ret(u64::MAX) } }),
    f!("_pathconf", |c| { let (_, n) = (c.arg(), c.arg()); c.ret(pathconf(n)) }),
    f!("_fpathconf", |c| { let (_, n) = (c.arg(), c.arg()); c.ret(pathconf(n)) }),
    f!("_ttyname", |c| { let fd = c.arg(); if fd <= 2 { ret_str(c, "/dev/ttys000") } else { c.set_errno(25); c.ret(0) } }),
    f!("_confstr", |c| {
        let (n, b, l) = (c.arg(), c.arg(), c.arg());
        let v: &[u8] = match n { 1 => b"/usr/bin:/bin:/usr/sbin:/sbin", 65537 | 65538 => b"/tmp/", _ => { c.set_errno(22); return c.ret(0) } };
        copy_out(c, b, l, v);
        c.ret(v.len() as u64 + 1)
    }),
    f!("_sync", |c| { let _ = c; Act::Linux(162, [0; 6], Post::Raw) }),
    f!("_setprogname", |c| { let p = c.arg(); let _ = c.mem.write_u64(SHIM_DATA + D_PROGNAME, p); c.ret(0) }),
    f!("_user_from_uid", |c| { let (id, no) = (c.arg(), c.arg()); user_group(c, id, no, true) }),
    f!("_group_from_gid", |c| { let (id, no) = (c.arg(), c.arg()); user_group(c, id, no, false) }),
    f!("_setpwent", |c| c.ret(1)),
    f!("_endpwent", |c| c.ret(0)),
    f!("_setgrent", |c| c.ret(1)),
    f!("_endgrent", |c| c.ret(0)),
    f!("_rpmatch", |c| { let p = c.arg(); let s = c.cstr(p); c.ret(match s.first() { Some(b'y' | b'Y') => 1, Some(b'n' | b'N') => 0, _ => u64::MAX }) }),
    f!("_expand_number", |c| {
        let (p, out) = (c.arg(), c.arg());
        let s = c.cstr(p);
        match expand_number(&s) { Some(v) => { let _ = c.mem.write_u64(out, v); c.ret(0) } None => { c.set_errno(22); c.ret(u64::MAX) } }
    }),
    ("_realpath", realpath as Func),
    ("_realpath$DARWIN_EXTSN", realpath as Func),
    f!("_mkstemp", |c| {
        let t = c.arg();
        match mktemp_step(c.mem, c.st, t, false, 0) {
            Then::Linux(nr, a, p) => Act::Linux(nr, a, p),
            _ => { c.set_errno(22); c.ret(u64::MAX) }
        }
    }),
    f!("_mkdtemp", |c| {
        let t = c.arg();
        match mktemp_step(c.mem, c.st, t, true, 0) {
            Then::Linux(nr, a, p) => Act::Linux(nr, a, p),
            _ => { c.set_errno(22); c.ret(0) }
        }
    }),
    f!("_mktemp", |c| { let t = c.arg(); if fill_x(c.mem, c.st, t) { c.ret(t) } else { c.set_errno(22); c.ret(0) } }),
    f!("_sysctlbyname", |c| {
        let (n, old, oldlen) = (c.arg(), c.arg(), c.arg());
        let name = c.cstr(n);
        answer(c, by_name(&name), old, oldlen)
    }),
    f!("_sysctl", |c| {
        let (mib, len, old, oldlen) = (c.arg(), c.arg(), c.arg(), c.arg());
        let a = c.mem.read_u32(mib).unwrap_or(0);
        let b = if len > 1 { c.mem.read_u32(mib + 4).unwrap_or(0) } else { 0 };
        let v = mib_name(a, b).and_then(by_name);
        answer(c, v, old, oldlen)
    }),
    f!("_select", |c| {
        let a: [u64; 5] = core::array::from_fn(|_| c.arg());
        Act::Linux(23, [a[0], a[1], a[2], a[3], a[4], 0], Post::Libc)
    }),
    f!("_select$1050", |c| {
        let a: [u64; 5] = core::array::from_fn(|_| c.arg());
        Act::Linux(23, [a[0], a[1], a[2], a[3], a[4], 0], Post::Libc)
    }),
    f!("_select$DARWIN_EXTSN", |c| {
        let a: [u64; 5] = core::array::from_fn(|_| c.arg());
        Act::Linux(23, [a[0], a[1], a[2], a[3], a[4], 0], Post::Libc)
    }),
];

#[cfg(test)]
mod tests {
    #[test]
    fn normalizes() {
        assert_eq!(super::normalize(b"/a/./b//../c/"), b"/a/c");
        assert_eq!(super::normalize(b"/../.."), b"/");
    }
    #[test]
    fn modes() {
        use super::apply_mode as m;
        assert_eq!(m(b"755", 0), Some(0o755));
        assert_eq!(m(b"u+x,go-w", 0o666), Some(0o744));
        assert_eq!(m(b"+x", 0o644), Some(0o755));
        assert_eq!(m(b"a=r", 0o777), Some(0o444));
        assert_eq!(m(b"g=u", 0o740), Some(0o770));
        assert_eq!(m(b"q", 0), None);
        assert_eq!(super::expand_number(b"4k"), Some(4096));
    }
}
