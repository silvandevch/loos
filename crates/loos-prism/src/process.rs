//! Process creation: `fork`, the `exec` family and `posix_spawn`.
//!
//! An `execve` of a Mach-O is finished by Prism itself (`Sys::ExecForeign`); a fork is followed by
//! giving the child a copy of the parent's Prism state (`lib.rs`, `Event::Forked`).
//! `posix_spawn` forks; the child runs the file actions and the exec in continuations (it never
//! executes program code), and exits with 127 if any of that fails.

#[allow(unused_imports)]
use alloc::{vec, vec::Vec};
use crate::darwin::{linux_to_darwin_errno, open_flags};
use crate::libsys::{read_cbytes, Act, Call, Func, Post, State, Then, D_ENVIRON};
use crate::loader::{SCRATCH, SHIM_DATA};
use loos_lin::mem::UserMem;

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

pub(crate) const K_EXEC: u8 = 30;
pub(crate) const K_SPAWN: u8 = 31;
pub(crate) const K_SPAWN_ACT: u8 = 32;
pub(crate) const K_SPAWN_PIPE: u8 = 33;
pub(crate) const K_SPAWN_WAIT: u8 = 34;
pub(crate) const K_SPAWN_ATTR: u8 = 35;

/// Exec request block (heap): file, argv, envp, search PATH?, exit 127 on failure?, pid out,
/// file actions, scratch.
const B_FILE: u64 = 0;
const B_ARGV: u64 = 8;
const B_ENVP: u64 = 16;
const B_SEARCH: u64 = 24;
const B_EXIT: u64 = 32;
const B_PIDP: u64 = 40;
const B_ACTS: u64 = 48;
const B_TMP: u64 = 56;
/// posix_spawn: the status pipe (two i32: read, write end), the child's errno, its pid, the attributes.
const B_PIPE: u64 = 64;
const B_ERR: u64 = 72;
const B_PID: u64 = 80;
const B_ATTR: u64 = 88;
const B_LEN: u64 = 96;

/// `posix_spawnattr_t` (heap): flags, process group, signal mask, signals to default (Darwin sets).
const A_FLAGS: u64 = 0;
const A_PGROUP: u64 = 8;
const A_MASK: u64 = 16;
const A_DEFAULT: u64 = 24;
const A_LEN: u64 = 32;
const SETPGROUP: u64 = 2;
const SETSIGDEF: u64 = 4;
const SETSIGMASK: u64 = 8;
const SETSID: u64 = 0x400;

/// Where candidate paths are built.
const CAND: u64 = SCRATCH + 0xc000;

/// File action list (heap): count, then records of 6 words: kind, fd, arg, mode, path, unused.
const ACT_MAX: u64 = 64;
const ACT_WORDS: u64 = 6;
const FA_OPEN: u64 = 1;
const FA_CLOSE: u64 = 2;
const FA_DUP2: u64 = 3;
const FA_CHDIR: u64 = 4;
const FA_FCHDIR: u64 = 5;

fn environ(mem: &mut UserMem) -> u64 {
    mem.read_u64(SHIM_DATA + D_ENVIRON).unwrap_or(0)
}

fn getenv(mem: &mut UserMem, key: &[u8]) -> Option<Vec<u8>> {
    let mut e = environ(mem);
    loop {
        let p = mem.read_u64(e).ok()?;
        if p == 0 {
            return None;
        }
        let s = read_cbytes(mem, p, 1 << 16);
        if s.len() > key.len() && s.starts_with(key) && s[key.len()] == b'=' {
            return Some(s[key.len() + 1..].to_vec());
        }
        e += 8;
    }
}

fn block(c: &mut Call, file: u64, argv: u64, envp: u64, search: bool, exit: bool) -> u64 {
    let b = c.st.heap.alloc(c.mem, B_LEN);
    let envp = if envp == 0 { environ(c.mem) } else { envp };
    for (o, v) in [(B_FILE, file), (B_ARGV, argv), (B_ENVP, envp), (B_SEARCH, search as u64), (B_EXIT, exit as u64), (B_PIDP, 0), (B_ACTS, 0), (B_TMP, 0)] {
        let _ = c.mem.write_u64(b + o, v);
    }
    b
}

fn rd(mem: &mut UserMem, b: u64, o: u64) -> u64 {
    mem.read_u64(b + o).unwrap_or(0)
}

/// Try candidate number `idx` of the exec in block `b`.
fn exec_try(mem: &mut UserMem, b: u64, idx: u64, last_err: i64) -> Then {
    let file = rd(mem, b, B_FILE);
    let name = read_cbytes(mem, file, 1024);
    let (argv, envp) = (rd(mem, b, B_ARGV), rd(mem, b, B_ENVP));
    if name.is_empty() {
        return fail(mem, b, 2);
    }
    let path = if rd(mem, b, B_SEARCH) == 0 || name.contains(&b'/') {
        if idx > 0 {
            return fail(mem, b, last_err);
        }
        file
    } else {
        let paths = getenv(mem, b"PATH").unwrap_or_else(|| b"/usr/bin:/bin:/usr/sbin:/sbin".to_vec());
        let Some(dir) = paths.split(|&ch| ch == b':').nth(idx as usize) else { return fail(mem, b, last_err) };
        let mut cand = if dir.is_empty() { b".".to_vec() } else { dir.to_vec() };
        cand.push(b'/');
        cand.extend_from_slice(&name);
        cand.push(0);
        let _ = mem.write(CAND, &cand);
        CAND
    };
    Then::Linux(59, [path, argv, envp, 0, 0, 0], Post::Cont(K_EXEC, [b, idx + 1, 0]))
}

fn fail(mem: &mut UserMem, b: u64, e: i64) -> Then {
    if rd(mem, b, B_EXIT) != 0 {
        // A posix_spawn child: tell the parent why (status pipe), then exit.
        let _ = mem.write_u32(b + B_ERR, e as u32);
        let w = rd(mem, b, B_PIPE) >> 32;
        Then::Linux(1, [w, b + B_ERR, 4, 0, 0, 0], Post::Cont(K_SPAWN_ACT, [b, u64::MAX, 0]))
    } else {
        Then::Fail(e, u64::MAX)
    }
}

/// The child of `posix_spawn`: run file action `i` (phase in ctx), then exec.
fn spawn_step(mem: &mut UserMem, b: u64, i: u64, phase: u64, v: i64) -> Then {
    let acts = rd(mem, b, B_ACTS);
    let n = if acts != 0 { rd(mem, acts, 0) } else { 0 };
    if i >= n {
        return exec_try(mem, b, 0, 2);
    }
    let r = acts + 8 + i * ACT_WORDS * 8;
    let (kind, fd, arg, mode, path) = (rd(mem, r, 0), rd(mem, r, 8), rd(mem, r, 16), rd(mem, r, 24), rd(mem, r, 32));
    let next = |ph: u64| Post::Cont(K_SPAWN_ACT, [b, i, ph]);
    match (kind, phase) {
        (FA_OPEN, 0) => Then::Linux(257, [(-100i64) as u64, path, open_flags(arg), mode, 0, 0], next(1)),
        (FA_OPEN, 1) if v as u64 == fd => spawn_step(mem, b, i + 1, 0, 0),
        (FA_OPEN, 1) => {
            let _ = mem.write_u64(b + B_TMP, v as u64);
            Then::Linux(33, [v as u64, fd, 0, 0, 0, 0], next(2))
        }
        (FA_OPEN, 2) => Then::Linux(3, [rd(mem, b, B_TMP), 0, 0, 0, 0, 0], next(3)),
        (FA_CLOSE, 0) => Then::Linux(3, [fd, 0, 0, 0, 0, 0], next(3)),
        (FA_DUP2, 0) if fd == arg => Then::Linux(72, [fd, 2, 0, 0, 0, 0], next(3)), // clear FD_CLOEXEC
        (FA_DUP2, 0) => Then::Linux(33, [fd, arg, 0, 0, 0, 0], next(3)),
        (FA_CHDIR, 0) => Then::Linux(80, [path, 0, 0, 0, 0, 0], next(3)),
        (FA_FCHDIR, 0) => Then::Linux(81, [fd, 0, 0, 0, 0, 0], next(3)),
        _ => spawn_step(mem, b, i + 1, 0, 0),
    }
}

pub fn cont(id: u8, ctx: [u64; 3], v: i64, mem: &mut UserMem, _st: &mut State) -> Option<Then> {
    Some(match id {
        K_EXEC => {
            // Only failures come back here.
            let e = -v;
            if matches!(e, 2 | 13 | 20) {
                exec_try(mem, ctx[0], ctx[1], e)
            } else {
                fail(mem, ctx[0], e)
            }
        }
        K_SPAWN if v < 0 => Then::Ret(linux_to_darwin_errno(-v) as u64), // fork failed: the error is the result
        K_SPAWN if v > 0 => {
            // Parent: close our copy of the write end, then wait for exec (EOF) or an errno.
            let b = ctx[0];
            let _ = mem.write_u64(b + B_PID, v as u64);
            let w = rd(mem, b, B_PIPE) >> 32;
            Then::Linux(3, [w, 0, 0, 0, 0, 0], Post::Cont(K_SPAWN_ATTR, [b, u64::MAX, 0]))
        }
        K_SPAWN => attr_step(mem, ctx[0], 0, 1), // the child
        K_SPAWN_ACT if ctx[1] == u64::MAX => Then::Linux(231, [127, 0, 0, 0, 0, 0], Post::Raw),
        K_SPAWN_ACT if v < 0 => fail(mem, ctx[0], -v),
        K_SPAWN_ATTR if ctx[1] == u64::MAX => {
            let b = ctx[0];
            let _ = mem.write_u32(b + B_ERR, 0);
            Then::Linux(0, [rd(mem, b, B_PIPE) & 0xffff_ffff, b + B_ERR, 4, 0, 0, 0], Post::Cont(K_SPAWN_WAIT, [b, 0, 0]))
        }
        // ENOSYS: the Linux layer has no process groups or sessions; there is nothing to apply.
        K_SPAWN_ATTR if v < 0 && v != -38 => fail(mem, ctx[0], -v),
        K_SPAWN_ATTR => attr_step(mem, ctx[0], ctx[1], ctx[2]),
        K_SPAWN_PIPE if v < 0 => Then::Ret(linux_to_darwin_errno(-v) as u64),
        K_SPAWN_PIPE => Then::Linux(57, [0; 6], Post::Cont(K_SPAWN, [ctx[0], 0, 0])),
        // Parent, after reading the status pipe: 0 bytes = the exec happened.
        K_SPAWN_WAIT if ctx[1] == 0 => {
            let b = ctx[0];
            let r = rd(mem, b, B_PIPE) & 0xffff_ffff;
            let n = v;
            Then::Linux(3, [r, 0, 0, 0, 0, 0], Post::Cont(K_SPAWN_WAIT, [b, 1, (n == 4) as u64]))
        }
        K_SPAWN_WAIT if ctx[1] == 1 && ctx[2] == 1 => {
            // The exec failed: reap the child, report its errno.
            Then::Linux(61, [rd(mem, ctx[0], B_PID), 0, 0, 0, 0, 0], Post::Cont(K_SPAWN_WAIT, [ctx[0], 2, 0]))
        }
        K_SPAWN_WAIT if ctx[1] == 2 => Then::Ret(linux_to_darwin_errno(mem.read_u32(ctx[0] + B_ERR).unwrap_or(2) as i64) as u64),
        K_SPAWN_WAIT => {
            let p = rd(mem, ctx[0], B_PIDP);
            if p != 0 {
                let pid = rd(mem, ctx[0], B_PID) as u32;
                let _ = mem.write_u32(p, pid);
            }
            Then::Ret(0)
        }
        K_SPAWN_ACT if ctx[2] == 3 => spawn_step(mem, ctx[0], ctx[1] + 1, 0, 0),
        K_SPAWN_ACT => spawn_step(mem, ctx[0], ctx[1], ctx[2], v),
        _ => return None,
    })
}

fn start(c: &mut Call, t: Then) -> Act {
    match t {
        Then::Linux(nr, a, p) => Act::Linux(nr, a, p),
        Then::Ret(v) => c.ret(v),
        Then::Fail(e, r) => {
            c.set_errno(e);
            c.ret(r)
        }
    }
}

fn exec(c: &mut Call, envp: Option<u64>, search: bool) -> Act {
    let (file, argv) = (c.arg(), c.arg());
    let envp = envp.unwrap_or_else(|| c.arg());
    let b = block(c, file, argv, envp, search, false);
    let t = exec_try(c.mem, b, 0, 2);
    start(c, t)
}

fn spawn(c: &mut Call, search: bool) -> Act {
    let (pidp, path, fa, attr, argv, envp) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg(), c.arg());
    let b = block(c, path, argv, envp, search, true);
    let _ = c.mem.write_u64(b + B_PIDP, pidp);
    let acts = if fa != 0 { c.mem.read_u64(fa).unwrap_or(0) } else { 0 };
    let _ = c.mem.write_u64(b + B_ACTS, acts);
    let a = if attr != 0 { c.mem.read_u64(attr).unwrap_or(0) } else { 0 };
    let _ = c.mem.write_u64(b + B_ATTR, a);
    // Status pipe (O_CLOEXEC): the child's exec closes it; a failure writes the errno into it.
    Act::Linux(293, [b + B_PIPE, 0x80000, 0, 0, 0, 0], Post::Cont(K_SPAWN_PIPE, [b, 0, 0]))
}

/// The child applies the attributes: stage 0 session/group, 1 mask, then defaults for signals
/// `sig..32`, then the file actions.
fn attr_step(mem: &mut UserMem, b: u64, stage: u64, sig: u64) -> Then {
    let a = rd(mem, b, B_ATTR);
    let (flags, pg, mask, def) = if a != 0 { (rd(mem, a, A_FLAGS), rd(mem, a, A_PGROUP), rd(mem, a, A_MASK), rd(mem, a, A_DEFAULT)) } else { (0, 0, 0, 0) };
    let next = |st: u64, sg: u64| Post::Cont(K_SPAWN_ATTR, [b, st, sg]);
    match stage {
        0 if flags & SETSID != 0 => Then::Linux(112, [0; 6], next(1, 1)),
        0 if flags & SETPGROUP != 0 => Then::Linux(109, [0, pg, 0, 0, 0, 0], next(1, 1)),
        0 => attr_step(mem, b, 1, 1),
        1 if flags & SETSIGMASK != 0 => {
            let _ = mem.write_u64(b + B_TMP, crate::signal::mask_to_linux(mask));
            Then::Linux(14, [2, b + B_TMP, 0, 8, 0, 0], next(2, 1))
        }
        1 => attr_step(mem, b, 2, 1),
        2 => {
            let Some(s) = (sig..32).find(|s| flags & SETSIGDEF != 0 && def >> (s - 1) & 1 == 1 && *s != 9 && *s != 17) else { return spawn_step(mem, b, 0, 0, 0) };
            // struct sigaction { handler = SIG_DFL, flags, restorer, mask } staged in the block.
            let _ = mem.write(SCRATCH, &[0u8; 32]);
            Then::Linux(13, [crate::posix::signal_to_linux(s), SCRATCH, 0, 8, 0, 0], next(2, s + 1))
        }
        _ => spawn_step(mem, b, 0, 0, 0),
    }
}

fn set_attr(c: &mut Call, a: u64, field: u64, v: u64) -> Act {
    let p = c.mem.read_u64(a).unwrap_or(0);
    if p == 0 {
        return c.ret(22);
    }
    let _ = c.mem.write_u64(p + field, v);
    c.ret(0)
}

fn get_attr(c: &mut Call, a: u64, field: u64) -> u64 {
    let p = c.mem.read_u64(a).unwrap_or(0);
    if p == 0 { 0 } else { c.mem.read_u64(p + field).unwrap_or(0) }
}

/// Append a file action to the list `*fa` points at.
fn add_action(c: &mut Call, fa: u64, rec: [u64; 5]) -> Act {
    let list = c.mem.read_u64(fa).unwrap_or(0);
    if list == 0 {
        return c.ret(22);
    }
    let n = rd(c.mem, list, 0);
    if n >= ACT_MAX {
        return c.ret(12);
    }
    let r = list + 8 + n * ACT_WORDS * 8;
    for (k, v) in rec.iter().enumerate() {
        let _ = c.mem.write_u64(r + k as u64 * 8, *v);
    }
    let _ = c.mem.write_u64(list, n + 1);
    c.ret(0)
}

fn copy_str(c: &mut Call, p: u64) -> u64 {
    let mut s = c.cstr(p);
    s.push(0);
    let d = c.st.heap.alloc(c.mem, s.len() as u64);
    let _ = c.mem.write(d, &s);
    d
}

pub static FUNCS: &[(&str, Func)] = &[
    f!("_fork", |c| { let _ = c; Act::Linux(57, [0; 6], Post::Libc) }),
    f!("_vfork", |c| { let _ = c; Act::Linux(57, [0; 6], Post::Libc) }),
    f!("_execve", |c| exec(c, None, false)),
    f!("_execv", |c| exec(c, Some(0), false)),
    f!("_execvp", |c| exec(c, Some(0), true)),
    f!("_posix_spawn", |c| spawn(c, false)),
    f!("_posix_spawnp", |c| spawn(c, true)),
    f!("_posix_spawn_file_actions_init", |c| {
        let fa = c.arg();
        let l = c.st.heap.alloc(c.mem, 8 + ACT_MAX * ACT_WORDS * 8);
        if l == 0 {
            return c.ret(12);
        }
        let _ = c.mem.write_u64(l, 0);
        let _ = c.mem.write_u64(fa, l);
        c.ret(0)
    }),
    f!("_posix_spawn_file_actions_destroy", |c| {
        let fa = c.arg();
        let l = c.mem.read_u64(fa).unwrap_or(0);
        c.st.heap.free(c.mem, l);
        let _ = c.mem.write_u64(fa, 0);
        c.ret(0)
    }),
    f!("_posix_spawn_file_actions_addopen", |c| {
        let (fa, fd, p, fl, m) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg());
        let s = copy_str(c, p);
        add_action(c, fa, [FA_OPEN, fd, fl, m, s])
    }),
    f!("_posix_spawn_file_actions_addclose", |c| { let (fa, fd) = (c.arg(), c.arg()); add_action(c, fa, [FA_CLOSE, fd, 0, 0, 0]) }),
    f!("_posix_spawn_file_actions_adddup2", |c| { let (fa, a, b) = (c.arg(), c.arg(), c.arg()); add_action(c, fa, [FA_DUP2, a, b, 0, 0]) }),
    f!("_posix_spawn_file_actions_addinherit_np", |c| { let (fa, fd) = (c.arg(), c.arg()); add_action(c, fa, [FA_DUP2, fd, fd, 0, 0]) }),
    f!("_posix_spawn_file_actions_addchdir_np", |c| { let (fa, p) = (c.arg(), c.arg()); let s = copy_str(c, p); add_action(c, fa, [FA_CHDIR, 0, 0, 0, s]) }),
    f!("_posix_spawn_file_actions_addfchdir_np", |c| { let (fa, fd) = (c.arg(), c.arg()); add_action(c, fa, [FA_FCHDIR, fd, 0, 0, 0]) }),
    // Attributes (flags, signal mask/defaults, process group) are accepted and not applied.
    f!("_posix_spawnattr_init", |c| {
        let a = c.arg();
        let p = c.st.heap.alloc(c.mem, A_LEN);
        if p == 0 {
            return c.ret(12);
        }
        let _ = c.mem.write(p, &[0u8; A_LEN as usize]);
        let _ = c.mem.write_u64(a, p);
        c.ret(0)
    }),
    f!("_posix_spawnattr_destroy", |c| { let a = c.arg(); let p = c.mem.read_u64(a).unwrap_or(0); c.st.heap.free(c.mem, p); let _ = c.mem.write_u64(a, 0); c.ret(0) }),
    f!("_posix_spawnattr_setflags", |c| { let (a, v) = (c.arg(), c.arg()); set_attr(c, a, A_FLAGS, v & 0xffff) }),
    f!("_posix_spawnattr_getflags", |c| { let (a, out) = (c.arg(), c.arg()); let v = get_attr(c, a, A_FLAGS); let _ = c.mem.write(out, &(v as u16).to_le_bytes()); c.ret(0) }),
    f!("_posix_spawnattr_setpgroup", |c| { let (a, v) = (c.arg(), c.arg()); set_attr(c, a, A_PGROUP, v & 0xffff_ffff) }),
    f!("_posix_spawnattr_setsigmask", |c| { let (a, p) = (c.arg(), c.arg()); let v = c.mem.read_u32(p).unwrap_or(0) as u64; set_attr(c, a, A_MASK, v) }),
    f!("_posix_spawnattr_setsigdefault", |c| { let (a, p) = (c.arg(), c.arg()); let v = c.mem.read_u32(p).unwrap_or(0) as u64; set_attr(c, a, A_DEFAULT, v) }),
    f!("_posix_spawnattr_setbinpref_np", |c| c.ret(0)),
];
