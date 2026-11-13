//! Directories, terminals, time, environment and user database for Prism's libSystem.

#[allow(unused_imports)]
use alloc::{format, string::String, vec, vec::Vec};
use crate::libsys::{read_cbytes, Act, Call, Func, Post, State, Then, D_STATIC};
use crate::loader::{SCRATCH, SHIM_DATA};
use loos_lin::mem::UserMem;

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

const ST: u64 = SHIM_DATA + D_STATIC;

// ------------------------------------------------------------ directories
// DIR (heap): +0 fd, +4 position, +8 valid bytes, +16 getdents64 buffer, then one Darwin dirent.
const DIR_BUF: u64 = 16;
const DIR_BUF_LEN: u64 = 4096;
const DIR_ENT: u64 = DIR_BUF + DIR_BUF_LEN;
const DIR_SIZE: u64 = DIR_ENT + 1048;

fn new_dir(fd: u64, mem: &mut UserMem, st: &mut State) -> u64 {
    let p = st.heap.alloc(mem, DIR_SIZE);
    if p != 0 {
        let _ = mem.write(p, &[0; 16]);
        let _ = mem.write_u32(p, fd as u32);
    }
    p
}

pub fn opendir_done(fd: u64, mem: &mut UserMem, st: &mut State) -> Then {
    match new_dir(fd, mem, st) {
        0 => Then::Fail(12, 0),
        p => Then::Ret(p),
    }
}

/// Convert the next buffered Linux `dirent64` into the DIR's Darwin `struct dirent`.
fn emit(dir: u64, mem: &mut UserMem) -> Then {
    let pos = mem.read_u32(dir + 4).unwrap_or(0) as u64;
    let e = dir + DIR_BUF + pos;
    let ino = mem.read_u64(e).unwrap_or(0);
    let off = mem.read_u64(e + 8).unwrap_or(0);
    let reclen = mem.read_vec(e + 16, 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0) as u64;
    let ty = mem.read_vec(e + 18, 1).map(|b| b[0]).unwrap_or(0);
    let name = read_cbytes(mem, e + 19, 1023);
    let _ = mem.write_u32(dir + 4, (pos + reclen.max(1)) as u32);
    let d = dir + DIR_ENT;
    let rec = (21 + name.len() as u64 + 1 + 7) & !7;
    let mut out = Vec::with_capacity(rec as usize);
    out.extend(ino.to_le_bytes());
    out.extend(off.to_le_bytes());
    out.extend((rec as u16).to_le_bytes());
    out.extend((name.len() as u16).to_le_bytes());
    out.push(ty);
    out.extend(&name);
    out.resize(rec as usize, 0);
    match mem.write(d, &out) {
        Ok(()) => Then::Ret(d),
        Err(e) => Then::Fail(-e, 0),
    }
}

pub fn readdir_done(dir: u64, n: u64, mem: &mut UserMem) -> Then {
    let _ = mem.write_u32(dir + 4, 0);
    let _ = mem.write_u32(dir + 8, n as u32);
    emit(dir, mem)
}

fn readdir(c: &mut Call) -> Act {
    let dir = c.arg();
    let pos = c.mem.read_u32(dir + 4).unwrap_or(0);
    let len = c.mem.read_u32(dir + 8).unwrap_or(0);
    if pos < len {
        return match emit(dir, c.mem) {
            Then::Ret(p) => c.ret(p),
            _ => c.ret(0),
        };
    }
    let fd = c.mem.read_u32(dir).unwrap_or(0) as u64;
    Act::Linux(217, [fd, dir + DIR_BUF, DIR_BUF_LEN, 0, 0, 0], Post::Cont(6, [dir, 0, 0]))
}

// ------------------------------------------------------------ terminals
// Darwin termios: four 8-byte flag words, cc[20] at 32, speeds at 56/64 (72 bytes).
// Linux kernel termios: four u32 flag words, c_line at 16, cc[19] at 17 (36 bytes).
const IFLAG: &[(u64, u64)] = &[(1, 1), (2, 2), (0x10, 0x10), (0x20, 0x20), (0x40, 0x40), (0x80, 0x80), (0x100, 0x100), (0x200, 0x400), (0x400, 0x1000), (0x800, 0x800), (0x4000, 0x4000)];
const OFLAG: &[(u64, u64)] = &[(1, 1), (2, 4)];
const CFLAG: &[(u64, u64)] = &[(0x100, 0x10), (0x200, 0x20), (0x400, 0x40), (0x800, 0x80), (0x1000, 0x100), (0x2000, 0x200), (0x4000, 0x400), (0x8000, 0x800)];
const LFLAG: &[(u64, u64)] = &[(1, 0x800), (2, 0x10), (4, 0x20), (8, 8), (0x10, 0x40), (0x20, 0x400), (0x40, 0x200), (0x80, 1), (0x100, 2), (0x400, 0x8000), (0x40_0000, 0x100), (0x8000_0000, 0x80)];
const CC: &[(usize, usize)] = &[(0, 4), (1, 11), (2, 16), (3, 2), (4, 14), (5, 3), (6, 12), (8, 0), (9, 1), (10, 10), (12, 8), (13, 9), (14, 15), (15, 13), (16, 6), (17, 5)];

fn map_bits(v: u64, table: &[(u64, u64)], to_linux: bool) -> u64 {
    table.iter().filter(|(d, l)| v & if to_linux { *d } else { *l } != 0).fold(0, |acc, (d, l)| acc | if to_linux { *l } else { *d })
}

pub fn tcget_done(dst: u64, mem: &mut UserMem) -> Then {
    let Ok(l) = mem.read_vec(SCRATCH, 36) else { return Then::Fail(14, u64::MAX) };
    let w = |i: usize| u32::from_le_bytes(l[i * 4..i * 4 + 4].try_into().unwrap()) as u64;
    let mut d = vec![0u8; 72];
    for (i, t) in [IFLAG, OFLAG, CFLAG, LFLAG].iter().enumerate() {
        d[i * 8..i * 8 + 8].copy_from_slice(&map_bits(w(i), t, false).to_le_bytes());
    }
    for &(di, li) in CC {
        d[32 + di] = l[17 + li];
    }
    d[56..64].copy_from_slice(&38400u64.to_le_bytes());
    d[64..72].copy_from_slice(&38400u64.to_le_bytes());
    match mem.write(dst, &d) {
        Ok(()) => Then::Ret(0),
        Err(e) => Then::Fail(-e, u64::MAX),
    }
}

/// `tcsetattr(fd, when, termios)`: when = TCSANOW/TCSADRAIN/TCSAFLUSH (| TCSASOFT).
pub fn tcset(c: &mut Call, fd: u64, when: u64, t: u64) -> Act {
    let Ok(d) = c.mem.read_vec(t, 72) else { c.set_errno(14); return c.ret(u64::MAX) };
    let w = |i: usize| u64::from_le_bytes(d[i * 8..i * 8 + 8].try_into().unwrap());
    let mut l = vec![0u8; 36];
    for (i, tb) in [IFLAG, OFLAG, CFLAG, LFLAG].iter().enumerate() {
        l[i * 4..i * 4 + 4].copy_from_slice(&(map_bits(w(i), tb, true) as u32).to_le_bytes());
    }
    for &(di, li) in CC {
        l[17 + li] = d[32 + di];
    }
    let _ = c.mem.write(SCRATCH, &l);
    Act::Linux(16, [fd, 0x5402 + (when & 3).min(2), SCRATCH, 0, 0, 0], Post::Libc)
}

pub fn ioctl(c: &mut Call, fd: u64, req: u64, a: u64) -> Act {
    let lin = |nr: u64| Act::Linux(16, [fd, nr, a, 0, 0, 0], Post::Libc);
    match req & 0xffff_ffff {
        0x4008_7468 => lin(0x5413), // TIOCGWINSZ
        0x8008_7467 => lin(0x5414), // TIOCSWINSZ
        0x4004_667f => lin(0x541b), // FIONREAD
        0x8004_667e => lin(0x5421), // FIONBIO
        0x4004_7477 => lin(0x540f), // TIOCGPGRP
        0x8004_7476 => lin(0x5410), // TIOCSPGRP
        0x2000_7461 => lin(0x540e), // TIOCSCTTY
        0x4048_7413 => Act::Linux(16, [fd, 0x5401, SCRATCH, 0, 0, 0], Post::Cont(7, [a, 0, 0])),
        r @ 0x8048_7414..=0x8048_7416 => tcset(c, fd, r - 0x8048_7414, a),
        _ => {
            c.set_errno(25); // ENOTTY
            c.ret(u64::MAX)
        }
    }
}

// ------------------------------------------------------------ time (UTC; LoOS keeps no time zones)
// struct tm: sec min hour mday mon year wday yday isdst (ints), gmtoff at 40, zone at 48.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
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

/// Broken-down UTC time of `t`: [sec, min, hour, mday, mon, year-1900, wday, yday, isdst].
pub fn tm_of(t: i64) -> [i64; 9] {
    let days = t.div_euclid(86400);
    let s = t.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let yday = days - days_from_civil(y, 1, 1);
    [s % 60, s / 60 % 60, s / 3600, d, m - 1, y - 1900, (days + 4).rem_euclid(7), yday, 0]
}

fn write_tm(mem: &mut UserMem, p: u64, tm: &[i64; 9]) {
    for (i, v) in tm.iter().enumerate() {
        let _ = mem.write_u32(p + i as u64 * 4, *v as i32 as u32);
    }
    let zone = ST + 0x110;
    let _ = mem.write(zone, b"UTC\0");
    let _ = mem.write_u64(p + 40, 0);
    let _ = mem.write_u64(p + 48, zone);
}

fn read_tm(mem: &mut UserMem, p: u64) -> [i64; 9] {
    let mut tm = [0i64; 9];
    for (i, v) in tm.iter_mut().enumerate() {
        *v = mem.read_u32(p + i as u64 * 4).unwrap_or(0) as i32 as i64;
    }
    tm
}

/// Seconds since the epoch for a (possibly unnormalized) UTC `struct tm`.
fn timegm(tm: &[i64; 9]) -> i64 {
    let mon = tm[4];
    let y = tm[5] + 1900 + mon.div_euclid(12);
    let m = mon.rem_euclid(12) + 1;
    (days_from_civil(y, m, 1) + tm[3] - 1) * 86400 + tm[2] * 3600 + tm[1] * 60 + tm[0]
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn asctime(tm: &[i64; 9]) -> String {
    format!("{} {} {:2} {:02}:{:02}:{:02} {}\n", &DAYS[tm[6].rem_euclid(7) as usize][..3], &MONTHS[tm[4].rem_euclid(12) as usize][..3], tm[3], tm[2], tm[1], tm[0], tm[5] + 1900)
}

/// strftime conversions (C/POSIX locale, UTC).
pub fn strftime(fmt: &[u8], tm: &[i64; 9]) -> Vec<u8> {
    let mut o = String::new();
    let mut i = 0;
    while i < fmt.len() {
        let ch = fmt[i];
        i += 1;
        if ch != b'%' || i >= fmt.len() {
            o.push(ch as char);
            continue;
        }
        let mut k = fmt[i];
        i += 1;
        if (k == b'E' || k == b'O') && i < fmt.len() {
            k = fmt[i];
            i += 1;
        }
        let h12 = if tm[2] % 12 == 0 { 12 } else { tm[2] % 12 };
        let s = match k {
            b'a' => String::from(&DAYS[tm[6].rem_euclid(7) as usize][..3]),
            b'A' => String::from(DAYS[tm[6].rem_euclid(7) as usize]),
            b'b' | b'h' => String::from(&MONTHS[tm[4].rem_euclid(12) as usize][..3]),
            b'B' => String::from(MONTHS[tm[4].rem_euclid(12) as usize]),
            b'c' => String::from_utf8_lossy(&strftime(b"%a %b %e %H:%M:%S %Y", tm)).into_owned(),
            b'C' => format!("{:02}", (tm[5] + 1900) / 100),
            b'd' => format!("{:02}", tm[3]),
            b'D' | b'x' => String::from_utf8_lossy(&strftime(b"%m/%d/%y", tm)).into_owned(),
            b'e' => format!("{:2}", tm[3]),
            b'F' => String::from_utf8_lossy(&strftime(b"%Y-%m-%d", tm)).into_owned(),
            b'H' => format!("{:02}", tm[2]),
            b'I' => format!("{:02}", h12),
            b'j' => format!("{:03}", tm[7] + 1),
            b'k' => format!("{:2}", tm[2]),
            b'l' => format!("{:2}", h12),
            b'm' => format!("{:02}", tm[4] + 1),
            b'M' => format!("{:02}", tm[1]),
            b'n' => String::from("\n"),
            b'p' => String::from(if tm[2] < 12 { "AM" } else { "PM" }),
            b'r' => String::from_utf8_lossy(&strftime(b"%I:%M:%S %p", tm)).into_owned(),
            b'R' => String::from_utf8_lossy(&strftime(b"%H:%M", tm)).into_owned(),
            b's' => format!("{}", timegm(tm)),
            b'S' => format!("{:02}", tm[0]),
            b't' => String::from("\t"),
            b'T' | b'X' => String::from_utf8_lossy(&strftime(b"%H:%M:%S", tm)).into_owned(),
            b'u' => format!("{}", if tm[6] == 0 { 7 } else { tm[6] }),
            b'w' => format!("{}", tm[6]),
            b'y' => format!("{:02}", (tm[5] + 1900).rem_euclid(100)),
            b'Y' => format!("{}", tm[5] + 1900),
            b'z' => String::from("+0000"),
            b'Z' => String::from("UTC"),
            b'%' => String::from("%"),
            other => format!("%{}", other as char),
        };
        o.push_str(&s);
    }
    o.into_bytes()
}

fn time_arg(c: &mut Call) -> i64 {
    let p = c.arg();
    c.mem.read_u64(p).unwrap_or(0) as i64
}

// ------------------------------------------------------------ environment
const D_ENVIRON: u64 = 0x20;

fn env_list(mem: &mut UserMem) -> Vec<u64> {
    let mut v = Vec::new();
    let mut p = mem.read_u64(SHIM_DATA + D_ENVIRON).unwrap_or(0);
    while p != 0 {
        match mem.read_u64(p) {
            Ok(0) | Err(_) => break,
            Ok(e) => v.push(e),
        }
        p += 8;
    }
    v
}

fn env_find(mem: &mut UserMem, list: &[u64], key: &[u8]) -> Option<usize> {
    list.iter().position(|&e| {
        let s = read_cbytes(mem, e, key.len() + 1);
        s.len() > key.len() && s.starts_with(key) && s[key.len()] == b'='
    })
}

/// Install a new `environ` array (the strings stay where they are).
fn env_store(c: &mut Call, list: &[u64]) -> bool {
    let p = c.st.heap.alloc(c.mem, (list.len() as u64 + 1) * 8);
    if p == 0 {
        return false;
    }
    let mut bytes: Vec<u8> = list.iter().flat_map(|e| e.to_le_bytes()).collect();
    bytes.extend([0; 8]);
    c.mem.write(p, &bytes).is_ok() && c.mem.write_u64(SHIM_DATA + D_ENVIRON, p).is_ok()
}

fn setenv(c: &mut Call, entry: u64, key: &[u8], overwrite: bool) -> Act {
    if key.is_empty() || key.contains(&b'=') {
        c.set_errno(22);
        return c.ret(u64::MAX);
    }
    let mut list = env_list(c.mem);
    match env_find(c.mem, &list, key) {
        Some(_) if !overwrite => return c.ret(0),
        Some(i) => list[i] = entry,
        None => list.push(entry),
    }
    if env_store(c, &list) { c.ret(0) } else { c.set_errno(12); c.ret(u64::MAX) }
}

// ------------------------------------------------------------ users and groups
// struct passwd: name, passwd, uid, gid, change, class, gecos, dir, shell, expire (72 bytes).
fn passwd(c: &mut Call, uid: u32) -> u64 {
    let (name, gid, home, gecos) = if uid == 0 { ("root", 0, "/var/root", "System Administrator") } else { ("user", 20, "/home", "User") };
    let p = ST + 0x400;
    let mut at = ST + 0x450;
    let mut s = |mem: &mut UserMem, v: &str| {
        let a = at;
        let _ = mem.write(a, v.as_bytes());
        let _ = mem.write(a + v.len() as u64, &[0]);
        at += v.len() as u64 + 1;
        a
    };
    let (n, pw, cl, ge, di, sh) = (s(c.mem, name), s(c.mem, "*"), s(c.mem, ""), s(c.mem, gecos), s(c.mem, home), s(c.mem, "/bin/sh"));
    for (o, v) in [(0, n), (8, pw), (24, 0), (32, cl), (40, ge), (48, di), (56, sh), (64, 0)] {
        let _ = c.mem.write_u64(p + o, v);
    }
    let _ = c.mem.write_u32(p + 16, uid);
    let _ = c.mem.write_u32(p + 20, gid);
    p
}

// struct group: name, passwd, gid, members (32 bytes).
fn group(c: &mut Call, gid: u32) -> u64 {
    let name: &[u8] = if gid == 0 { b"wheel\0" } else { b"staff\0" };
    let p = ST + 0x500;
    let members = ST + 0x520;
    let (n, pw) = (ST + 0x540, ST + 0x550);
    let _ = c.mem.write(n, name);
    let _ = c.mem.write(pw, b"*\0");
    let _ = c.mem.write_u64(members, 0);
    for (o, v) in [(0, n), (8, pw), (24, members)] {
        let _ = c.mem.write_u64(p + o, v);
    }
    let _ = c.mem.write_u64(p + 16, gid as u64);
    p
}

fn path_part(c: &mut Call, dir: bool) -> Act {
    let p = c.arg();
    let s = if p == 0 { Vec::new() } else { c.cstr(p) };
    let trimmed = {
        let mut e = s.len();
        while e > 1 && s[e - 1] == b'/' {
            e -= 1;
        }
        &s[..e]
    };
    let out: Vec<u8> = if trimmed.is_empty() {
        b".".to_vec()
    } else if dir {
        match trimmed.iter().rposition(|&b| b == b'/') {
            None => b".".to_vec(),
            Some(i) => {
                let mut e = i;
                while e > 0 && trimmed[e - 1] == b'/' {
                    e -= 1;
                }
                if e == 0 { b"/".to_vec() } else { trimmed[..e].to_vec() }
            }
        }
    } else if trimmed == b"/" {
        b"/".to_vec()
    } else {
        trimmed[trimmed.iter().rposition(|&b| b == b'/').map(|i| i + 1).unwrap_or(0)..].to_vec()
    };
    let buf = ST + if dir { 0xc00 } else { 0x800 };
    let mut v = out;
    v.truncate(1023);
    v.push(0);
    let _ = c.mem.write(buf, &v);
    c.ret(buf)
}

pub static FUNCS: &[(&str, Func)] = &[
    // directories
    f!("_opendir$INODE64", |c| { let p = c.arg(); Act::Linux(2, [p, 0x9_0000, 0, 0, 0, 0], Post::Cont(5, [0; 3])) }),
    f!("_opendir", |c| { let p = c.arg(); Act::Linux(2, [p, 0x9_0000, 0, 0, 0, 0], Post::Cont(5, [0; 3])) }),
    f!("_fdopendir$INODE64", |c| { let fd = c.arg(); let d = new_dir(fd, c.mem, c.st); c.ret(d) }),
    f!("_readdir$INODE64", |c| readdir(c)),
    f!("_readdir", |c| readdir(c)),
    f!("_closedir", |c| { let d = c.arg(); let fd = c.mem.read_u32(d).unwrap_or(0) as u64; c.st.heap.free(c.mem, d); Act::Linux(3, [fd, 0, 0, 0, 0, 0], Post::Libc) }),
    f!("_rewinddir$INODE64", |c| { let d = c.arg(); let fd = c.mem.read_u32(d).unwrap_or(0) as u64; let _ = c.mem.write(d + 4, &[0; 8]); Act::Linux(8, [fd, 0, 0, 0, 0, 0], Post::Value(0)) }),
    f!("_dirfd", |c| { let d = c.arg(); let fd = c.mem.read_u32(d).unwrap_or(0) as u64; c.ret(fd) }),
    // time
    f!("_gmtime", |c| { let t = time_arg(c); write_tm(c.mem, ST, &tm_of(t)); c.ret(ST) }),
    f!("_localtime", |c| { let t = time_arg(c); write_tm(c.mem, ST, &tm_of(t)); c.ret(ST) }),
    f!("_gmtime_r", |c| { let t = time_arg(c); let p = c.arg(); write_tm(c.mem, p, &tm_of(t)); c.ret(p) }),
    f!("_localtime_r", |c| { let t = time_arg(c); let p = c.arg(); write_tm(c.mem, p, &tm_of(t)); c.ret(p) }),
    f!("_mktime", |c| { let p = c.arg(); let t = timegm(&read_tm(c.mem, p)); write_tm(c.mem, p, &tm_of(t)); c.ret(t as u64) }),
    f!("_timegm", |c| { let p = c.arg(); let t = timegm(&read_tm(c.mem, p)); write_tm(c.mem, p, &tm_of(t)); c.ret(t as u64) }),
    f!("_asctime", |c| { let p = c.arg(); let s = asctime(&read_tm(c.mem, p)); let b = ST + 0x80; crate::libsys::copy_out(c, b, 32, s.as_bytes()); c.ret(b) }),
    f!("_asctime_r", |c| { let (p, b) = (c.arg(), c.arg()); let s = asctime(&read_tm(c.mem, p)); crate::libsys::copy_out(c, b, 26, s.as_bytes()); c.ret(b) }),
    f!("_ctime", |c| { let t = time_arg(c); let s = asctime(&tm_of(t)); let b = ST + 0x80; crate::libsys::copy_out(c, b, 32, s.as_bytes()); c.ret(b) }),
    f!("_ctime_r", |c| { let t = time_arg(c); let b = c.arg(); let s = asctime(&tm_of(t)); crate::libsys::copy_out(c, b, 26, s.as_bytes()); c.ret(b) }),
    f!("_strftime", |c| {
        let (b, max, f, p) = (c.arg(), c.arg(), c.arg(), c.arg());
        let fmt = c.cstr(f);
        let tm = read_tm(c.mem, p);
        let s = strftime(&fmt, &tm);
        if (s.len() as u64) < max {
            crate::libsys::copy_out(c, b, max, &s);
            c.ret(s.len() as u64)
        } else {
            c.ret(0)
        }
    }),
    f!("_tzset", |c| c.ret(0)),
    // environment
    f!("_setenv", |c| {
        let (k, v, ow) = (c.arg(), c.arg(), c.arg());
        let key = c.cstr(k);
        let mut e = key.clone();
        e.push(b'=');
        e.extend(c.cstr(v));
        e.push(0);
        let p = c.st.heap.alloc(c.mem, e.len() as u64);
        if p == 0 || c.mem.write(p, &e).is_err() {
            c.set_errno(12);
            return c.ret(u64::MAX);
        }
        setenv(c, p, &key, ow != 0)
    }),
    f!("_putenv", |c| {
        let p = c.arg();
        let s = c.cstr(p);
        let Some(eq) = s.iter().position(|&b| b == b'=') else { c.set_errno(22); return c.ret(u64::MAX) };
        let key = s[..eq].to_vec();
        setenv(c, p, &key, true)
    }),
    f!("_unsetenv", |c| {
        let k = c.arg();
        let key = c.cstr(k);
        let mut list = env_list(c.mem);
        while let Some(i) = env_find(c.mem, &list, &key) {
            list.remove(i);
        }
        if env_store(c, &list) { c.ret(0) } else { c.set_errno(12); c.ret(u64::MAX) }
    }),
    // users and groups
    f!("_getpwuid", |c| { let u = c.arg() as u32; if u == 0 || u == 501 { let p = passwd(c, u); c.ret(p) } else { c.ret(0) } }),
    f!("_getpwnam", |c| {
        let n = c.arg();
        let name = c.cstr(n);
        match name.as_slice() {
            b"root" => { let p = passwd(c, 0); c.ret(p) }
            b"user" => { let p = passwd(c, 501); c.ret(p) }
            _ => c.ret(0),
        }
    }),
    f!("_getgrgid", |c| { let g = c.arg() as u32; if g == 0 || g == 20 { let p = group(c, g); c.ret(p) } else { c.ret(0) } }),
    f!("_getgrnam", |c| {
        let n = c.arg();
        let name = c.cstr(n);
        match name.as_slice() {
            b"wheel" => { let p = group(c, 0); c.ret(p) }
            b"staff" => { let p = group(c, 20); c.ret(p) }
            _ => c.ret(0),
        }
    }),
    // paths
    f!("_basename", |c| path_part(c, false)),
    f!("_dirname", |c| path_part(c, true)),
];
