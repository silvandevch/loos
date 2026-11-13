//! Signals: `sigaction`, `signal`, `sigprocmask` and the sigset helpers.
//!
//! Delivery uses the Linux layer's signal frames. The handler registered with LoOS is the
//! `__prism_sigtramp` stub: it renumbers the signal (Linux → Darwin) and continues in the program's
//! handler, which returns to the `__prism_sigreturn` stub (Linux `rt_sigreturn`).
//! The arm64 interpreter does not take signal frames yet: there, only `SIG_DFL`/`SIG_IGN` work.

use crate::libsys::{Act, Call, Func, Post};
use crate::loader::SCRATCH;
use crate::posix::signal_to_linux;

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

/// Linux signal number → Darwin.
pub fn signal_to_darwin(s: u64) -> u64 {
    (1..32).find(|&d| signal_to_linux(d) == s).unwrap_or(s)
}

/// Darwin `sigset_t` (32 bits, bit n-1 = signal n) → Linux mask.
fn mask_to_linux(m: u64) -> u64 {
    (1..32).filter(|d| m >> (d - 1) & 1 == 1).fold(0, |acc, d| acc | 1 << (signal_to_linux(d) - 1))
}

pub(crate) fn mask_to_darwin(m: u64) -> u64 {
    (1..32).filter(|d| m >> (signal_to_linux(*d) - 1) & 1 == 1).fold(0, |acc, d| acc | 1 << (d - 1))
}

const FLAGS: [(u64, u64); 7] = [(1, 0x0800_0000), (2, 0x1000_0000), (4, 0x8000_0000), (8, 1), (0x10, 0x4000_0000), (0x20, 2), (0x40, 4)];

fn flags_to_linux(f: u64) -> u64 {
    FLAGS.iter().filter(|(d, _)| f & d != 0).fold(0, |acc, (_, l)| acc | l)
}

/// Install (handler, Darwin flags, Darwin mask) for Darwin signal `sig`; returns the LoOS call.
/// `ret`: the C result on success (0 for sigaction, the old handler for signal).
fn install(c: &mut Call, sig: u64, handler: u64, flags: u64, mask: u64, ret: Post) -> Act {
    if !(1..32).contains(&sig) || sig == 9 || sig == 17 {
        c.set_errno(22);
        return c.ret(u64::MAX);
    }
    let custom = handler > 1;
    if custom && c.st.arm {
        c.set_errno(95); // EOPNOTSUPP: no signal frames on the interpreter yet
        return c.ret(u64::MAX);
    }
    c.st.sigs.insert(sig, (handler, flags, mask));
    let tramp = c.stub_addr("__prism_sigtramp");
    let restorer = c.stub_addr("__prism_sigreturn");
    let lin = [if custom { tramp } else { handler }, flags_to_linux(flags) | 0x0400_0000, restorer, mask_to_linux(mask)];
    for (i, v) in lin.iter().enumerate() {
        let _ = c.mem.write_u64(SCRATCH + i as u64 * 8, *v);
    }
    Act::Linux(13, [signal_to_linux(sig), SCRATCH, 0, 8, 0, 0], ret)
}

/// Entered from a Linux signal frame: rdi = Linux signal, rsi = siginfo, rdx = ucontext.
fn sigtramp(c: &mut Call) -> Act {
    let lsig = c.arg();
    let (info, uc) = (c.arg(), c.arg());
    let sig = signal_to_darwin(lsig);
    let Some(&(handler, flags, _)) = c.st.sigs.get(&sig) else { return Act::Abort(alloc::format!("signal {sig} without a handler")) };
    if flags & 4 != 0 {
        c.st.sigs.insert(sig, (0, 0, 0)); // SA_RESETHAND (LoOS reset its side too)
    }
    c.abi.set_arg(0, sig);
    c.abi.set_arg(1, info);
    c.abi.set_arg(2, uc);
    // Tail-call: the handler returns where the signal frame says (the sigreturn stub).
    match c.abi.jump(c.mem, handler, None) {
        Ok(()) => Act::Done,
        Err(_) => Act::Abort("cannot enter a signal handler".into()),
    }
}

fn set_bit(c: &mut Call, on: Option<bool>) -> Act {
    let (p, s) = (c.arg(), c.arg());
    if !(1..32).contains(&s) {
        c.set_errno(22);
        return c.ret(u64::MAX);
    }
    let v = c.mem.read_u32(p).unwrap_or(0);
    let bit = 1u32 << (s - 1);
    match on {
        Some(true) => { let _ = c.mem.write_u32(p, v | bit); c.ret(0) }
        Some(false) => { let _ = c.mem.write_u32(p, v & !bit); c.ret(0) }
        None => c.ret((v & bit != 0) as u64),
    }
}

pub static FUNCS: &[(&str, Func)] = &[
    ("__prism_sigtramp", sigtramp as Func),
    f!("__prism_sigreturn", |_c| Act::Linux(15, [0; 6], Post::Raw)),
    f!("_sigaction", |c| {
        let (sig, act, old) = (c.arg(), c.arg(), c.arg());
        if old != 0 {
            let (h, fl, m) = c.st.sigs.get(&sig).copied().unwrap_or((0, 0, 0));
            let _ = c.mem.write_u64(old, h);
            let _ = c.mem.write_u32(old + 8, m as u32);
            let _ = c.mem.write_u32(old + 12, fl as u32);
        }
        if act == 0 {
            return c.ret(0);
        }
        let h = c.mem.read_u64(act).unwrap_or(0);
        let m = c.mem.read_u32(act + 8).unwrap_or(0) as u64;
        let fl = c.mem.read_u32(act + 12).unwrap_or(0) as u64;
        install(c, sig, h, fl, m, Post::Libc)
    }),
    f!("_signal", |c| {
        let (sig, h) = (c.arg(), c.arg());
        let old = c.st.sigs.get(&sig).map(|s| s.0).unwrap_or(0);
        install(c, sig, h, 2, 0, Post::Value(old)) // BSD semantics: SA_RESTART
    }),
    f!("_sigprocmask", |c| {
        let (how, set, old) = (c.arg(), c.arg(), c.arg());
        let lhow = match how { 1 => 0, 2 => 1, 3 => 2, _ => { c.set_errno(22); return c.ret(u64::MAX) } };
        let s = if set != 0 { let m = c.mem.read_u32(set).unwrap_or(0) as u64; let _ = c.mem.write_u64(SCRATCH, mask_to_linux(m)); SCRATCH } else { 0 };
        Act::Linux(14, [lhow, s, if old != 0 { SCRATCH + 8 } else { 0 }, 8, 0, 0], Post::Cont(8, [old, 0, 0]))
    }),
    f!("_pthread_sigmask", |c| {
        let (how, set, old) = (c.arg(), c.arg(), c.arg());
        let lhow = match how { 1 => 0, 2 => 1, 3 => 2, _ => return c.ret(22) };
        let s = if set != 0 { let m = c.mem.read_u32(set).unwrap_or(0) as u64; let _ = c.mem.write_u64(SCRATCH, mask_to_linux(m)); SCRATCH } else { 0 };
        Act::Linux(14, [lhow, s, if old != 0 { SCRATCH + 8 } else { 0 }, 8, 0, 0], Post::Cont(8, [old, 0, 0]))
    }),
    f!("_sigemptyset", |c| { let p = c.arg(); let _ = c.mem.write_u32(p, 0); c.ret(0) }),
    f!("_sigfillset", |c| { let p = c.arg(); let _ = c.mem.write_u32(p, u32::MAX); c.ret(0) }),
    f!("_sigaddset", |c| set_bit(c, Some(true))),
    f!("_sigdelset", |c| set_bit(c, Some(false))),
    f!("_sigismember", |c| set_bit(c, None)),
];
