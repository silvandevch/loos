//! x64 structured exception handling, table-driven: `.pdata` lookup, `UNWIND_INFO` interpretation (the work of
//! `RtlLookupFunctionEntry`/`RtlVirtualUnwind`) and CONTEXT access. Dispatching to handlers needs to run guest code, so
//! that part lives with the kernel's thread driver (`kffi::win`).

use crate::Process;
use alloc::vec::Vec;

/// UNW_FLAG_EHANDLER.
pub const UNW_EHANDLER: u32 = 1;
/// UNW_FLAG_UHANDLER.
pub const UNW_UHANDLER: u32 = 2;
const UNW_CHAININFO: u32 = 4;
/// EXCEPTION_UNWINDING.
pub const EXC_UNWINDING: u32 = 2;
/// EXCEPTION_TARGET_UNWIND.
pub const EXC_TARGET_UNWIND: u32 = 0x20;
/// Size of a CONTEXT record.
pub const CONTEXT_SIZE: usize = 1232;
const CTX_GPR: u64 = 0x78;
const CTX_RIP: u64 = 0xf8;

/// General registers (in CONTEXT/encoding order: rax rcx rdx rbx rsp rbp rsi rdi r8..r15) and rip.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Ctx {
    /// Registers.
    pub r: [u64; 16],
    /// Instruction pointer.
    pub rip: u64,
}

impl Ctx {
    /// Stack pointer.
    pub fn rsp(&self) -> u64 {
        self.r[4]
    }
}

/// A mapped image with unwind tables.
pub struct Image {
    /// Load address.
    pub base: u64,
    /// Mapped size.
    pub size: u64,
    /// Guest address of the `.pdata` table.
    pub pdata_va: u64,
    /// `(begin, end, unwind info)` RVAs, sorted.
    pub funcs: Vec<(u32, u32, u32)>,
}

/// What [`Process::virtual_unwind`] found.
pub struct UnwindOut {
    /// Language-specific handler address (absolute), if the function has one of the requested kind.
    pub handler: Option<u64>,
    /// Pointer to the handler data (guest address).
    pub data: u64,
    /// Establisher frame of the unwound function.
    pub frame: u64,
}

impl Process {
    /// Registers an image's `.pdata` (called by the loaders once the image is mapped).
    pub fn register_image(&mut self, base: u64, size: u64, pdata_rva: u32, pdata_size: u32) {
        let mut funcs = Vec::new();
        if pdata_rva != 0 {
            if let Ok(b) = self.mem.read_vec(base + u64::from(pdata_rva), pdata_size as usize) {
                for c in b.chunks_exact(12) {
                    let f = |o: usize| u32::from_le_bytes(c[o..o + 4].try_into().unwrap());
                    funcs.push((f(0), f(4), f(8)));
                }
            }
        }
        self.images.push(Image { base, size, pdata_va: base + u64::from(pdata_rva), funcs });
    }

    /// `RtlLookupFunctionEntry`: `(image index, function index)` covering `pc`.
    pub fn lookup_function(&self, pc: u64) -> Option<(usize, usize)> {
        let ii = self.images.iter().position(|i| (i.base..i.base + i.size).contains(&pc))?;
        let rva = (pc - self.images[ii].base) as u32;
        let f = &self.images[ii].funcs;
        let k = f.partition_point(|e| e.0 <= rva);
        (k > 0 && rva < f[k - 1].1).then_some((ii, k - 1))
    }

    /// Guest address of a function's RUNTIME_FUNCTION entry.
    pub fn function_entry_addr(&self, ii: usize, fi: usize) -> u64 {
        self.images[ii].pdata_va + 12 * fi as u64
    }

    /// Reads a CONTEXT record.
    pub fn read_ctx(&mut self, a: u64) -> Option<Ctx> {
        let b = self.mem.read_vec(a + CTX_GPR, 16 * 8 + 8).ok()?;
        let q = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        let mut c = Ctx::default();
        for i in 0..16 {
            c.r[i] = q(i);
        }
        c.rip = q(16);
        Some(c)
    }

    /// Writes the integer registers and rip into a CONTEXT record (other fields untouched).
    pub fn write_ctx(&mut self, a: u64, c: &Ctx) {
        let mut b = Vec::with_capacity(16 * 8 + 8);
        for r in c.r {
            b.extend_from_slice(&r.to_le_bytes());
        }
        b.extend_from_slice(&c.rip.to_le_bytes());
        let _ = self.mem.write(a + CTX_GPR, &b);
        debug_assert_eq!(CTX_RIP, CTX_GPR + 16 * 8);
    }

    /// Registers as they were when the current syscall was made, as if the caller had just returned.
    pub fn capture_ctx(&self) -> Ctx {
        let mut c = Ctx { r: self.regs, rip: 0 };
        c.r[1] = self.regs[10]; // the stub moved rcx to r10
        c.r[4] = self.cur_rsp + 8;
        c.rip = self.ret_addr;
        c
    }

    /// `RtlVirtualUnwind`: undoes the effect of function `(ii, fi)` on `ctx` (a context inside it at `pc`).
    pub fn virtual_unwind(&mut self, ii: usize, fi: usize, pc: u64, htype: u32, ctx: &mut Ctx) -> Option<UnwindOut> {
        let base = self.images[ii].base;
        let (begin, _, mut info) = self.images[ii].funcs[fi];
        let start_rsp = ctx.rsp();
        let mut handler = None;
        let mut data = 0;
        let mut frame_reg_base = None;
        let mut first = true;
        let mut depth = 0;
        loop {
            let ia = base + u64::from(info);
            let h = self.mem.read_vec(ia, 4).ok()?;
            let (flags, prolog, count, fr) = (u32::from(h[0] >> 3), u64::from(h[1]), usize::from(h[2]), h[3]);
            let codes = self.mem.read_vec(ia + 4, count * 2).ok()?;
            // The offset only limits the codes while `pc` is still inside this function's prolog.
            let in_prolog = first && pc >= base + u64::from(begin) && pc < base + u64::from(begin) + prolog;
            let off = pc.saturating_sub(base + u64::from(begin));
            let mut i = 0;
            while i < count {
                let (code_off, opb) = (u64::from(codes[2 * i]), codes[2 * i + 1]);
                let (op, oi) = (opb & 15, usize::from(opb >> 4));
                let slot = |k: usize| u64::from(u16::from_le_bytes([codes[2 * (i + k)], codes[2 * (i + k) + 1]]));
                let applies = !in_prolog || code_off <= off;
                match op {
                    0 => {
                        if applies {
                            ctx.r[oi] = self.mem.read_u64(ctx.r[4]).ok()?;
                            ctx.r[4] += 8;
                        }
                    }
                    1 => {
                        let (size, n) = if oi == 0 { (slot(1) * 8, 2) } else { (slot(1) | slot(2) << 16, 3) };
                        if applies {
                            ctx.r[4] += size;
                        }
                        i += n - 1;
                    }
                    2 => {
                        if applies {
                            ctx.r[4] += oi as u64 * 8 + 8;
                        }
                    }
                    3 => {
                        if applies {
                            let (reg, off16) = ((fr & 15) as usize, u64::from(fr >> 4) * 16);
                            ctx.r[4] = ctx.r[reg].wrapping_sub(off16);
                            frame_reg_base = Some(ctx.r[reg].wrapping_sub(off16));
                        }
                    }
                    4 | 5 => {
                        let (o, n) = if op == 4 { (slot(1) * 8, 2) } else { (slot(1) | slot(2) << 16, 3) };
                        if applies {
                            ctx.r[oi] = self.mem.read_u64(ctx.r[4] + o).ok()?;
                        }
                        i += n - 1;
                    }
                    8 => i += 1,
                    9 => i += 2,
                    10 => {
                        if applies {
                            let sp = ctx.r[4] + if oi == 1 { 8 } else { 0 };
                            ctx.rip = self.mem.read_u64(sp).ok()?;
                            ctx.r[4] = self.mem.read_u64(sp + 24).ok()?;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            let tail = ia + 4 + 2 * ((count as u64 + 1) & !1);
            if flags & UNW_CHAININFO != 0 {
                let c = self.mem.read_vec(tail, 12).ok()?;
                info = u32::from_le_bytes(c[8..12].try_into().unwrap());
                first = false;
                depth += 1;
                if depth > 32 {
                    return None;
                }
                continue;
            }
            if flags & htype != 0 {
                handler = Some(base + u64::from(self.mem.read_u32(tail).ok()?));
                data = tail + 4;
            }
            break;
        }
        // Return address: rip = [rsp]; rsp += 8 (unless a machine frame already set rip).
        let establisher = frame_reg_base.unwrap_or(start_rsp);
        if self.images[ii].funcs.get(fi).is_some() {
            ctx.rip = self.mem.read_u64(ctx.r[4]).ok()?;
            ctx.r[4] += 8;
        }
        Some(UnwindOut { handler, data, frame: establisher })
    }

    /// Bump-allocates zeroed guest scratch memory (exception records, contexts); see [`Process::scratch_mark`].
    pub fn scratch_alloc(&mut self, n: u64) -> u64 {
        const SIZE: u64 = 1 << 20;
        if self.scratch == 0 {
            self.scratch = self.mem.map_anywhere(SIZE, 3, &[], "seh").unwrap_or(0);
            self.scratch_next = self.scratch;
        }
        let n = (n + 15) & !15;
        if self.scratch == 0 || self.scratch_next + n > self.scratch + SIZE {
            return 0;
        }
        let a = self.scratch_next;
        self.scratch_next += n;
        let _ = self.mem.write(a, &alloc::vec![0u8; n as usize]);
        a
    }

    /// Current scratch position (restore with [`Process::scratch_release`] when a dispatch is done).
    pub fn scratch_mark(&mut self) -> u64 {
        self.scratch_alloc(0);
        self.scratch_next
    }

    /// Frees scratch memory allocated since `mark`.
    pub fn scratch_release(&mut self, mark: u64) {
        self.scratch_next = mark;
    }

    /// Exception calls: `Some` if `name` is one of them.
    pub(crate) fn seh_syscall(&mut self, name: &str, a: [u64; 4], rsp: u64) -> Option<crate::Ret> {
        use crate::Ret;
        let v = |x: u64| Some(Ret::Value(x));
        match name {
            "RtlLookupFunctionEntry" => match self.lookup_function(a[0]) {
                Some((ii, fi)) => {
                    if a[1] != 0 {
                        let _ = self.mem.write(a[1], &self.images[ii].base.to_le_bytes());
                    }
                    v(self.function_entry_addr(ii, fi))
                }
                None => {
                    if a[1] != 0 {
                        let _ = self.mem.write(a[1], &0u64.to_le_bytes());
                    }
                    v(0)
                }
            },
            "RtlVirtualUnwind" => {
                // (type, base, pc, rf, ctx, &handlerData, &establisher, ctxptrs)
                let (ctxa, hd, ef) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 6), self.stack_arg(rsp, 7));
                let mut c = self.read_ctx(ctxa)?;
                let h = match self.lookup_function(a[2]) {
                    Some((ii, fi)) => self.virtual_unwind(ii, fi, a[2], a[0] as u32, &mut c),
                    None => {
                        c.rip = self.mem.read_u64(c.r[4]).unwrap_or(0);
                        c.r[4] += 8;
                        None
                    }
                };
                self.write_ctx(ctxa, &c);
                if let Some(o) = &h {
                    if hd != 0 {
                        let _ = self.mem.write(hd, &o.data.to_le_bytes());
                    }
                    if ef != 0 {
                        let _ = self.mem.write(ef, &o.frame.to_le_bytes());
                    }
                }
                v(h.and_then(|o| o.handler).unwrap_or(0))
            }
            "RtlCaptureContext" => {
                let c = self.capture_ctx();
                self.write_ctx(a[0], &c);
                let _ = self.mem.write(a[0] + 0x30, &0x10_000Bu32.to_le_bytes());
                let _ = self.mem.write(a[0] + 0x44, &0x202u32.to_le_bytes());
                v(0)
            }
            "SetUnhandledExceptionFilter" => {
                let old = core::mem::replace(&mut self.unhandled_filter, a[0]);
                v(old)
            }
            "RaiseException" => Some(Ret::Raise { code: a[0] as u32, flags: a[1] as u32, nargs: a[2] as u32, args: a[3] }),
            "RtlUnwindEx" => {
                let ret = a[3];
                Some(Ret::Unwind { frame: a[0], ip: a[1], rec: a[2], ret })
            }
            _ => None,
        }
    }

    /// Native function number a stub address performs (`None` if `addr` is not a syscall stub).
    pub fn stub_nr(&mut self, addr: u64) -> Option<u32> {
        let mut b = self.mem.read_vec(addr, 8).ok()?;
        if b[0..2] == [0xff, 0x25] {
            // `jmp [rip+disp]`: the import thunk the linker puts in front of a DLL function.
            let slot = addr.wrapping_add(6).wrapping_add(i64::from(i32::from_le_bytes(b[2..6].try_into().unwrap())) as u64);
            let target = self.mem.read_u64(slot).ok()?;
            b = self.mem.read_vec(target, 8).ok()?;
        }
        (b[0..4] == [0x4c, 0x8b, 0xd1, 0xb8]).then(|| u32::from_le_bytes(b[4..8].try_into().unwrap()))
    }
}
