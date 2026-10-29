//! x64 structured exception handling: table-based virtual unwind and
//! `RtlDispatchException` ordering (vectored handlers, frame handlers,
//! continue handlers, unhandled filter).
//!
//! Implements the documented PE32+ `.pdata`/`UNWIND_INFO` format (Microsoft
//! "x64 exception handling" spec). Memory is read through [`Memory`] so the
//! same code runs in the kernel (user-mode dispatch on the user stack) and on
//! the host in tests. Handlers are invoked through a caller closure because
//! running them means switching to user mode.
//!
//! Epilog detection (unwinding while RIP sits inside an epilog) is not done:
//! the prolog-offset rule is applied and the function treated as in its body.

use alloc::vec::Vec;

/// Byte-addressed view of the faulting address space.
pub trait Memory {
    /// Fills `buf` from `addr`; `false` if any byte is unmapped.
    fn read(&self, addr: u64, buf: &mut [u8]) -> bool;
}

fn rd_u64(m: &dyn Memory, a: u64) -> Option<u64> {
    let mut b = [0; 8];
    m.read(a, &mut b).then(|| u64::from_le_bytes(b))
}

fn rd_u32(m: &dyn Memory, a: u64) -> Option<u32> {
    let mut b = [0; 4];
    m.read(a, &mut b).then(|| u32::from_le_bytes(b))
}

fn rd_u16(m: &dyn Memory, a: u64) -> Option<u16> {
    let mut b = [0; 2];
    m.read(a, &mut b).then(|| u16::from_le_bytes(b))
}

/// Integer register file in x64 encoding order (RAX=0 … R15=15, RSP=4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// Instruction pointer.
    pub rip: u64,
    /// General-purpose registers.
    pub gpr: [u64; 16],
}

/// Register index of RSP.
pub const RSP: usize = 4;

impl Context {
    /// Stack pointer.
    pub fn rsp(&self) -> u64 {
        self.gpr[RSP]
    }
}

/// `RUNTIME_FUNCTION` (RVAs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeFunction {
    /// Function start RVA.
    pub begin: u32,
    /// Function end RVA (exclusive).
    pub end: u32,
    /// `UNWIND_INFO` RVA.
    pub unwind: u32,
}

/// A loaded image with its exception directory, sorted by `begin`.
#[derive(Clone, Debug)]
pub struct Image {
    /// Load address.
    pub base: u64,
    /// `SizeOfImage`.
    pub size: u64,
    /// `.pdata` entries.
    pub functions: Vec<RuntimeFunction>,
}

impl Image {
    /// Reads the exception directory (`rva`, `size` from data directory 3).
    pub fn load(base: u64, size: u64, mem: &dyn Memory, rva: u32, len: u32) -> Option<Self> {
        let mut functions = Vec::with_capacity(len as usize / 12);
        for i in 0..len / 12 {
            let a = base + rva as u64 + i as u64 * 12;
            functions.push(RuntimeFunction { begin: rd_u32(mem, a)?, end: rd_u32(mem, a + 4)?, unwind: rd_u32(mem, a + 8)? });
        }
        functions.sort_by_key(|f| f.begin);
        Some(Self { base, size, functions })
    }

    /// `RtlLookupFunctionEntry`.
    pub fn lookup(&self, rip: u64) -> Option<RuntimeFunction> {
        let rva = u32::try_from(rip.checked_sub(self.base)?).ok()?;
        let i = self.functions.partition_point(|f| f.begin <= rva).checked_sub(1)?;
        let f = self.functions[i];
        (rva < f.end).then_some(f)
    }
}

const UNW_FLAG_EHANDLER: u8 = 1;
const UNW_FLAG_CHAININFO: u8 = 4;

/// Language handler found while unwinding a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHandler {
    /// Absolute handler address.
    pub routine: u64,
    /// Absolute address of the handler-specific data.
    pub data: u64,
    /// Establisher frame passed to the handler.
    pub establisher: u64,
}

/// Unwind failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwindError {
    /// Unwind data or stack unreadable.
    BadMemory,
    /// Unknown version or opcode.
    BadUnwindInfo,
}

/// `RtlVirtualUnwind`: rewinds `ctx` to the caller of `func` and returns the
/// exception handler if the function has one and RIP is past its prolog.
pub fn virtual_unwind(img: &Image, func: RuntimeFunction, ctx: &mut Context, mem: &dyn Memory) -> Result<Option<FrameHandler>, UnwindError> {
    let e = UnwindError::BadMemory;
    let offset = ctx.rip.wrapping_sub(img.base + func.begin as u64);
    let mut info = img.base + func.unwind as u64;
    let mut handler = None;
    let mut primary = true;
    let mut establisher = ctx.rsp();
    loop {
        let mut h = [0u8; 4];
        if !mem.read(info, &mut h) {
            return Err(e);
        }
        let (ver, flags, prolog, count) = (h[0] & 7, h[0] >> 3, h[1] as u64, h[2] as usize);
        let (freg, foff) = ((h[3] & 15) as usize, (h[3] >> 4) as u64 * 16);
        if !(1..=2).contains(&ver) {
            return Err(UnwindError::BadUnwindInfo);
        }
        let codes = info + 4;
        let in_prolog = primary && offset < prolog;
        if freg != 0 && primary && !in_prolog {
            establisher = ctx.gpr[freg].wrapping_sub(foff);
        }
        let mut i = 0;
        while i < count {
            let slot = rd_u16(mem, codes + i as u64 * 2).ok_or(e)?;
            let (at, op, inf) = ((slot & 0xff) as u64, (slot >> 8) as u8 & 15, (slot >> 12) as usize);
            let arg = |n: usize| rd_u16(mem, codes + (i + n) as u64 * 2).map(u64::from).ok_or(e);
            let arg32 = |n: usize| Ok::<u64, UnwindError>(arg(n)? | arg(n + 1)? << 16);
            let slots = match op {
                0 | 2 | 3 | 10 => 1,
                1 => if inf == 0 { 2 } else { 3 },
                4 | 8 => 2,
                5 | 9 => 3,
                6 => 2, // UWOP_EPILOG (v2) — ignored
                _ => return Err(UnwindError::BadUnwindInfo),
            };
            if in_prolog && at > offset {
                i += slots;
                continue;
            }
            let rsp = ctx.rsp();
            match op {
                0 => {
                    ctx.gpr[inf] = rd_u64(mem, rsp).ok_or(e)?;
                    ctx.gpr[RSP] = rsp + 8;
                }
                1 => ctx.gpr[RSP] = rsp + if inf == 0 { arg(1)? * 8 } else { arg32(1)? },
                2 => ctx.gpr[RSP] = rsp + inf as u64 * 8 + 8,
                3 => ctx.gpr[RSP] = ctx.gpr[freg].wrapping_sub(foff),
                // SAVE_NONVOL offsets are relative to RSP at the point the code applies.
                4 => ctx.gpr[inf] = rd_u64(mem, rsp + arg(1)? * 8).ok_or(e)?,
                5 => ctx.gpr[inf] = rd_u64(mem, rsp + arg32(1)?).ok_or(e)?,
                10 => {
                    let frame = rsp + if inf == 1 { 8 } else { 0 };
                    ctx.rip = rd_u64(mem, frame).ok_or(e)?;
                    ctx.gpr[RSP] = rd_u64(mem, frame + 24).ok_or(e)?;
                    return Ok(None);
                }
                _ => {} // XMM saves and epilog markers do not change integer state
            }
            i += slots;
        }
        let tail = codes + count.div_ceil(2) as u64 * 4;
        if primary && flags & UNW_FLAG_EHANDLER != 0 && !in_prolog {
            handler = Some(FrameHandler { routine: img.base + rd_u32(mem, tail).ok_or(e)? as u64, data: tail + 4, establisher });
        }
        if flags & UNW_FLAG_CHAININFO == 0 {
            break;
        }
        info = img.base + rd_u32(mem, tail + 8).ok_or(e)? as u64;
        primary = false;
    }
    let rsp = ctx.rsp();
    ctx.rip = rd_u64(mem, rsp).ok_or(e)?;
    ctx.gpr[RSP] = rsp + 8;
    Ok(handler)
}

/// Handler verdict (`EXCEPTION_DISPOSITION` / VEH return value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// Resume at the (possibly modified) context.
    ContinueExecution,
    /// Try the next handler.
    ContinueSearch,
}

/// What the dispatcher asks the caller to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandlerCall {
    /// Vectored exception handler.
    Vectored(u64),
    /// Frame-based language handler.
    Frame(FrameHandler),
    /// Vectored continue handler (runs after a handler resumed execution).
    Continue(u64),
    /// `SetUnhandledExceptionFilter` routine.
    Unhandled(u64),
}

/// Final result of [`dispatch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Resume execution at the context the handler left.
    Continue,
    /// No handler took it: terminate the process (or second-chance debugger).
    Unhandled,
    /// The stack walk failed (corrupt stack or unwind data).
    BadStack,
}

/// Vectored handler registrations (`AddVectoredExceptionHandler` family).
#[derive(Default)]
pub struct Vectored {
    exception: Vec<(u64, u64)>,
    continue_: Vec<(u64, u64)>,
    next: u64,
    /// `SetUnhandledExceptionFilter` routine.
    pub unhandled_filter: Option<u64>,
}

impl Vectored {
    /// Empty list.
    pub fn new() -> Self {
        Self::default()
    }

    fn add(list: &mut Vec<(u64, u64)>, next: &mut u64, first: bool, routine: u64) -> u64 {
        *next += 1;
        if first {
            list.insert(0, (*next, routine));
        } else {
            list.push((*next, routine));
        }
        *next
    }

    /// `AddVectoredExceptionHandler`: returns a registration cookie.
    pub fn add_exception(&mut self, first: bool, routine: u64) -> u64 {
        Self::add(&mut self.exception, &mut self.next, first, routine)
    }

    /// `AddVectoredContinueHandler`.
    pub fn add_continue(&mut self, first: bool, routine: u64) -> u64 {
        Self::add(&mut self.continue_, &mut self.next, first, routine)
    }

    /// `RemoveVectored{Exception,Continue}Handler`: whether the cookie existed.
    pub fn remove(&mut self, cookie: u64) -> bool {
        let n = self.exception.len() + self.continue_.len();
        self.exception.retain(|e| e.0 != cookie);
        self.continue_.retain(|e| e.0 != cookie);
        n != self.exception.len() + self.continue_.len()
    }
}

/// Maximum frames walked before declaring the stack corrupt.
pub const MAX_FRAMES: usize = 4096;

/// `RtlDispatchException` + `UnhandledExceptionFilter` ordering.
///
/// `call` runs a handler with the context of the frame being examined (for
/// vectored/unhandled handlers: the faulting context) and may modify it. On
/// [`Outcome::Continue`], `ctx` holds the resume context. `stack` is the
/// thread's `[limit, base)`.
pub fn dispatch(
    ctx: &mut Context,
    images: &[Image],
    mem: &dyn Memory,
    veh: &Vectored,
    stack: (u64, u64),
    call: &mut dyn FnMut(HandlerCall, &mut Context) -> Disposition,
) -> Outcome {
    let resumed = |ctx: &mut Context, call: &mut dyn FnMut(HandlerCall, &mut Context) -> Disposition| {
        for &(_, r) in &veh.continue_ {
            if call(HandlerCall::Continue(r), ctx) == Disposition::ContinueExecution {
                break;
            }
        }
        Outcome::Continue
    };
    for &(_, r) in &veh.exception {
        if call(HandlerCall::Vectored(r), ctx) == Disposition::ContinueExecution {
            return resumed(ctx, call);
        }
    }
    let mut frame = *ctx;
    for _ in 0..MAX_FRAMES {
        if frame.rip == 0 {
            break;
        }
        if !(stack.0..stack.1).contains(&frame.rsp()) || frame.rsp() & 7 != 0 {
            return Outcome::BadStack;
        }
        let hit = images.iter().find(|i| frame.rip.wrapping_sub(i.base) < i.size).and_then(|i| Some((i, i.lookup(frame.rip)?)));
        let handler = match hit {
            Some((img, f)) => match virtual_unwind(img, f, &mut frame, mem) {
                Ok(h) => h,
                Err(_) => return Outcome::BadStack,
            },
            None => {
                // Leaf function: return address on top of the stack.
                let Some(ret) = rd_u64(mem, frame.rsp()) else { return Outcome::BadStack };
                frame.rip = ret;
                frame.gpr[RSP] += 8;
                None
            }
        };
        if let Some(h) = handler {
            if h.establisher < stack.0 || h.establisher >= stack.1 {
                return Outcome::BadStack;
            }
            if call(HandlerCall::Frame(h), ctx) == Disposition::ContinueExecution {
                return resumed(ctx, call);
            }
        }
    }
    if let Some(f) = veh.unhandled_filter {
        if call(HandlerCall::Unhandled(f), ctx) == Disposition::ContinueExecution {
            return Outcome::Continue;
        }
    }
    Outcome::Unhandled
}
