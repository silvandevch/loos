//! More of Prism's libSystem in Rust: ctype tables, error strings, file status, directories,
//! process control and other POSIX calls. Functions that need two kernel steps use
//! [`Post::Cont`], finished in [`cont`].

#[allow(unused_imports)]
use alloc::{format, string::String, vec, vec::Vec};
use crate::darwin::{dirfd, open_flags, stat_to_darwin, STAT_STAGE};
use crate::libsys::{read_cbytes, Act, Call, Func, Post, State, Then, D_ERRSTR, D_RUNE, D_STATIC};
use crate::loader::{SCRATCH, SHIM_DATA};
use loos_lin::mem::UserMem;

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

/// Darwin error messages, numbered from 0 (generated on macOS, one per line).
pub static ERRORS_TXT: &str = include_str!("../data/errno.txt");
/// Number of messages.
pub fn errors() -> impl Iterator<Item = &'static str> {
    ERRORS_TXT.lines()
}

// ctype bits (SDK `_CTYPE_*`).
const A: u32 = 0x100;
const C: u32 = 0x200;
const D: u32 = 0x400;
const G: u32 = 0x800;
const L: u32 = 0x1000;
const P: u32 = 0x2000;
const S: u32 = 0x4000;
const U: u32 = 0x8000;
const X: u32 = 0x1_0000;
const B: u32 = 0x2_0000;
const R: u32 = 0x4_0000;
const SW1: u32 = 0x4000_0000;

/// Character class bits of byte `c` in the C locale (digit value in the low bits, as macOS does).
pub fn ctype(c: u32) -> u32 {
    let Ok(b) = u8::try_from(c) else { return 0 };
    if b >= 0x80 {
        return 0;
    }
    let mut t = 0;
    if b < 0x20 || b == 0x7f {
        t |= C;
    }
    if b == b' ' || b == b'\t' {
        t |= B;
    }
    if matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        t |= S;
    }
    if b.is_ascii_uppercase() {
        t |= U | A;
    }
    if b.is_ascii_lowercase() {
        t |= L | A;
    }
    if b.is_ascii_digit() {
        t |= D | X | (b - b'0') as u32;
    } else if b.is_ascii_hexdigit() {
        t |= X | (b.to_ascii_lowercase() - b'a' + 10) as u32;
    }
    if (0x21..0x7f).contains(&b) {
        t |= G | R | SW1;
        if b.is_ascii_punctuation() {
            t |= P;
        }
    }
    if b == b' ' {
        t |= R | SW1;
    }
    t
}

/// Fill the parts of the libSystem data page this module owns.
pub fn init_data(d: &mut [u8]) {
    // _DefaultRuneLocale: magic, encoding, then runetype/maplower/mapupper[256] from offset 60.
    let r = D_RUNE as usize;
    d[r..r + 8].copy_from_slice(b"RuneMagA");
    d[r + 8..r + 12].copy_from_slice(b"NONE");
    for c in 0..256u32 {
        let lo = if (b'A' as u32..=b'Z' as u32).contains(&c) { c + 32 } else { c };
        let up = if (b'a' as u32..=b'z' as u32).contains(&c) { c - 32 } else { c };
        let i = c as usize * 4;
        d[r + 60 + i..r + 64 + i].copy_from_slice(&ctype(c).to_le_bytes());
        d[r + 1084 + i..r + 1088 + i].copy_from_slice(&lo.to_le_bytes());
        d[r + 2108 + i..r + 2112 + i].copy_from_slice(&up.to_le_bytes());
    }
    // sys_errlist: pointer table, then the strings.
    let e = D_ERRSTR as usize;
    let mut at = e + 0x400;
    for (i, msg) in errors().enumerate() {
        d[e + i * 8..e + i * 8 + 8].copy_from_slice(&(SHIM_DATA + at as u64).to_le_bytes());
        d[at..at + msg.len()].copy_from_slice(msg.as_bytes());
        at += msg.len() + 1;
    }
}

/// Address of the message for Darwin errno `e`, if known.
fn errmsg(mem: &mut UserMem, e: u64) -> Option<u64> {
    (e < errors().count() as u64).then(|| mem.read_u64(SHIM_DATA + D_ERRSTR + e * 8).ok()).flatten()
}

// Continuation ids.
const K_STAT: u8 = 1;
const K_GETFL: u8 = 2;
const K_RAISE: u8 = 3;
const K_REMOVE: u8 = 4;
const K_OPENDIR: u8 = 5;
const K_READDIR: u8 = 6;
const K_TCGET: u8 = 7;
const K_SIGMASK: u8 = 8;

/// Second step of a two-step call; `v` is the LoOS syscall's result.
pub fn cont(id: u8, ctx: [u64; 3], v: i64, mem: &mut UserMem, st: &mut State) -> Then {
    match id {
        K_READDIR if v == 0 => Then::Ret(0),
        K_READDIR if v < 0 => Then::Fail(-v, 0),
        K_OPENDIR if v < 0 => Then::Fail(-v, 0),
        K_REMOVE if v == -21 => Then::Linux(84, [ctx[0], 0, 0, 0, 0, 0], Post::Libc),
        _ if v < 0 => Then::Fail(-v, u64::MAX),
        K_STAT => {
            let mut l = [0u8; 144];
            match mem.read(STAT_STAGE, &mut l).and_then(|_| mem.write(ctx[0], &stat_to_darwin(&l))) {
                Ok(()) => Then::Ret(0),
                Err(e) => Then::Fail(-e, u64::MAX),
            }
        }
        K_GETFL => Then::Ret(linux_to_darwin_oflags(v as u64)),
        K_RAISE => Then::Linux(62, [v as u64, ctx[0], 0, 0, 0, 0], Post::Libc),
        K_REMOVE => Then::Ret(0),
        K_OPENDIR => crate::posix2::opendir_done(v as u64, mem, st),
        K_READDIR => crate::posix2::readdir_done(ctx[0], v as u64, mem),
        K_TCGET => crate::posix2::tcget_done(ctx[0], mem),
        K_SIGMASK => {
            if ctx[0] != 0 {
                let m = mem.read_u64(SCRATCH + 8).unwrap_or(0);
                let _ = mem.write_u32(ctx[0], crate::signal::mask_to_darwin(m) as u32);
            }
            Then::Ret(0)
        }
        _ => Then::Ret(v as u64),
    }
}

/// Linux open flags → Darwin (for `F_GETFL`).
fn linux_to_darwin_oflags(f: u64) -> u64 {
    let mut d = f & 3;
    for (lx, dw) in [(0x800, 0x4), (0x400, 0x8), (0x40, 0x200), (0x200, 0x400), (0x80, 0x800), (0x1000, 0x80)] {
        if f & lx != 0 {
            d |= dw;
        }
    }
    d
}

/// Darwin signal number → Linux.
pub fn signal_to_linux(s: u64) -> u64 {
    match s {
        10 => 7,
        12 => 31,
        16 => 23,
        17 => 19,
        18 => 20,
        19 => 18,
        20 => 17,
        23 => 29,
        30 => 10,
        31 => 12,
        s => s,
    }
}

/// Darwin `RLIMIT_*` → Linux.
fn rlimit_to_linux(r: u64) -> u64 {
    match r {
        5 => 9,
        6 => 8,
        7 => 6,
        8 => 7,
        r => r,
    }
}

fn stat_call(c: &mut Call, nr: u64, first: u64, path: u64, flags: u64, buf: u64) -> Act {
    let args = if nr == 262 { [first, path, STAT_STAGE, flags, 0, 0] } else { [first, STAT_STAGE, 0, 0, 0, 0] };
    let _ = c;
    Act::Linux(nr, args, Post::Cont(K_STAT, [buf, 0, 0]))
}

fn alloc_zeroed(c: &mut Call, n: u64) -> u64 {
    let p = c.st.heap.alloc(c.mem, n);
    if p != 0 && c.mem.write(p, &vec![0; n as usize]).is_err() {
        return 0;
    }
    p
}

fn realloc(c: &mut Call, old: u64, n: u64) -> u64 {
    let p = c.st.heap.alloc(c.mem, n);
    if p != 0 && old != 0 {
        let keep = c.st.heap.size(c.mem, old).min(n);
        if let Ok(v) = c.mem.read_vec(old, keep as usize) {
            let _ = c.mem.write(p, &v);
        }
        c.st.heap.free(c.mem, old);
    }
    p
}

fn ret_str(c: &mut Call, s: &str) -> Act {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    let p = c.st.heap.alloc(c.mem, v.len() as u64);
    if p != 0 {
        let _ = c.mem.write(p, &v);
    }
    c.ret(p)
}

/// Span of the leading bytes of `s` that are (or, with `inv`, are not) in `set`.
fn span(c: &mut Call, inv: bool) -> Act {
    let (a, b) = (c.arg(), c.arg());
    let (s, set) = (c.cstr(a), c.cstr(b));
    let n = s.iter().take_while(|x| set.contains(x) != inv).count();
    c.ret(n as u64)
}

pub static FUNCS: &[(&str, Func)] = &[
    // ctype
    f!("___maskrune", |c| { let (ch, m) = (c.arg() as u32, c.arg() as u32); c.ret((ctype(ch) & m) as u64) }),
    f!("___istype", |c| { let (ch, m) = (c.arg() as u32, c.arg() as u32); c.ret((ctype(ch) & m != 0) as u64) }),
    f!("___tolower", |c| { let v = c.arg() as u32; c.ret(if (65..=90).contains(&v) { v + 32 } else { v } as u64) }),
    f!("___toupper", |c| { let v = c.arg() as u32; c.ret(if (97..=122).contains(&v) { v - 32 } else { v } as u64) }),
    f!("_setlocale", |c| { let _ = (c.arg(), c.arg()); let p = SHIM_DATA + D_STATIC + 0x100; let _ = c.mem.write(p, b"C\0"); c.ret(p) }),
    f!("_localeconv", |c| {
        // struct lconv: decimal_point "." and empty strings / CHAR_MAX elsewhere.
        let p = SHIM_DATA + D_STATIC + 0x300;
        let dot = SHIM_DATA + D_STATIC + 0x104;
        let empty = dot + 1;
        let _ = c.mem.write(dot, b".\0");
        for k in 0..10 {
            let _ = c.mem.write_u64(p + k * 8, if k == 0 { dot } else { empty });
        }
        let _ = c.mem.write(p + 80, &[0x7f; 16]);
        c.ret(p)
    }),
    // memory
    f!("_malloc_type_malloc", |c| { let n = c.arg(); let p = c.st.heap.alloc(c.mem, n); c.ret(p) }),
    f!("_malloc_type_calloc", |c| { let (a, b) = (c.arg(), c.arg()); match a.checked_mul(b) { Some(n) => { let p = alloc_zeroed(c, n); c.ret(p) } None => c.ret(0) } }),
    f!("_malloc_type_realloc", |c| { let (o, n) = (c.arg(), c.arg()); let p = realloc(c, o, n); c.ret(p) }),
    f!("_reallocf", |c| { let (o, n) = (c.arg(), c.arg()); let p = realloc(c, o, n); if p == 0 && o != 0 { c.st.heap.free(c.mem, o); } c.ret(p) }),
    f!("_malloc_type_free", |c| { let p = c.arg(); c.st.heap.free(c.mem, p); c.ret(0) }),
    f!("_valloc", |c| { let n = c.arg(); let p = c.st.heap.alloc(c.mem, n); c.ret(p) }),
    f!("___bzero", |c| { let (p, n) = (c.arg(), c.arg()); let _ = c.mem.write(p, &vec![0; n.min(1 << 32) as usize]); c.ret(0) }),
    f!("_memset_pattern16", |c| {
        let (p, pat, n) = (c.arg(), c.arg(), c.arg());
        let pat = c.mem.read_vec(pat, 16).unwrap_or_else(|_| vec![0; 16]);
        let v: Vec<u8> = (0..n.min(1 << 30) as usize).map(|i| pat[i % 16]).collect();
        let _ = c.mem.write(p, &v);
        c.ret(0)
    }),
    // strings
    f!("___strlcpy_chk", |c| { let (d, s, n) = (c.arg(), c.arg(), c.arg()); let v = c.cstr(s); crate::libsys::copy_out(c, d, n, &v); c.ret(v.len() as u64) }),
    f!("___strlcat_chk", |c| {
        let (d, s, n) = (c.arg(), c.arg(), c.arg());
        let mut v = c.cstr(d);
        let src = c.cstr(s);
        let total = v.len() + src.len();
        v.extend(src);
        crate::libsys::copy_out(c, d, n, &v);
        c.ret(total as u64)
    }),
    f!("___strncpy_chk", |c| { let (d, s, n) = (c.arg(), c.arg(), c.arg()); let mut v = read_cbytes(c.mem, s, n as usize); v.resize(n.min(1 << 30) as usize, 0); let _ = c.mem.write(d, &v); c.ret(d) }),
    f!("___strncat_chk", |c| { let (d, s, n) = (c.arg(), c.arg(), c.arg()); let at = d + c.cstr(d).len() as u64; let mut v = read_cbytes(c.mem, s, n as usize); v.push(0); let _ = c.mem.write(at, &v); c.ret(d) }),
    f!("_strncat", |c| { let (d, s, n) = (c.arg(), c.arg(), c.arg()); let at = d + c.cstr(d).len() as u64; let mut v = read_cbytes(c.mem, s, n as usize); v.push(0); let _ = c.mem.write(at, &v); c.ret(d) }),
    f!("_strspn", |c| span(c, false)),
    f!("_strcspn", |c| span(c, true)),
    f!("_strpbrk", |c| { let (a, b) = (c.arg(), c.arg()); let (s, set) = (c.cstr(a), c.cstr(b)); let r = s.iter().position(|x| set.contains(x)).map(|i| a + i as u64).unwrap_or(0); c.ret(r) }),
    f!("_strndup", |c| { let (s, n) = (c.arg(), c.arg()); let mut v = read_cbytes(c.mem, s, n as usize); v.push(0); let p = c.st.heap.alloc(c.mem, v.len() as u64); if p != 0 { let _ = c.mem.write(p, &v); } c.ret(p) }),
    f!("_strcasestr", |c| {
        let (h, n) = (c.arg(), c.arg());
        let (hs, ns) = (c.cstr(h).to_ascii_lowercase(), c.cstr(n).to_ascii_lowercase());
        let r = if ns.is_empty() { Some(0) } else { hs.windows(ns.len()).position(|w| w == ns.as_slice()) };
        c.ret(r.map(|i| h + i as u64).unwrap_or(0))
    }),
    f!("_strsep", |c| {
        let (sp, d) = (c.arg(), c.arg());
        let s = c.mem.read_u64(sp).unwrap_or(0);
        if s == 0 {
            return c.ret(0);
        }
        let (body, delim) = (c.cstr(s), c.cstr(d));
        match body.iter().position(|x| delim.contains(x)) {
            Some(i) => { let _ = c.mem.write(s + i as u64, &[0]); let _ = c.mem.write_u64(sp, s + i as u64 + 1); }
            None => { let _ = c.mem.write_u64(sp, 0); }
        }
        c.ret(s)
    }),
    f!("_strtok_r", |c| { let (s, d, save) = (c.arg(), c.arg(), c.arg()); strtok(c, s, d, save) }),
    f!("_strtok", |c| { let (s, d) = (c.arg(), c.arg()); strtok(c, s, d, SHIM_DATA + D_STATIC + 0x108) }),
    f!("_strerror", |c| {
        let e = c.arg() & 0xffff_ffff;
        if let Some(p) = errmsg(c.mem, e) {
            return c.ret(p);
        }
        let p = SHIM_DATA + D_STATIC + 0x180;
        let _ = c.mem.write(p, format!("Unknown error: {}\0", e as i32).as_bytes());
        c.ret(p)
    }),
    f!("_strerror_r", |c| {
        let (e, buf, n) = (c.arg() & 0xffff_ffff, c.arg(), c.arg());
        let msg = errors().nth(e as usize).map(String::from).unwrap_or_else(|| format!("Unknown error: {}", e as i32));
        crate::libsys::copy_out(c, buf, n, msg.as_bytes());
        c.ret(if (msg.len() as u64) < n { 0 } else { 34 })
    }),
    // file status
    f!("_stat$INODE64", |c| { let (p, b) = (c.arg(), c.arg()); stat_call(c, 4, p, 0, 0, b) }),
    f!("_stat", |c| { let (p, b) = (c.arg(), c.arg()); stat_call(c, 4, p, 0, 0, b) }),
    f!("_lstat$INODE64", |c| { let (p, b) = (c.arg(), c.arg()); stat_call(c, 6, p, 0, 0, b) }),
    f!("_lstat", |c| { let (p, b) = (c.arg(), c.arg()); stat_call(c, 6, p, 0, 0, b) }),
    f!("_fstat$INODE64", |c| { let (fd, b) = (c.arg(), c.arg()); stat_call(c, 5, fd, 0, 0, b) }),
    f!("_fstat", |c| { let (fd, b) = (c.arg(), c.arg()); stat_call(c, 5, fd, 0, 0, b) }),
    f!("_fstatat$INODE64", |c| {
        let (fd, p, b, fl) = (c.arg(), c.arg(), c.arg(), c.arg());
        let lfl = if fl & 0x20 != 0 { 0x100 } else { 0 }; // AT_SYMLINK_NOFOLLOW
        stat_call(c, 262, dirfd(fd), p, lfl, b)
    }),
    // descriptors and files
    f!("_fcntl", |c| {
        let (fd, cmd) = (c.arg(), c.arg());
        c.abi.va_start();
        let a = c.arg();
        match cmd {
            0..=2 => Act::Linux(72, [fd, cmd, a, 0, 0, 0], Post::Libc),
            3 => Act::Linux(72, [fd, 3, 0, 0, 0, 0], Post::Cont(K_GETFL, [0; 3])),
            4 => Act::Linux(72, [fd, 4, open_flags(a), 0, 0, 0], Post::Libc),
            67 => Act::Linux(72, [fd, 1030, a, 0, 0, 0], Post::Libc), // F_DUPFD_CLOEXEC
            48 | 51 => Act::Linux(74, [fd, 0, 0, 0, 0, 0], Post::Libc), // F_NOCACHE (advice) / F_FULLFSYNC
            _ => { c.set_errno(22); c.ret(u64::MAX) }
        }
    }),
    f!("_pipe", |c| { let p = c.arg(); Act::Linux(22, [p, 0, 0, 0, 0, 0], Post::Libc) }),
    f!("_umask", |c| { let m = c.arg(); Act::Linux(95, [m, 0, 0, 0, 0, 0], Post::Raw) }),
    f!("_chmod", |c| { let (p, m) = (c.arg(), c.arg()); Act::Linux(90, [p, m, 0, 0, 0, 0], Post::Libc) }),
    f!("_fchmod", |c| { let (fd, m) = (c.arg(), c.arg()); Act::Linux(91, [fd, m, 0, 0, 0, 0], Post::Libc) }),
    f!("_chown", |c| { let (p, u, g) = (c.arg(), c.arg(), c.arg()); Act::Linux(92, [p, u, g, 0, 0, 0], Post::Libc) }),
    f!("_fchown", |c| { let (fd, u, g) = (c.arg(), c.arg(), c.arg()); Act::Linux(93, [fd, u, g, 0, 0, 0], Post::Libc) }),
    f!("_ftruncate", |c| { let (fd, n) = (c.arg(), c.arg()); Act::Linux(77, [fd, n, 0, 0, 0, 0], Post::Libc) }),
    f!("_truncate", |c| { let (p, n) = (c.arg(), c.arg()); Act::Linux(76, [p, n, 0, 0, 0, 0], Post::Libc) }),
    f!("_fsync", |c| { let fd = c.arg(); Act::Linux(74, [fd, 0, 0, 0, 0, 0], Post::Libc) }),
    f!("_readlink", |c| { let (p, b, n) = (c.arg(), c.arg(), c.arg()); Act::Linux(89, [p, b, n, 0, 0, 0], Post::Libc) }),
    f!("_symlink", |c| { let (a, b) = (c.arg(), c.arg()); Act::Linux(88, [a, b, 0, 0, 0, 0], Post::Libc) }),
    f!("_link", |c| { let (a, b) = (c.arg(), c.arg()); Act::Linux(86, [a, b, 0, 0, 0, 0], Post::Libc) }),
    f!("_remove", |c| { let p = c.arg(); Act::Linux(87, [p, 0, 0, 0, 0, 0], Post::Cont(K_REMOVE, [p, 0, 0])) }),
    f!("_pread", |c| { let (fd, b, n, o) = (c.arg(), c.arg(), c.arg(), c.arg()); Act::Linux(17, [fd, b, n, o, 0, 0], Post::Libc) }),
    f!("_pwrite", |c| { let (fd, b, n, o) = (c.arg(), c.arg(), c.arg(), c.arg()); Act::Linux(18, [fd, b, n, o, 0, 0], Post::Libc) }),
    f!("_writev", |c| { let (fd, v, n) = (c.arg(), c.arg(), c.arg()); Act::Linux(20, [fd, v, n, 0, 0, 0], Post::Libc) }),
    f!("_readv", |c| { let (fd, v, n) = (c.arg(), c.arg(), c.arg()); Act::Linux(19, [fd, v, n, 0, 0, 0], Post::Libc) }),
    f!("_poll", |c| { let (p, n, t) = (c.arg(), c.arg(), c.arg()); Act::Linux(7, [p, n, t as i32 as i64 as u64, 0, 0, 0], Post::Libc) }),
    f!("_mmap", |c| {
        let (a, n, prot, fl, fd, off) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg(), c.arg());
        Act::Linux(9, [a, n, prot, crate::darwin::mmap_flags(fl), fd, off], Post::Libc)
    }),
    f!("_munmap", |c| { let (a, n) = (c.arg(), c.arg()); Act::Linux(11, [a, n, 0, 0, 0, 0], Post::Libc) }),
    f!("_mprotect", |c| { let (a, n, p) = (c.arg(), c.arg(), c.arg()); Act::Linux(10, [a, n, p, 0, 0, 0], Post::Libc) }),
    f!("_madvise", |c| c.ret(0)),
    // processes
    f!("_waitpid", |c| { let (p, s, o) = (c.arg(), c.arg(), c.arg()); Act::Linux(61, [p as i32 as i64 as u64, s, o, 0, 0, 0], Post::Libc) }),
    f!("_wait", |c| { let s = c.arg(); Act::Linux(61, [u64::MAX, s, 0, 0, 0, 0], Post::Libc) }),
    f!("_wait4", |c| { let (p, s, o, r) = (c.arg(), c.arg(), c.arg(), c.arg()); Act::Linux(61, [p as i32 as i64 as u64, s, o, r, 0, 0], Post::Libc) }),
    f!("_kill", |c| { let (p, s) = (c.arg(), c.arg()); Act::Linux(62, [p as i32 as i64 as u64, signal_to_linux(s), 0, 0, 0, 0], Post::Libc) }),
    f!("_raise", |c| { let s = c.arg(); Act::Linux(39, [0; 6], Post::Cont(K_RAISE, [signal_to_linux(s), 0, 0])) }),
    f!("_alarm", |c| { let s = c.arg(); Act::Linux(37, [s, 0, 0, 0, 0, 0], Post::Raw) }),
    f!("_sched_yield", |c| { let _ = c; Act::Linux(24, [0; 6], Post::Libc) }),
    f!("_getrlimit", |c| { let (r, p) = (c.arg(), c.arg()); Act::Linux(97, [rlimit_to_linux(r), p, 0, 0, 0, 0], Post::Libc) }),
    f!("_setrlimit", |c| { let (r, p) = (c.arg(), c.arg()); Act::Linux(160, [rlimit_to_linux(r), p, 0, 0, 0, 0], Post::Libc) }),
    f!("_getpgrp", |c| { let _ = c; Act::Linux(111, [0; 6], Post::Raw) }),
    f!("_setsid", |c| { let _ = c; Act::Linux(112, [0; 6], Post::Libc) }),
    f!("_gethostname", |c| { let (p, n) = (c.arg(), c.arg()); crate::libsys::copy_out(c, p, n, b"loos"); c.ret(0) }),
    f!("_getlogin", |c| ret_str(c, "user")),
    f!("___assert_rtn", |c| {
        let (func, file, line, expr) = (c.arg(), c.arg(), c.arg(), c.arg());
        let (func, file, expr) = (c.cstr(func), c.cstr(file), c.cstr(expr));
        let s = |v: &[u8]| String::from_utf8_lossy(v).into_owned();
        Act::Abort(format!("Assertion failed: ({}), function {}, file {}, line {}.", s(&expr), s(&func), s(&file), line as i32))
    }),
    f!("__NSGetProgname", |c| c.ret(SHIM_DATA + 0x38)),
    f!("__NSGetExecutablePath", |c| {
        let (buf, sizep) = (c.arg(), c.arg());
        let apple = c.st.start_args[3];
        let first = c.mem.read_u64(apple).unwrap_or(0);
        let s = c.cstr(first);
        let path = s.strip_prefix(b"executable_path=").unwrap_or(&s).to_vec();
        let cap = c.mem.read_u32(sizep).unwrap_or(0) as usize;
        if path.len() + 1 > cap {
            let _ = c.mem.write_u32(sizep, path.len() as u32 + 1);
            return c.ret(u64::MAX);
        }
        crate::libsys::copy_out(c, buf, cap as u64, &path);
        c.ret(0)
    }),
    f!("_compat_mode", |c| { let (_f, m) = (c.arg(), c.arg()); let mode = c.cstr(m).to_ascii_lowercase(); c.ret((mode != b"legacy") as u64) }),
    f!("___darwin_check_fd_set_overflow", |c| { let fd = c.arg() as i32; c.ret((0..1024).contains(&fd) as u64) }),
    f!("dyld_stub_binder", |_c| Act::Abort("lazy binding stub called (Prism binds every import at load time)".into())),
    f!("_tcgetattr", |c| { let (fd, t) = (c.arg(), c.arg()); Act::Linux(16, [fd, 0x5401, SCRATCH, 0, 0, 0], Post::Cont(K_TCGET, [t, 0, 0])) }),
    f!("_tcsetattr", |c| { let (fd, act, t) = (c.arg(), c.arg(), c.arg()); crate::posix2::tcset(c, fd, act, t) }),
    f!("_ioctl", |c| { let (fd, req) = (c.arg(), c.arg()); c.abi.va_start(); let a = c.arg(); crate::posix2::ioctl(c, fd, req, a) }),
];

fn strtok(c: &mut Call, s: u64, d: u64, save: u64) -> Act {
    let mut p = if s != 0 { s } else { c.mem.read_u64(save).unwrap_or(0) };
    if p == 0 {
        return c.ret(0);
    }
    let delim = c.cstr(d);
    let body = c.cstr(p);
    let skip = body.iter().take_while(|x| delim.contains(x)).count();
    if skip == body.len() {
        let _ = c.mem.write_u64(save, 0);
        return c.ret(0);
    }
    p += skip as u64;
    match body[skip..].iter().position(|x| delim.contains(x)) {
        Some(i) => {
            let _ = c.mem.write(p + i as u64, &[0]);
            let _ = c.mem.write_u64(save, p + i as u64 + 1);
        }
        None => {
            let _ = c.mem.write_u64(save, 0);
        }
    }
    c.ret(p)
}
