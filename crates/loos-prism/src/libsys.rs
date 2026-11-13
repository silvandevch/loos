//! Prism's libSystem: the C library macOS programs import from `/usr/lib/libSystem.B.dylib`,
//! implemented in Rust. Every function sees its arguments through [`Abi`] (x86-64 SysV registers,
//! or the ARM64 interpreter's registers with Apple's variadic-on-stack rule), works on the
//! process's memory, and either returns a value or forwards to a LoOS (Linux-layer) syscall.

#[allow(unused_imports)]
use alloc::{boxed::Box, collections::BTreeMap, format, string::{String, ToString}, vec, vec::Vec};
use crate::darwin::linux_to_darwin_errno;
use crate::loader::{FIXED_STUBS, SCRATCH, SCRATCH_LEN, SHIM_BASE, SHIM_DATA, STUB_SIZE};
use loos_lin::mem::*;

/// Bytes of the libSystem data page(s).
pub const DATA_LEN: u64 = 0x8000;
const D_STDIN: u64 = 0x00;
const D_STDOUT: u64 = 0x08;
const D_STDERR: u64 = 0x10;
/// errno (the address `___error()` returns).
pub const D_ERRNO: u64 = 0x18;
const D_ENVIRON: u64 = 0x20;
const D_ARGC: u64 = 0x28;
const D_ARGV: u64 = 0x30;
const D_PROGNAME: u64 = 0x38;
const D_TASK: u64 = 0x40;
const D_GUARD: u64 = 0x48;
const D_PAGESIZE: u64 = 0x50;
const D_MBMAX: u64 = 0x58;
pub(crate) const D_OPTIND: u64 = 0x60;
pub(crate) const D_OPTARG: u64 = 0x68;
pub(crate) const D_OPTERR: u64 = 0x70;
pub(crate) const D_OPTOPT: u64 = 0x74;
pub(crate) const D_OPTRESET: u64 = 0x78;
/// getopt's position inside a cluster of short options (`-abc`).
pub(crate) const D_OPTPLACE: u64 = 0x88;
/// `_DefaultRuneLocale` (the ctype tables the SDK's inline `isalpha` etc. read).
pub(crate) const D_RUNE: u64 = 0x4000;
/// `strerror` / `sys_errlist` strings, then the pointer table.
pub(crate) const D_ERRSTR: u64 = 0x5000;
/// Static results: `struct tm`, `asctime` text, `struct passwd`.
pub(crate) const D_STATIC: u64 = 0x7000;
/// `FILE` objects: the one for fd `n` lives at `SHIM_DATA + FILE0 + n * FILE_SIZE`.
const FILE0: u64 = 0x100;
const FILE_SIZE: u64 = 0x100;
const MAX_FILE_FD: u64 = 60;
/// Mach port name `mach_task_self()` returns.
pub const TASK_PORT: u64 = 0x103;

/// Data symbols and their offsets in the data page.
pub fn data_symbol(name: &str) -> Option<u64> {
    Some(match name {
        "___stdinp" => D_STDIN,
        "___stdoutp" => D_STDOUT,
        "___stderrp" => D_STDERR,
        "_environ" => D_ENVIRON,
        "_NXArgc" => D_ARGC,
        "_NXArgv" => D_ARGV,
        "___progname" => D_PROGNAME,
        "_mach_task_self_" => D_TASK,
        "___stack_chk_guard" => D_GUARD,
        "_vm_page_size" | "_vm_kernel_page_size" => D_PAGESIZE,
        "___mb_cur_max" => D_MBMAX,
        "_optind" => D_OPTIND,
        "_optarg" => D_OPTARG,
        "_opterr" => D_OPTERR,
        "_optopt" => D_OPTOPT,
        "_optreset" => D_OPTRESET,
        "__DefaultRuneLocale" => D_RUNE,
        "_sys_nerr" => D_ERRSTR - 8,
        _ => return None,
    })
}

/// Initial contents of the data page.
pub fn data_init() -> Vec<u8> {
    let mut d = vec![0u8; DATA_LEN as usize];
    let mut put = |o: u64, v: u64| d[o as usize..o as usize + 8].copy_from_slice(&v.to_le_bytes());
    put(D_STDIN, file_for(0));
    put(D_STDOUT, file_for(1));
    put(D_STDERR, file_for(2));
    put(D_TASK, TASK_PORT);
    put(D_GUARD, 0x2f8a_5c31_e7d4_9b06);
    put(D_PAGESIZE, PAGE);
    put(D_MBMAX, 1);
    put(D_OPTIND, 1);
    put(D_OPTERR, 1);
    put(D_ERRSTR - 8, crate::posix::errors().count() as u64);
    crate::posix::init_data(&mut d);
    d
}

fn file_for(fd: u64) -> u64 {
    SHIM_DATA + FILE0 + fd * FILE_SIZE
}

fn fd_of(file: u64) -> Option<u64> {
    let o = file.checked_sub(SHIM_DATA + FILE0)?;
    (o % FILE_SIZE == 0 && o / FILE_SIZE <= MAX_FILE_FD).then_some(o / FILE_SIZE)
}

/// Argument access for one call, independent of the CPU.
pub trait Args {
    /// Next integer/pointer argument.
    fn int(&mut self, mem: &mut UserMem) -> u64;
    /// Next floating-point (double) argument.
    fn float(&mut self, mem: &mut UserMem) -> f64;
}

/// The calling convention of the CPU running the program.
pub trait Abi: Args {
    /// The named arguments are consumed; what follows is `...` (ARM64 Apple ABI: all on the stack).
    fn va_start(&mut self);
    /// Set the return value.
    fn ret(&mut self, v: u64);
    /// Set integer argument register `i` (for calls Prism makes into the program).
    fn set_arg(&mut self, i: usize, v: u64);
    /// Continue at `target` instead of returning; `ret_to` is the address it returns to
    /// (`None`: enter like a process entry point, with the stack pointer unchanged).
    fn jump(&mut self, mem: &mut UserMem, target: u64, ret_to: Option<u64>) -> Result<(), i64>;
}

/// An x86-64 `va_list` (`{gp_offset, fp_offset, overflow_arg_area, reg_save_area}`).
struct X86VaList {
    gp: u32,
    fp: u32,
    over: u64,
    save: u64,
}

impl Args for X86VaList {
    fn int(&mut self, mem: &mut UserMem) -> u64 {
        if self.gp < 48 {
            let v = mem.read_u64(self.save + self.gp as u64).unwrap_or(0);
            self.gp += 8;
            v
        } else {
            let v = mem.read_u64(self.over).unwrap_or(0);
            self.over += 8;
            v
        }
    }
    fn float(&mut self, mem: &mut UserMem) -> f64 {
        if self.fp < 176 {
            let v = mem.read_u64(self.save + self.fp as u64).unwrap_or(0);
            self.fp += 16;
            f64::from_bits(v)
        } else {
            let v = mem.read_u64(self.over).unwrap_or(0);
            self.over += 8;
            f64::from_bits(v)
        }
    }
}

/// An ARM64 Apple `va_list` (a pointer to 8-byte stack slots).
struct ArmVaList(u64);

impl Args for ArmVaList {
    fn int(&mut self, mem: &mut UserMem) -> u64 {
        let v = mem.read_u64(self.0).unwrap_or(0);
        self.0 += 8;
        v
    }
    fn float(&mut self, mem: &mut UserMem) -> f64 {
        f64::from_bits(self.int(mem))
    }
}

/// How a forwarded LoOS syscall's result becomes the C function's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Post {
    /// C library style: negative → `-1` with `errno` set (Darwin numbering).
    Libc,
    /// Darwin system-call style: error → carry set, positive Darwin errno.
    Darwin,
    /// Return the value as is.
    Raw,
    /// Return this value on success (e.g. `puts`: a non-negative number), `-1`/errno on error.
    Value(u64),
    /// Darwin `stat64`: convert the Linux `stat` staged at `darwin::STAT_STAGE` into this buffer.
    DarwinStat(u64),
    /// Darwin `pipe`: the two descriptors (staged in the scratch buffer) are returned in rax/rdx (x0/x1).
    DarwinPipe,
    /// Finish in Rust: `posix::cont(id, ctx, result)` decides the C result (or a further syscall).
    Cont(u8, [u64; 3]),
}

/// What a continuation decides.
pub enum Then {
    /// Return this value.
    Ret(u64),
    /// Fail with this Linux errno, returning `ret` (-1, or NULL for pointer results).
    Fail(i64, u64),
    /// Run another LoOS syscall.
    Linux(u64, [u64; 6], Post),
}

/// What a libSystem function asks Prism to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// The return value is set (or control was redirected with [`Abi::jump`]).
    Done,
    /// Run LoOS syscall `nr` (Linux numbering) and convert its result.
    Linux(u64, [u64; 6], Post),
    /// The function is not implemented: stop the process with a diagnostic.
    Missing,
    /// Abnormal termination (`abort`, stack-protector failure) with this message.
    Abort(String),
}

/// Per-process libSystem state.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// Stub index → symbol name.
    pub stubs: Vec<String>,
    funcs: Vec<Option<Func>>,
    pub(crate) heap: Heap,
    /// Initializers still to run, then the entry point.
    pub inits: Vec<u64>,
    /// `Some(main)` for `LC_MAIN` programs, `None` for `LC_UNIXTHREAD`.
    pub main: Option<u64>,
    /// Thread entry (`LC_UNIXTHREAD`).
    pub thread_entry: u64,
    /// argc, argv, envp, apple.
    pub start_args: [u64; 4],
    pub(crate) atexit: Vec<(u64, Option<u64>)>,
    exit_code: Option<u64>,
    /// Thread-local-variable templates and the blocks made from them (template start → block).
    pub tlv: Vec<(u64, u64, u64)>,
    tlv_blocks: BTreeMap<u64, u64>,
    pub(crate) rng: u64,
    /// Darwin signal → (handler, Darwin flags, Darwin mask) as the program installed them.
    pub(crate) sigs: BTreeMap<u64, (u64, u64, u64)>,
    /// The program runs on the ARM64 interpreter (affects `va_list` layout).
    pub arm: bool,
    /// Unix time (ns) at monotonic time zero.
    pub epoch_ns: u64,
    /// Diagnostics for the system log.
    pub notes: String,
}

pub(crate) type Func = fn(&mut Call) -> Act;

impl State {
    /// State for a freshly loaded program.
    pub fn new(stubs: Vec<String>) -> Self {
        let funcs = stubs.iter().map(|s| FUNCS.iter().chain(crate::posix::FUNCS).chain(crate::posix2::FUNCS).chain(crate::getopt::FUNCS).chain(crate::signal::FUNCS).find(|f| f.0 == s.as_str()).map(|f| f.1)).collect();
        Self { stubs, funcs, rng: 0x853c_49e6_748f_ea9b, ..Self::default() }
    }

    /// Run stub `i`.
    pub fn call(&mut self, i: usize, abi: &mut dyn Abi, mem: &mut UserMem, now_ns: u64) -> Act {
        let Some(f) = self.funcs.get(i).copied() else { return Act::Missing };
        match f {
            Some(f) => f(&mut Call { abi, mem, st: self, now_ns }),
            None => Act::Missing,
        }
    }

    /// Name of stub `i`.
    pub fn name(&self, i: usize) -> &str {
        self.stubs.get(i).map(|s| s.as_str()).unwrap_or("?")
    }
}

/// One libSystem call in progress.
pub struct Call<'a> {
    /// Arguments / return.
    pub abi: &'a mut dyn Abi,
    /// Process memory.
    pub mem: &'a mut UserMem,
    /// libSystem state.
    pub st: &'a mut State,
    /// Monotonic clock (ns).
    pub now_ns: u64,
}

impl Call<'_> {
    pub(crate) fn arg(&mut self) -> u64 {
        self.abi.int(self.mem)
    }
    pub(crate) fn ret(&mut self, v: u64) -> Act {
        self.abi.ret(v);
        Act::Done
    }
    pub(crate) fn cstr(&mut self, p: u64) -> Vec<u8> {
        read_cbytes(self.mem, p, 1 << 20)
    }
    pub(crate) fn set_errno(&mut self, e: i64) {
        let _ = self.mem.write_u32(SHIM_DATA + D_ERRNO, linux_to_darwin_errno(e) as u32);
    }
    /// Write `bytes` to fd through the Linux layer (staged in the scratch buffer).
    pub(crate) fn out(&mut self, fd: u64, bytes: &[u8], post: Post) -> Act {
        let n = bytes.len().min(SCRATCH_LEN as usize);
        if self.mem.write(SCRATCH, &bytes[..n]).is_err() {
            return self.ret(u64::MAX);
        }
        Act::Linux(1, [fd, SCRATCH, n as u64, 0, 0, 0], post)
    }
    pub(crate) fn stub_addr(&self, name: &str) -> u64 {
        let i = self.st.stubs.iter().position(|s| s == name).unwrap_or(0);
        SHIM_BASE + i as u64 * STUB_SIZE
    }
}

/// Read a NUL-terminated string (at most `max` bytes).
pub fn read_cbytes(mem: &mut UserMem, mut p: u64, max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    while out.len() < max {
        // Never read across a page boundary in one go: the next page may be unmapped.
        let n = (64 - (p % 64)) as usize;
        if mem.read(p, &mut buf[..n]).is_err() {
            break;
        }
        match buf[..n].iter().position(|&b| b == 0) {
            Some(z) => {
                out.extend_from_slice(&buf[..z]);
                return out;
            }
            None => out.extend_from_slice(&buf[..n]),
        }
        p += n as u64;
    }
    out
}

// ---------------------------------------------------------------- heap

#[derive(Clone, Debug, Default)]
pub(crate) struct Heap {
    cur: u64,
    end: u64,
    free: BTreeMap<u64, Vec<u64>>,
}

const BIG: u64 = 128 * 1024;

impl Heap {
    pub(crate) fn alloc(&mut self, mem: &mut UserMem, n: u64) -> u64 {
        let need = n.max(1).checked_add(16).unwrap_or(u64::MAX);
        if need > 1 << 40 {
            return 0;
        }
        if need > BIG {
            let len = ceil(need);
            return match mem.map_anywhere(len, PROT_READ | PROT_WRITE, &[], "malloc (large)") {
                Ok(a) => {
                    let _ = mem.write_u64(a, len);
                    let _ = mem.write_u64(a + 8, n);
                    a + 16
                }
                Err(_) => 0,
            };
        }
        let class = need.next_power_of_two().max(32);
        let block = match self.free.get_mut(&class).and_then(|v| v.pop()) {
            Some(b) => b,
            None => {
                if self.cur + class > self.end {
                    let chunk = 4u64 << 20;
                    match mem.map_anywhere(chunk, PROT_READ | PROT_WRITE, &[], "malloc") {
                        Ok(a) => {
                            self.cur = a;
                            self.end = a + chunk;
                        }
                        Err(_) => return 0,
                    }
                }
                let b = self.cur;
                self.cur += class;
                b
            }
        };
        let _ = mem.write_u64(block, class);
        let _ = mem.write_u64(block + 8, n);
        block + 16
    }

    pub(crate) fn size(&self, mem: &mut UserMem, p: u64) -> u64 {
        if p < 16 {
            return 0;
        }
        mem.read_u64(p - 8).unwrap_or(0)
    }

    pub(crate) fn free(&mut self, mem: &mut UserMem, p: u64) {
        if p < 16 {
            return;
        }
        let Ok(class) = mem.read_u64(p - 16) else { return };
        if class > BIG {
            let _ = mem.unmap(p - 16, class);
        } else if class.is_power_of_two() && class >= 32 {
            self.free.entry(class).or_default().push(p - 16);
        }
    }
}

// ---------------------------------------------------------------- printf

fn pad(out: &mut Vec<u8>, body: &[u8], width: usize, left: bool, zero: bool) {
    let fill = width.saturating_sub(body.len());
    if left {
        out.extend_from_slice(body);
        out.extend(core::iter::repeat_n(b' ', fill));
    } else if zero {
        // Zero padding goes after the sign / 0x prefix.
        let sign = body.iter().take_while(|&&c| c == b'-' || c == b'+' || c == b' ').count();
        let pre = if body[sign..].starts_with(b"0x") || body[sign..].starts_with(b"0X") { sign + 2 } else { sign };
        out.extend_from_slice(&body[..pre]);
        out.extend(core::iter::repeat_n(b'0', fill));
        out.extend_from_slice(&body[pre..]);
    } else {
        out.extend(core::iter::repeat_n(b' ', fill));
        out.extend_from_slice(body);
    }
}

fn pow10(n: i32) -> f64 {
    let mut v = 1.0;
    for _ in 0..n.unsigned_abs() {
        v *= 10.0;
    }
    if n < 0 {
        1.0 / v
    } else {
        v
    }
}

/// `x >= 0` with `prec` decimals.
fn fixed(x: f64, prec: usize) -> String {
    let prec = prec.min(17);
    if x >= 1e18 {
        // Integer part beyond u64 precision: print digits of the scaled value.
        let mut e = 0;
        let mut m = x;
        while m >= 1e17 {
            m /= 10.0;
            e += 1;
        }
        let mut s = format!("{}", m as u64);
        s.extend(core::iter::repeat_n('0', e));
        if prec > 0 {
            s.push('.');
            s.extend(core::iter::repeat_n('0', prec));
        }
        return s;
    }
    let scale = pow10(prec as i32) as u128;
    let v = (x * scale as f64 + 0.5) as u128;
    let (i, f) = (v / scale, v % scale);
    if prec == 0 {
        format!("{i}")
    } else {
        format!("{i}.{f:0prec$}")
    }
}

fn exp_form(x: f64, prec: usize, upper: bool) -> String {
    let mut e = 0i32;
    let mut m = x;
    if m != 0.0 {
        while m >= 10.0 {
            m /= 10.0;
            e += 1;
        }
        while m < 1.0 {
            m *= 10.0;
            e -= 1;
        }
    }
    let mut s = fixed(m, prec);
    if s.starts_with("10") {
        e += 1;
        s = fixed(m / 10.0, prec);
    }
    format!("{s}{}{}{:02}", if upper { 'E' } else { 'e' }, if e < 0 { '-' } else { '+' }, e.unsigned_abs())
}

fn strip_zeros(s: String) -> String {
    if !s.contains('.') {
        return s;
    }
    let (num, exp) = match s.find(['e', 'E']) {
        Some(i) => (s[..i].to_string(), s[i..].to_string()),
        None => (s.clone(), String::new()),
    };
    let num = num.trim_end_matches('0').trim_end_matches('.');
    format!("{num}{exp}")
}

fn float_body(x: f64, conv: u8, prec: Option<usize>, alt: bool) -> String {
    let upper = conv.is_ascii_uppercase();
    if x.is_nan() {
        return if upper { "NAN".into() } else { "nan".into() };
    }
    if x.is_infinite() {
        return if upper { "INF".into() } else { "inf".into() };
    }
    let p = prec.unwrap_or(6);
    match conv | 0x20 {
        b'e' => exp_form(x, p, upper),
        b'g' => {
            let p = p.max(1);
            let e = if x == 0.0 { 0 } else { exp_form(x, p - 1, false).rsplit(['e']).next().and_then(|s| s.parse::<i32>().ok()).unwrap_or(0) };
            let s = if e < -4 || e >= p as i32 { exp_form(x, p - 1, upper) } else { fixed(x, (p as i32 - 1 - e).max(0) as usize) };
            if alt {
                s
            } else {
                strip_zeros(s)
            }
        }
        b'a' => format!("{x:e}"),
        _ => fixed(x, p),
    }
}

/// Format like C `printf`.
pub fn format(mem: &mut UserMem, fmt: &[u8], args: &mut dyn Args) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != b'%' {
            out.push(c);
            continue;
        }
        let (mut left, mut zero, mut plus, mut space, mut alt) = (false, false, false, false, false);
        while i < fmt.len() {
            match fmt[i] {
                b'-' => left = true,
                b'0' => zero = true,
                b'+' => plus = true,
                b' ' => space = true,
                b'#' => alt = true,
                b'\'' => {}
                _ => break,
            }
            i += 1;
        }
        let mut width = 0usize;
        if fmt.get(i) == Some(&b'*') {
            let w = args.int(mem) as i32;
            if w < 0 {
                left = true;
            }
            width = w.unsigned_abs() as usize;
            i += 1;
        }
        while let Some(d @ b'0'..=b'9') = fmt.get(i) {
            width = width * 10 + (d - b'0') as usize;
            i += 1;
        }
        let mut prec = None;
        if fmt.get(i) == Some(&b'.') {
            i += 1;
            let mut p = 0usize;
            if fmt.get(i) == Some(&b'*') {
                let v = args.int(mem) as i32;
                i += 1;
                prec = (v >= 0).then_some(v as usize);
            } else {
                while let Some(d @ b'0'..=b'9') = fmt.get(i) {
                    p = p * 10 + (d - b'0') as usize;
                    i += 1;
                }
                prec = Some(p);
            }
        }
        let width = width.min(4096);
        // Length modifiers: only the integer width matters (registers and slots are 64-bit).
        let mut bits = 32;
        while let Some(&m) = fmt.get(i) {
            match m {
                b'h' => bits = if bits == 16 { 8 } else { 16 },
                b'l' | b'q' | b'j' | b'z' | b't' | b'L' => bits = 64,
                _ => break,
            }
            i += 1;
        }
        let Some(&conv) = fmt.get(i) else { break };
        i += 1;
        let sign_of = |neg: bool| if neg { "-" } else if plus { "+" } else if space { " " } else { "" };
        match conv {
            b'%' => out.push(b'%'),
            b'd' | b'i' => {
                let raw = args.int(mem);
                let v = match bits {
                    8 => raw as i8 as i64,
                    16 => raw as i16 as i64,
                    32 => raw as i32 as i64,
                    _ => raw as i64,
                };
                let mut digits = format!("{}", v.unsigned_abs());
                if let Some(p) = prec {
                    if p == 0 && v == 0 {
                        digits.clear();
                    }
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                }
                let body = format!("{}{digits}", sign_of(v < 0));
                pad(&mut out, body.as_bytes(), width, left, zero && prec.is_none());
            }
            b'u' | b'x' | b'X' | b'o' | b'p' => {
                let raw = args.int(mem);
                let v = if conv == b'p' { raw } else {
                    match bits {
                        8 => raw as u8 as u64,
                        16 => raw as u16 as u64,
                        32 => raw as u32 as u64,
                        _ => raw,
                    }
                };
                let mut digits = match conv {
                    b'u' => format!("{v}"),
                    b'o' => format!("{v:o}"),
                    b'X' => format!("{v:X}"),
                    _ => format!("{v:x}"),
                };
                if let Some(p) = prec {
                    if p == 0 && v == 0 {
                        digits.clear();
                    }
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                }
                let prefix = match conv {
                    b'p' => "0x",
                    b'x' if alt && v != 0 => "0x",
                    b'X' if alt && v != 0 => "0X",
                    b'o' if alt && !digits.starts_with('0') => "0",
                    _ => "",
                };
                let body = format!("{prefix}{digits}");
                pad(&mut out, body.as_bytes(), width, left, zero && prec.is_none());
            }
            b'c' => {
                let ch = args.int(mem) as u8;
                pad(&mut out, &[ch], width, left, false);
            }
            b's' => {
                let p = args.int(mem);
                let mut s = if p == 0 { b"(null)".to_vec() } else { read_cbytes(mem, p, prec.unwrap_or(1 << 20)) };
                if let Some(pr) = prec {
                    s.truncate(pr);
                }
                pad(&mut out, &s, width, left, false);
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => {
                let x = args.float(mem);
                let neg = x.is_sign_negative() && !x.is_nan();
                let body = format!("{}{}", sign_of(neg), float_body(if neg { -x } else { x }, conv, prec, alt));
                pad(&mut out, body.as_bytes(), width, left, zero && x.is_finite());
            }
            b'n' => {
                let p = args.int(mem);
                let _ = mem.write_u32(p, out.len() as u32);
            }
            _ => {
                out.push(b'%');
                out.push(conv);
            }
        }
        if out.len() > 1 << 22 {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------- functions

pub(crate) fn copy_out(c: &mut Call, dst: u64, size: u64, bytes: &[u8]) {
    if size == 0 {
        return;
    }
    let n = bytes.len().min(size as usize - 1);
    let _ = c.mem.write(dst, &bytes[..n]);
    let _ = c.mem.write(dst + n as u64, &[0]);
}

fn va_list(c: &mut Call, ap: u64, arm: bool) -> Box<dyn Args> {
    if arm {
        Box::new(ArmVaList(ap))
    } else {
        let gp = c.mem.read_u32(ap).unwrap_or(48);
        let fp = c.mem.read_u32(ap + 4).unwrap_or(176);
        Box::new(X86VaList { gp, fp, over: c.mem.read_u64(ap + 8).unwrap_or(0), save: c.mem.read_u64(ap + 16).unwrap_or(0) })
    }
}

/// `printf` family core: `dest` is where the text goes, `fixed` how many named args precede the format.
enum Dest {
    Fd(u64),
    Buf(u64, u64),
}

fn printf_common(c: &mut Call, dest: Dest, valist: Option<bool>) -> Act {
    let f = c.arg();
    let fmt = c.cstr(f);
    let text = match valist {
        Some(arm) => {
            let ap = c.arg();
            let mut a = va_list(c, ap, arm);
            format(c.mem, &fmt, a.as_mut())
        }
        None => {
            c.abi.va_start();
            let mem = &mut *c.mem;
            format(mem, &fmt, c.abi)
        }
    };
    match dest {
        Dest::Fd(fd) => {
            let n = text.len() as u64;
            c.out(fd, &text, Post::Value(n))
        }
        Dest::Buf(p, size) => {
            copy_out(c, p, size, &text);
            c.ret(text.len() as u64)
        }
    }
}

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

fn start(c: &mut Call) -> Act {
    if let Some(init) = (!c.st.inits.is_empty()).then(|| c.st.inits.remove(0)) {
        // Initializers get (argc, argv, envp, apple) like main.
        for (i, v) in c.st.start_args.iter().enumerate() {
            c.abi.set_arg(i, *v);
        }
        let back = c.stub_addr(FIXED_STUBS[0]);
        return match c.abi.jump(c.mem, init, Some(back)) {
            Ok(()) => Act::Done,
            Err(_) => Act::Abort("cannot call an initializer".into()),
        };
    }
    let [argc, argv, envp, _] = c.st.start_args;
    let _ = c.mem.write_u64(SHIM_DATA + D_ENVIRON, envp);
    let _ = c.mem.write_u64(SHIM_DATA + D_ARGC, argc);
    let _ = c.mem.write_u64(SHIM_DATA + D_ARGV, argv);
    let prog = c.mem.read_u64(argv).unwrap_or(0);
    let _ = c.mem.write_u64(SHIM_DATA + D_PROGNAME, prog);
    let r = match c.st.main {
        Some(main) => {
            for (i, v) in c.st.start_args.iter().enumerate() {
                c.abi.set_arg(i, *v);
            }
            let back = c.stub_addr(FIXED_STUBS[1]);
            c.abi.jump(c.mem, main, Some(back))
        }
        None => {
            let e = c.st.thread_entry;
            c.abi.jump(c.mem, e, None)
        }
    };
    match r {
        Ok(()) => Act::Done,
        Err(_) => Act::Abort("cannot enter the program".into()),
    }
}

fn exit(c: &mut Call) -> Act {
    let code = match c.st.exit_code {
        Some(v) => v,
        None => {
            let v = c.arg();
            c.st.exit_code = Some(v);
            v
        }
    };
    if let Some((f, arg)) = c.st.atexit.pop() {
        if let Some(a) = arg {
            c.abi.set_arg(0, a);
        }
        let back = c.stub_addr("_exit");
        let back = if back == SHIM_BASE { c.stub_addr(FIXED_STUBS[1]) } else { back };
        if c.abi.jump(c.mem, f, Some(back)).is_ok() {
            return Act::Done;
        }
    }
    Act::Linux(231, [code & 0xff, 0, 0, 0, 0, 0], Post::Raw)
}

fn main_ret(c: &mut Call) -> Act {
    exit(c)
}

fn strtol(c: &mut Call, signed: bool) -> Act {
    let p = c.arg();
    let endp = c.arg();
    let mut base = c.arg() as u32;
    let s = c.cstr(p);
    let mut i = s.iter().take_while(|b| b.is_ascii_whitespace()).count();
    let mut neg = false;
    if let Some(&sg @ (b'-' | b'+')) = s.get(i) {
        neg = sg == b'-';
        i += 1;
    }
    if (base == 0 || base == 16) && s.get(i) == Some(&b'0') && matches!(s.get(i + 1), Some(b'x' | b'X')) && s.get(i + 2).is_some_and(|b| b.is_ascii_hexdigit()) {
        i += 2;
        base = 16;
    } else if base == 0 && s.get(i) == Some(&b'0') {
        base = 8;
    } else if base == 0 {
        base = 10;
    }
    let start = i;
    let mut v: u64 = 0;
    let mut over = false;
    while let Some(d) = s.get(i).and_then(|&b| (b as char).to_digit(base.clamp(2, 36))) {
        match v.checked_mul(base as u64).and_then(|x| x.checked_add(d as u64)) {
            Some(x) => v = x,
            None => over = true,
        }
        i += 1;
    }
    if endp != 0 {
        let _ = c.mem.write_u64(endp, if i == start { p } else { p + i as u64 });
    }
    let r = if signed {
        let lim = if neg { 1u64 << 63 } else { i64::MAX as u64 };
        if over || v > lim {
            c.set_errno(34);
            if neg { i64::MIN as u64 } else { i64::MAX as u64 }
        } else if neg {
            (v as i64).wrapping_neg() as u64
        } else {
            v
        }
    } else if over {
        c.set_errno(34);
        u64::MAX
    } else if neg {
        v.wrapping_neg()
    } else {
        v
    };
    c.ret(r)
}

fn mem_op(c: &mut Call, kind: u8) -> Act {
    let (a, b, n) = (c.arg(), c.arg(), c.arg());
    if n > 1 << 32 {
        return c.ret(a);
    }
    match kind {
        b'c' | b'm' => {
            let Ok(v) = c.mem.read_vec(b, n as usize) else { return Act::Abort("memcpy from an unmapped address".into()) };
            if c.mem.write(a, &v).is_err() {
                return Act::Abort("memcpy to an unmapped address".into());
            }
            c.ret(a)
        }
        b's' => {
            if c.mem.write(a, &vec![b as u8; n as usize]).is_err() {
                return Act::Abort("memset on an unmapped address".into());
            }
            c.ret(a)
        }
        _ => {
            let x = c.mem.read_vec(a, n as usize).unwrap_or_default();
            let y = c.mem.read_vec(b, n as usize).unwrap_or_default();
            let r = x.iter().zip(&y).find(|(p, q)| p != q).map(|(p, q)| *p as i32 - *q as i32).unwrap_or(0);
            c.ret(r as i64 as u64)
        }
    }
}

fn str_cmp(c: &mut Call, limit: bool, nocase: bool) -> Act {
    let (a, b) = (c.arg(), c.arg());
    let n = if limit { c.arg() as usize } else { usize::MAX };
    let mut x = read_cbytes(c.mem, a, n.min(1 << 20));
    let mut y = read_cbytes(c.mem, b, n.min(1 << 20));
    if nocase {
        x.make_ascii_lowercase();
        y.make_ascii_lowercase();
    }
    x.push(0);
    y.push(0);
    let r = x.iter().zip(&y).take(n).find(|(p, q)| p != q).map(|(p, q)| *p as i32 - *q as i32).unwrap_or(0);
    c.ret(r as i64 as u64)
}

pub(crate) fn timespec_now(c: &Call) -> (u64, u64) {
    let wall = c.st.epoch_ns + c.now_ns;
    (wall / 1_000_000_000, wall % 1_000_000_000)
}

fn file_fd(c: &mut Call) -> Option<u64> {
    let f = c.arg();
    fd_of(f)
}

static FUNCS: &[(&str, Func)] = &[
    ("__prism_start", start as Func),
    ("__prism_main_ret", main_ret as Func),
    ("_exit", exit as Func),
    f!("__exit", |c| { let v = c.arg(); Act::Linux(231, [v & 0xff, 0, 0, 0, 0, 0], Post::Raw) }),
    f!("_abort", |_c| Act::Abort("abort() called".into())),
    f!("___stack_chk_fail", |_c| Act::Abort("stack smashing detected".into())),
    f!("___chk_fail_overflow", |_c| Act::Abort("buffer overflow detected".into())),
    f!("_atexit", |c| { let f = c.arg(); c.st.atexit.push((f, None)); c.ret(0) }),
    f!("___cxa_atexit", |c| { let (f, a) = (c.arg(), c.arg()); c.st.atexit.push((f, Some(a))); c.ret(0) }),
    f!("___error", |c| c.ret(SHIM_DATA + D_ERRNO)),
    // stdio (unbuffered: FILE objects name file descriptors)
    f!("_printf", |c| printf_common(c, Dest::Fd(1), None)),
    f!("_vprintf", |c| { let arm = is_arm_abi(c); printf_common(c, Dest::Fd(1), Some(arm)) }),
    f!("_fprintf", |c| match file_fd(c) { Some(fd) => printf_common(c, Dest::Fd(fd), None), None => c.ret(u64::MAX) }),
    f!("_vfprintf", |c| { let arm = is_arm_abi(c); match file_fd(c) { Some(fd) => printf_common(c, Dest::Fd(fd), Some(arm)), None => c.ret(u64::MAX) } }),
    f!("_dprintf", |c| { let fd = c.arg(); printf_common(c, Dest::Fd(fd), None) }),
    f!("_sprintf", |c| { let p = c.arg(); printf_common(c, Dest::Buf(p, 1 << 20), None) }),
    f!("_snprintf", |c| { let (p, n) = (c.arg(), c.arg()); printf_common(c, Dest::Buf(p, n), None) }),
    f!("_vsnprintf", |c| { let arm = is_arm_abi(c); let (p, n) = (c.arg(), c.arg()); printf_common(c, Dest::Buf(p, n), Some(arm)) }),
    f!("_vsprintf", |c| { let arm = is_arm_abi(c); let p = c.arg(); printf_common(c, Dest::Buf(p, 1 << 20), Some(arm)) }),
    f!("___sprintf_chk", |c| { let p = c.arg(); let _flag = c.arg(); let n = c.arg(); printf_common(c, Dest::Buf(p, n), None) }),
    f!("___snprintf_chk", |c| { let (p, n) = (c.arg(), c.arg()); let _ = (c.arg(), c.arg()); printf_common(c, Dest::Buf(p, n), None) }),
    f!("___vsnprintf_chk", |c| { let arm = is_arm_abi(c); let (p, n) = (c.arg(), c.arg()); let _ = (c.arg(), c.arg()); printf_common(c, Dest::Buf(p, n), Some(arm)) }),
    f!("_puts", |c| { let p = c.arg(); let mut s = c.cstr(p); s.push(b'\n'); c.out(1, &s, Post::Value(1)) }),
    f!("_putchar", |c| { let ch = c.arg() as u8; c.out(1, &[ch], Post::Value(ch as u64)) }),
    f!("_fputs", |c| { let p = c.arg(); let s = c.cstr(p); match file_fd(c) { Some(fd) => c.out(fd, &s, Post::Value(1)), None => c.ret(u64::MAX) } }),
    f!("_fputc", |c| { let ch = c.arg() as u8; match file_fd(c) { Some(fd) => c.out(fd, &[ch], Post::Value(ch as u64)), None => c.ret(u64::MAX) } }),
    f!("_putc", |c| { let ch = c.arg() as u8; match file_fd(c) { Some(fd) => c.out(fd, &[ch], Post::Value(ch as u64)), None => c.ret(u64::MAX) } }),
    f!("_fwrite", |c| {
        let (p, size, n) = (c.arg(), c.arg(), c.arg());
        let total = size.saturating_mul(n).min(SCRATCH_LEN);
        let Some(fd) = file_fd(c) else { return c.ret(0) };
        let data = c.mem.read_vec(p, total as usize).unwrap_or_default();
        c.out(fd, &data, Post::Value(if size == 0 { 0 } else { total / size }))
    }),
    f!("_fflush", |c| c.ret(0)),
    f!("_setvbuf", |c| c.ret(0)),
    f!("_fileno", |c| match file_fd(c) { Some(fd) => c.ret(fd), None => c.ret(u64::MAX) }),
    f!("_fdopen", |c| { let fd = c.arg(); c.ret(if fd <= MAX_FILE_FD { file_for(fd) } else { 0 }) }),
    // POSIX I/O
    f!("_write", |c| { let (fd, p, n) = (c.arg(), c.arg(), c.arg()); Act::Linux(1, [fd, p, n, 0, 0, 0], Post::Libc) }),
    f!("_read", |c| { let (fd, p, n) = (c.arg(), c.arg(), c.arg()); Act::Linux(0, [fd, p, n, 0, 0, 0], Post::Libc) }),
    f!("_close", |c| { let fd = c.arg(); Act::Linux(3, [fd, 0, 0, 0, 0, 0], Post::Libc) }),
    f!("_lseek", |c| { let (fd, o, w) = (c.arg(), c.arg(), c.arg()); Act::Linux(8, [fd, o, w, 0, 0, 0], Post::Libc) }),
    f!("_open", |c| {
        let (p, fl) = (c.arg(), c.arg());
        c.abi.va_start();
        let mode = c.arg();
        Act::Linux(2, [p, crate::darwin::open_flags(fl), mode, 0, 0, 0], Post::Libc)
    }),
    f!("_unlink", |c| { let p = c.arg(); Act::Linux(87, [p, 0, 0, 0, 0, 0], Post::Libc) }),
    f!("_mkdir", |c| { let (p, m) = (c.arg(), c.arg()); Act::Linux(83, [p, m, 0, 0, 0, 0], Post::Libc) }),
    f!("_rmdir", |c| { let p = c.arg(); Act::Linux(84, [p, 0, 0, 0, 0, 0], Post::Libc) }),
    f!("_chdir", |c| { let p = c.arg(); Act::Linux(80, [p, 0, 0, 0, 0, 0], Post::Libc) }),
    f!("_rename", |c| { let (a, b) = (c.arg(), c.arg()); Act::Linux(82, [a, b, 0, 0, 0, 0], Post::Libc) }),
    f!("_access", |c| { let (p, m) = (c.arg(), c.arg()); Act::Linux(21, [p, m, 0, 0, 0, 0], Post::Libc) }),
    f!("_dup", |c| { let fd = c.arg(); Act::Linux(32, [fd, 0, 0, 0, 0, 0], Post::Libc) }),
    f!("_dup2", |c| { let (a, b) = (c.arg(), c.arg()); Act::Linux(33, [a, b, 0, 0, 0, 0], Post::Libc) }),
    f!("_isatty", |c| { let fd = c.arg(); c.ret((fd <= 2) as u64) }),
    f!("_getcwd", |c| {
        let (mut p, mut n) = (c.arg(), c.arg());
        if p == 0 {
            // getcwd(NULL, size): the library allocates the buffer (1024 bytes when size is 0).
            n = if n == 0 { 1024 } else { n };
            p = c.st.heap.alloc(c.mem, n);
        }
        Act::Linux(79, [p, n, 0, 0, 0, 0], Post::Value(p))
    }),
    f!("_getpid", |c| { let _ = c; Act::Linux(39, [0; 6], Post::Raw) }),
    f!("_getppid", |c| { let _ = c; Act::Linux(110, [0; 6], Post::Raw) }),
    f!("_getuid", |c| c.ret(501)),
    f!("_geteuid", |c| c.ret(501)),
    f!("_getgid", |c| c.ret(20)),
    f!("_getegid", |c| c.ret(20)),
    f!("_usleep", |c| {
        let us = c.arg();
        let _ = c.mem.write_u64(SCRATCH, us / 1_000_000);
        let _ = c.mem.write_u64(SCRATCH + 8, us % 1_000_000 * 1000);
        Act::Linux(35, [SCRATCH, 0, 0, 0, 0, 0], Post::Libc)
    }),
    f!("_sleep", |c| {
        let s = c.arg();
        let _ = c.mem.write_u64(SCRATCH, s);
        let _ = c.mem.write_u64(SCRATCH + 8, 0);
        Act::Linux(35, [SCRATCH, 0, 0, 0, 0, 0], Post::Value(0))
    }),
    f!("_nanosleep", |c| { let (a, b) = (c.arg(), c.arg()); Act::Linux(35, [a, b, 0, 0, 0, 0], Post::Libc) }),
    // memory
    f!("_malloc", |c| { let n = c.arg(); let p = c.st.heap.alloc(c.mem, n); c.ret(p) }),
    f!("_calloc", |c| {
        let (a, b) = (c.arg(), c.arg());
        let Some(n) = a.checked_mul(b) else { return c.ret(0) };
        let p = c.st.heap.alloc(c.mem, n);
        if p != 0 && c.mem.write(p, &vec![0; n as usize]).is_err() {
            return c.ret(0);
        }
        c.ret(p)
    }),
    f!("_realloc", |c| {
        let (old, n) = (c.arg(), c.arg());
        let p = c.st.heap.alloc(c.mem, n);
        if p != 0 && old != 0 {
            let keep = c.st.heap.size(c.mem, old).min(n);
            if let Ok(v) = c.mem.read_vec(old, keep as usize) {
                let _ = c.mem.write(p, &v);
            }
            c.st.heap.free(c.mem, old);
        }
        c.ret(p)
    }),
    f!("_free", |c| { let p = c.arg(); c.st.heap.free(c.mem, p); c.ret(0) }),
    f!("_malloc_size", |c| { let p = c.arg(); let n = c.st.heap.size(c.mem, p); c.ret(n) }),
    f!("_posix_memalign", |c| {
        let (out, align, n) = (c.arg(), c.arg(), c.arg());
        if !align.is_power_of_two() || align > 4096 {
            return c.ret(22);
        }
        // Blocks are size-class aligned to 16; over-allocate for larger alignments.
        let raw = c.st.heap.alloc(c.mem, n + align);
        let p = if align <= 16 { raw } else { (raw + align - 1) & !(align - 1) };
        if p != raw {
            // Keep a valid header in front of the aligned pointer so free() still works.
            let class = c.mem.read_u64(raw - 16).unwrap_or(0);
            let _ = c.mem.write_u64(p - 16, class);
            let _ = c.mem.write_u64(p - 8, n);
        }
        let _ = c.mem.write_u64(out, p);
        c.ret(if raw == 0 { 12 } else { 0 })
    }),
    f!("_memcpy", |c| mem_op(c, b'c')),
    f!("_memmove", |c| mem_op(c, b'm')),
    f!("_memset", |c| mem_op(c, b's')),
    f!("_memcmp", |c| mem_op(c, b'=')),
    f!("_bcmp", |c| mem_op(c, b'=')),
    f!("___memcpy_chk", |c| mem_op(c, b'c')),
    f!("___memmove_chk", |c| mem_op(c, b'm')),
    f!("___memset_chk", |c| mem_op(c, b's')),
    f!("_bzero", |c| { let (p, n) = (c.arg(), c.arg()); let _ = c.mem.write(p, &vec![0; n.min(1 << 32) as usize]); c.ret(0) }),
    f!("_memchr", |c| {
        let (p, ch, n) = (c.arg(), c.arg() as u8, c.arg());
        let v = c.mem.read_vec(p, n.min(1 << 30) as usize).unwrap_or_default();
        let r = v.iter().position(|&b| b == ch).map(|i| p + i as u64).unwrap_or(0);
        c.ret(r)
    }),
    // strings
    f!("_strlen", |c| { let p = c.arg(); let n = c.cstr(p).len(); c.ret(n as u64) }),
    f!("_strnlen", |c| { let (p, m) = (c.arg(), c.arg()); let n = read_cbytes(c.mem, p, m as usize).len(); c.ret(n as u64) }),
    f!("_strcmp", |c| str_cmp(c, false, false)),
    f!("_strncmp", |c| str_cmp(c, true, false)),
    f!("_strcasecmp", |c| str_cmp(c, false, true)),
    f!("_strncasecmp", |c| str_cmp(c, true, true)),
    f!("_strcpy", |c| { let (d, s) = (c.arg(), c.arg()); let mut v = c.cstr(s); v.push(0); let _ = c.mem.write(d, &v); c.ret(d) }),
    f!("___strcpy_chk", |c| { let (d, s) = (c.arg(), c.arg()); let mut v = c.cstr(s); v.push(0); let _ = c.mem.write(d, &v); c.ret(d) }),
    f!("_stpcpy", |c| { let (d, s) = (c.arg(), c.arg()); let mut v = c.cstr(s); let n = v.len(); v.push(0); let _ = c.mem.write(d, &v); c.ret(d + n as u64) }),
    f!("_strncpy", |c| {
        let (d, s, n) = (c.arg(), c.arg(), c.arg());
        let mut v = read_cbytes(c.mem, s, n as usize);
        v.resize(n.min(1 << 30) as usize, 0);
        let _ = c.mem.write(d, &v);
        c.ret(d)
    }),
    f!("_strlcpy", |c| {
        let (d, s, n) = (c.arg(), c.arg(), c.arg());
        let v = c.cstr(s);
        copy_out(c, d, n, &v);
        c.ret(v.len() as u64)
    }),
    f!("_strcat", |c| { let (d, s) = (c.arg(), c.arg()); let n = c.cstr(d).len() as u64; let mut v = c.cstr(s); v.push(0); let _ = c.mem.write(d + n, &v); c.ret(d) }),
    f!("___strcat_chk", |c| { let (d, s) = (c.arg(), c.arg()); let n = c.cstr(d).len() as u64; let mut v = c.cstr(s); v.push(0); let _ = c.mem.write(d + n, &v); c.ret(d) }),
    f!("_strlcat", |c| {
        let (d, s, n) = (c.arg(), c.arg(), c.arg());
        let mut v = c.cstr(d);
        let src = c.cstr(s);
        let total = v.len() + src.len();
        v.extend(src);
        copy_out(c, d, n, &v);
        c.ret(total as u64)
    }),
    f!("_strchr", |c| { let (p, ch) = (c.arg(), c.arg() as u8); let s = c.cstr(p); let r = if ch == 0 { Some(s.len()) } else { s.iter().position(|&b| b == ch) }; c.ret(r.map(|i| p + i as u64).unwrap_or(0)) }),
    f!("_strrchr", |c| { let (p, ch) = (c.arg(), c.arg() as u8); let s = c.cstr(p); let r = if ch == 0 { Some(s.len()) } else { s.iter().rposition(|&b| b == ch) }; c.ret(r.map(|i| p + i as u64).unwrap_or(0)) }),
    f!("_strstr", |c| {
        let (h, n) = (c.arg(), c.arg());
        let (hs, ns) = (c.cstr(h), c.cstr(n));
        let r = if ns.is_empty() { Some(0) } else { hs.windows(ns.len()).position(|w| w == ns.as_slice()) };
        c.ret(r.map(|i| h + i as u64).unwrap_or(0))
    }),
    f!("_strdup", |c| {
        let s = c.arg();
        let mut v = c.cstr(s);
        v.push(0);
        let p = c.st.heap.alloc(c.mem, v.len() as u64);
        if p != 0 {
            let _ = c.mem.write(p, &v);
        }
        c.ret(p)
    }),
    f!("_atoi", |c| { let p = c.arg(); let s = c.cstr(p); c.ret(parse_int(&s) as i32 as i64 as u64) }),
    f!("_atol", |c| { let p = c.arg(); let s = c.cstr(p); c.ret(parse_int(&s) as u64) }),
    f!("_atoll", |c| { let p = c.arg(); let s = c.cstr(p); c.ret(parse_int(&s) as u64) }),
    f!("_strtol", |c| strtol(c, true)),
    f!("_strtoll", |c| strtol(c, true)),
    f!("_strtoul", |c| strtol(c, false)),
    f!("_strtoull", |c| strtol(c, false)),
    f!("_toupper", |c| { let v = c.arg(); c.ret(if (b'a' as u64..=b'z' as u64).contains(&v) { v - 32 } else { v }) }),
    f!("_tolower", |c| { let v = c.arg(); c.ret(if (b'A' as u64..=b'Z' as u64).contains(&v) { v + 32 } else { v }) }),
    f!("_abs", |c| { let v = c.arg() as i32; c.ret(v.unsigned_abs() as u64) }),
    f!("_labs", |c| { let v = c.arg() as i64; c.ret(v.unsigned_abs()) }),
    // environment
    f!("_getenv", |c| {
        let name = c.arg();
        let key = c.cstr(name);
        let mut env = c.mem.read_u64(SHIM_DATA + D_ENVIRON).unwrap_or(0);
        while env != 0 {
            let Ok(p) = c.mem.read_u64(env) else { break };
            if p == 0 {
                break;
            }
            let e = read_cbytes(c.mem, p, 32768);
            if e.len() > key.len() && e.starts_with(&key) && e[key.len()] == b'=' {
                return c.ret(p + key.len() as u64 + 1);
            }
            env += 8;
        }
        c.ret(0)
    }),
    f!("_getprogname", |c| { let p = c.mem.read_u64(SHIM_DATA + D_PROGNAME).unwrap_or(0); let s = c.cstr(p); let base = s.iter().rposition(|&b| b == b'/').map(|i| i + 1).unwrap_or(0); c.ret(p + base as u64) }),
    f!("__NSGetArgc", |c| c.ret(SHIM_DATA + D_ARGC)),
    f!("__NSGetArgv", |c| c.ret(SHIM_DATA + D_ARGV)),
    f!("__NSGetEnviron", |c| c.ret(SHIM_DATA + D_ENVIRON)),
    f!("_sysconf", |c| { let n = c.arg(); c.ret(match n { 29 => PAGE, 57 | 58 => 1, 3 => 100, 5 => 256, _ => u64::MAX }) }),
    f!("_getpagesize", |c| c.ret(PAGE)),
    // time
    f!("_time", |c| { let p = c.arg(); let (s, _) = timespec_now(c); if p != 0 { let _ = c.mem.write_u64(p, s); } c.ret(s) }),
    f!("_gettimeofday", |c| { let p = c.arg(); let (s, ns) = timespec_now(c); if p != 0 { let _ = c.mem.write_u64(p, s); let _ = c.mem.write_u32(p + 8, (ns / 1000) as u32); } c.ret(0) }),
    f!("_clock_gettime", |c| {
        let (id, p) = (c.arg(), c.arg());
        // Darwin clock ids: 0 REALTIME, 6 MONOTONIC, 4 MONOTONIC_RAW, 8 UPTIME_RAW, 12/16 CPU time.
        let (s, ns) = if id == 0 { timespec_now(c) } else { (c.now_ns / 1_000_000_000, c.now_ns % 1_000_000_000) };
        let _ = c.mem.write_u64(p, s);
        let _ = c.mem.write_u64(p + 8, ns);
        c.ret(0)
    }),
    f!("_clock_gettime_nsec_np", |c| { let id = c.arg(); let v = if id == 0 { c.st.epoch_ns + c.now_ns } else { c.now_ns }; c.ret(v) }),
    f!("_mach_absolute_time", |c| c.ret(c.now_ns)),
    f!("_mach_continuous_time", |c| c.ret(c.now_ns)),
    f!("_mach_timebase_info", |c| { let p = c.arg(); let _ = c.mem.write_u32(p, 1); let _ = c.mem.write_u32(p + 4, 1); c.ret(0) }),
    f!("_clock", |c| c.ret(c.now_ns / 1000)),
    // randomness
    f!("_arc4random", |c| { let v = next_rand(c.st); c.ret(v & 0xffff_ffff) }),
    f!("_arc4random_uniform", |c| { let n = c.arg() & 0xffff_ffff; let v = next_rand(c.st); c.ret(if n == 0 { 0 } else { v % n }) }),
    f!("_arc4random_buf", |c| {
        let (p, n) = (c.arg(), c.arg().min(1 << 20));
        let bytes: Vec<u8> = (0..n).map(|_| next_rand(c.st) as u8).collect();
        let _ = c.mem.write(p, &bytes);
        c.ret(0)
    }),
    f!("_rand", |c| { let v = next_rand(c.st); c.ret(v & 0x7fff_ffff) }),
    f!("_srand", |c| { let s = c.arg(); c.st.rng = s | 1; c.ret(0) }),
    f!("_random", |c| { let v = next_rand(c.st); c.ret(v & 0x7fff_ffff) }),
    f!("_srandom", |c| { let s = c.arg(); c.st.rng = s | 1; c.ret(0) }),
    // Mach (the task's own port names; IPC goes through the Darwin trap layer)
    f!("_mach_task_self", |c| c.ret(TASK_PORT)),
    f!("_mach_thread_self", |c| c.ret(TASK_PORT + 4)),
    f!("_mach_host_self", |c| c.ret(TASK_PORT + 8)),
    f!("_mach_port_deallocate", |c| c.ret(0)),
    // Thread-local variables (one thread per process for now: one block per template).
    f!("__tlv_bootstrap", |c| {
        let desc = c.arg();
        let key_off = c.mem.read_u64(desc + 16).unwrap_or(0);
        let (start, init, size) = c.st.tlv.first().copied().unwrap_or((0, 0, 0));
        let block = match c.st.tlv_blocks.get(&start) {
            Some(&b) => b,
            None => {
                let total: u64 = c.st.tlv.iter().map(|t| t.2).sum::<u64>().max(16);
                let b = c.st.heap.alloc(c.mem, total);
                let _ = c.mem.write(b, &vec![0; total as usize]);
                let mut at = b;
                for &(s, init_len, len) in &c.st.tlv.clone() {
                    if let Ok(v) = c.mem.read_vec(s, init_len as usize) {
                        let _ = c.mem.write(at, &v);
                    }
                    at += len;
                }
                c.st.tlv_blocks.insert(start, b);
                b
            }
        };
        let _ = (init, size);
        c.ret(block + key_off)
    }),
    // Single-threaded processes: locks are uncontended and succeed immediately.
    f!("_pthread_mutex_init", |c| c.ret(0)),
    f!("_pthread_mutex_lock", |c| c.ret(0)),
    f!("_pthread_mutex_trylock", |c| c.ret(0)),
    f!("_pthread_mutex_unlock", |c| c.ret(0)),
    f!("_pthread_mutex_destroy", |c| c.ret(0)),
    f!("_pthread_once", |c| {
        let (once, f) = (c.arg(), c.arg());
        // pthread_once_t: {long sig; char opaque[8]}; mark done in the opaque word.
        if c.mem.read_u64(once + 8).unwrap_or(0) == 1 {
            return c.ret(0);
        }
        let _ = c.mem.write_u64(once + 8, 1);
        let back = c.stub_addr("_pthread_once");
        // Run the init routine; it returns into pthread_once, which now sees it done.
        c.abi.set_arg(0, once);
        c.abi.set_arg(1, f);
        match c.abi.jump(c.mem, f, Some(back)) {
            Ok(()) => Act::Done,
            Err(_) => c.ret(0),
        }
    }),
    f!("_pthread_self", |c| c.ret(SHIM_DATA + 0x80)),
    f!("_pthread_main_np", |c| c.ret(1)),
];

fn is_arm_abi(c: &Call) -> bool {
    c.st.arm
}

fn parse_int(s: &[u8]) -> i64 {
    let mut i = s.iter().take_while(|b| b.is_ascii_whitespace()).count();
    let neg = s.get(i) == Some(&b'-');
    if matches!(s.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let mut v: i64 = 0;
    while let Some(d @ b'0'..=b'9') = s.get(i) {
        v = v.wrapping_mul(10).wrapping_add((d - b'0') as i64);
        i += 1;
    }
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

pub(crate) fn next_rand(st: &mut State) -> u64 {
    let mut x = st.rng;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    st.rng = x;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    struct V(Vec<u64>, Vec<f64>);
    impl Args for V {
        fn int(&mut self, _: &mut UserMem) -> u64 {
            self.0.remove(0)
        }
        fn float(&mut self, _: &mut UserMem) -> f64 {
            self.1.remove(0)
        }
    }

    fn fmt(f: &str, ints: &[u64], fl: &[f64]) -> String {
        let mut m = UserMem::new();
        String::from_utf8(format(&mut m, f.as_bytes(), &mut V(ints.to_vec(), fl.to_vec()))).unwrap()
    }

    #[test]
    fn printf_formats() {
        assert_eq!(fmt("%d|%5d|%-5d|%05d|%+d", &[(-3i64) as u64, 42, 42, 42, 7], &[]), "-3|   42|42   |00042|+7");
        assert_eq!(fmt("%x %X %#x %o %u %lu", &[255, 255, 255, 8, (-1i32) as u32 as u64, u64::MAX], &[]), "ff FF 0xff 10 4294967295 18446744073709551615");
        assert_eq!(fmt("%.2f %f %e %g %g", &[], &[3.14159, 1.5, 12345.678, 0.0001, 100000.0]), "3.14 1.500000 1.234568e+04 0.0001 100000");
        assert_eq!(fmt("%g %.3g %-8.3f|", &[], &[1e20, 2.0 / 3.0, -1.0]), "1e+20 0.667 -1.000  |");
        assert_eq!(fmt("%c%%%5.1s|%*d", &[b'A' as u64, 0, 3, 9], &[]), "A%    (|  9");
    }
}
