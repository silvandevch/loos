//! AArch64 user-mode interpreter: runs Apple Silicon (arm64) programs on the x86-64 host.
//!
//! The interpreter executes A64 integer, load/store (including LSE atomics and exclusives),
//! branch, scalar floating-point and the common SIMD moves directly against the process's
//! [`UserMem`]; it stops at `svc` (Darwin system calls), at a branch into Prism's libSystem
//! stubs, at faults and at undefined instructions. It is deliberately separate from the
//! native x86-64 path: an arm64 process never enters ring 3.

use crate::libsys::{Abi, Args};
use crate::loader::{SHIM_BASE, STUB_SIZE};
use loos_lin::mem::UserMem;

/// Why [`Cpu::run`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// `svc` (the immediate); syscall number in x16.
    Svc(u16),
    /// Branch into libSystem stub `i` (arguments in x0-x7 / d0-d7, return to x30).
    Shim(usize),
    /// Memory access fault at this address.
    Fault(u64),
    /// Undefined or unsupported instruction (the encoding), at `pc`.
    Undefined(u32),
    /// `brk`.
    Brk,
    /// The instruction budget ran out.
    Budget,
}

/// Size of the libSystem stub range the interpreter traps on.
const SHIM_SPAN: u64 = 0x10_0000;

/// Architectural state of one arm64 thread.
#[derive(Clone, Debug)]
pub struct Cpu {
    /// x0-x30.
    pub x: [u64; 31],
    /// Stack pointer.
    pub sp: u64,
    /// Program counter.
    pub pc: u64,
    /// NZCV flags.
    pub n: bool,
    /// Zero.
    pub z: bool,
    /// Carry.
    pub c: bool,
    /// Overflow.
    pub v: bool,
    /// v0-v31.
    pub q: [u128; 32],
    /// TPIDR_EL0 / TPIDRRO_EL0 (thread pointer; Darwin keeps its TSD pointer in TPIDRRO_EL0).
    pub tpidr: u64,
    /// TPIDRRO_EL0.
    pub tpidrro: u64,
    /// FPCR / FPSR.
    pub fpcr: u64,
    fpsr: u64,
    /// Exclusive monitor address.
    excl: Option<u64>,
    code_page: u64,
    code: [u8; 4096],
    /// Instructions executed.
    pub icount: u64,
}

type R<T> = Result<T, Stop>;

fn sext(v: u64, bits: u32) -> i64 {
    ((v << (64 - bits)) as i64) >> (64 - bits)
}

fn ones(n: u32) -> u64 {
    if n >= 64 {
        u64::MAX
    } else {
        (1u64 << n) - 1
    }
}

fn ror(v: u64, r: u32, size: u32) -> u64 {
    let r = r % size;
    let v = v & ones(size);
    if r == 0 {
        v
    } else {
        ((v >> r) | (v << (size - r))) & ones(size)
    }
}

fn replicate(v: u64, esize: u32) -> u64 {
    let mut out = 0;
    let mut i = 0;
    while i < 64 {
        out |= (v & ones(esize)) << i;
        i += esize;
    }
    out
}

/// DecodeBitMasks from the Arm ARM: (wmask, tmask).
fn bit_masks(n: u32, imms: u32, immr: u32, size: u32, logical: bool) -> Option<(u64, u64)> {
    let x = (n << 6) | (!imms & 0x3f);
    if x == 0 {
        return None;
    }
    let len = 31 - x.leading_zeros();
    if len < 1 {
        return None;
    }
    let levels = (1u32 << len) - 1;
    let s = imms & levels;
    let r = immr & levels;
    let esize = 1u32 << len;
    if esize > size || logical && s == levels {
        return None;
    }
    let d = s.wrapping_sub(r) & levels;
    let welem = ones(s + 1);
    let telem = ones(d + 1);
    let w = replicate(ror(welem, r, esize), esize);
    let t = replicate(telem, esize);
    Some((w & ones(size), t & ones(size)))
}

fn sqrt(x: f64) -> f64 {
    if x <= 0.0 || x.is_nan() || x.is_infinite() {
        return if x == 0.0 || x.is_infinite() && x > 0.0 { x } else { f64::NAN };
    }
    let mut g = f64::from_bits((x.to_bits() >> 1) + (1023u64 << 51));
    for _ in 0..6 {
        g = 0.5 * (g + x / g);
    }
    g
}

fn floor(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 4.5e15 {
        return x;
    }
    let t = x as i64 as f64;
    if t > x {
        t - 1.0
    } else {
        t
    }
}

fn round_half_away(x: f64) -> f64 {
    if x < 0.0 {
        -floor(-x + 0.5)
    } else {
        floor(x + 0.5)
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl Cpu {
    /// A thread entering at `pc` with stack `sp`.
    pub fn new(pc: u64, sp: u64) -> Self {
        Self { x: [0; 31], sp, pc, n: false, z: false, c: false, v: false, q: [0; 32], tpidr: 0, tpidrro: 0, fpcr: 0, fpsr: 0, excl: None, code_page: u64::MAX, code: [0; 4096], icount: 0 }
    }

    /// Register `r` with 31 = zero register.
    fn xr(&self, r: u32) -> u64 {
        if r == 31 {
            0
        } else {
            self.x[r as usize]
        }
    }
    /// Register `r` with 31 = SP.
    fn xs(&self, r: u32) -> u64 {
        if r == 31 {
            self.sp
        } else {
            self.x[r as usize]
        }
    }
    fn set(&mut self, r: u32, v: u64, sf: bool) {
        if r != 31 {
            self.x[r as usize] = if sf { v } else { v & 0xffff_ffff };
        }
    }
    fn set_sp(&mut self, r: u32, v: u64, sf: bool) {
        let v = if sf { v } else { v & 0xffff_ffff };
        if r == 31 {
            self.sp = v;
        } else {
            self.x[r as usize] = v;
        }
    }

    fn cond(&self, c: u32) -> bool {
        let r = match c >> 1 {
            0 => self.z,
            1 => self.c,
            2 => self.n,
            3 => self.v,
            4 => self.c && !self.z,
            5 => self.n == self.v,
            6 => self.n == self.v && !self.z,
            _ => true,
        };
        if c & 1 == 1 && c != 15 {
            !r
        } else {
            r
        }
    }

    fn add_with_carry(&mut self, a: u64, b: u64, carry: bool, sf: bool, flags: bool) -> u64 {
        let (r, nz, c, v) = if sf {
            let (s1, c1) = a.overflowing_add(b);
            let (s, c2) = s1.overflowing_add(carry as u64);
            let v = ((a ^ s) & (b ^ s)) >> 63 != 0;
            (s, s, c1 || c2, v)
        } else {
            let (a, b) = (a as u32, b as u32);
            let (s1, c1) = a.overflowing_add(b);
            let (s, c2) = s1.overflowing_add(carry as u32);
            let v = ((a ^ s) & (b ^ s)) >> 31 != 0;
            (s as u64, (s as i32) as i64 as u64, c1 || c2, v)
        };
        if flags {
            self.n = (nz as i64) < 0;
            self.z = r == 0;
            self.c = c;
            self.v = v;
        }
        r
    }

    fn logic_flags(&mut self, r: u64, sf: bool) {
        self.n = if sf { (r as i64) < 0 } else { (r as i32) < 0 };
        self.z = if sf { r == 0 } else { r as u32 == 0 };
        self.c = false;
        self.v = false;
    }

    fn shift(v: u64, kind: u32, amt: u32, sf: bool) -> u64 {
        let size = if sf { 64 } else { 32 };
        let v = v & ones(size);
        let amt = amt % size;
        match kind {
            0 => (v << amt) & ones(size),
            1 => v >> amt,
            2 => {
                if sf {
                    ((v as i64) >> amt) as u64
                } else {
                    (((v as i32) >> amt) as u32) as u64
                }
            }
            _ => ror(v, amt, size),
        }
    }

    fn extend(v: u64, option: u32, shift: u32) -> u64 {
        let e = match option {
            0 => v as u8 as u64,
            1 => v as u16 as u64,
            2 => v as u32 as u64,
            3 => v,
            4 => v as i8 as i64 as u64,
            5 => v as i16 as i64 as u64,
            6 => v as i32 as i64 as u64,
            _ => v,
        };
        e << shift
    }

    // -------------------------------------------------- memory

    fn load(&mut self, mem: &mut UserMem, addr: u64, size: u32) -> R<u64> {
        let mut b = [0u8; 8];
        mem.read(addr, &mut b[..size as usize]).map_err(|_| Stop::Fault(addr))?;
        Ok(u64::from_le_bytes(b))
    }
    fn store(&mut self, mem: &mut UserMem, addr: u64, size: u32, v: u64) -> R<()> {
        if self.excl.is_some_and(|e| e & !63 == addr & !63) {
            self.excl = None;
        }
        // Stores into the code page currently cached must be seen by the fetcher.
        if addr & !4095 == self.code_page || (addr + size as u64 - 1) & !4095 == self.code_page {
            self.code_page = u64::MAX;
        }
        mem.write(addr, &v.to_le_bytes()[..size as usize]).map_err(|_| Stop::Fault(addr))
    }
    fn load_q(&mut self, mem: &mut UserMem, addr: u64, size: u32) -> R<u128> {
        if size <= 8 {
            return self.load(mem, addr, size).map(|v| v as u128);
        }
        let mut b = [0u8; 16];
        mem.read(addr, &mut b).map_err(|_| Stop::Fault(addr))?;
        Ok(u128::from_le_bytes(b))
    }
    fn store_q(&mut self, mem: &mut UserMem, addr: u64, size: u32, v: u128) -> R<()> {
        if size <= 8 {
            return self.store(mem, addr, size, v as u64);
        }
        self.store(mem, addr, 8, v as u64)?;
        self.store(mem, addr + 8, 8, (v >> 64) as u64)
    }

    fn fetch(&mut self, mem: &mut UserMem) -> R<u32> {
        let page = self.pc & !4095;
        if page != self.code_page {
            mem.read(page, &mut self.code).map_err(|_| Stop::Fault(self.pc))?;
            self.code_page = page;
        }
        let o = (self.pc & 4095) as usize;
        Ok(u32::from_le_bytes(self.code[o..o + 4].try_into().unwrap()))
    }

    /// Run up to `budget` instructions.
    pub fn run(&mut self, mem: &mut UserMem, budget: u64) -> Stop {
        for _ in 0..budget {
            if self.pc.wrapping_sub(SHIM_BASE) < SHIM_SPAN {
                return Stop::Shim(((self.pc - SHIM_BASE) / STUB_SIZE) as usize);
            }
            if self.pc & 3 != 0 {
                return Stop::Fault(self.pc);
            }
            let ins = match self.fetch(mem) {
                Ok(i) => i,
                Err(s) => return s,
            };
            self.icount += 1;
            match self.step(mem, ins) {
                Ok(true) => {}
                Ok(false) => self.pc = self.pc.wrapping_add(4),
                Err(s) => return s,
            }
        }
        Stop::Budget
    }

    /// Execute one instruction. Ok(true): the pc was set; Ok(false): fall through.
    fn step(&mut self, mem: &mut UserMem, i: u32) -> R<bool> {
        let rd = i & 31;
        let rn = (i >> 5) & 31;
        let rm = (i >> 16) & 31;
        let sf = i >> 31 == 1;
        let undef = Err(Stop::Undefined(i));
        match (i >> 25) & 0xf {
            // ---------------- data processing (immediate)
            0b1000 | 0b1001 => match (i >> 23) & 7 {
                0 | 1 => {
                    let imm = sext((((i >> 5) & 0x7ffff) << 2 | (i >> 29) & 3) as u64, 21);
                    let v = if sf { (self.pc & !4095).wrapping_add((imm << 12) as u64) } else { self.pc.wrapping_add(imm as u64) };
                    self.set(rd, v, true);
                }
                2 => {
                    let imm = (((i >> 10) & 0xfff) as u64) << if (i >> 22) & 1 == 1 { 12 } else { 0 };
                    let (sub, s) = ((i >> 30) & 1 == 1, (i >> 29) & 1 == 1);
                    let a = self.xs(rn);
                    let r = if sub { self.add_with_carry(a, !imm, true, sf, s) } else { self.add_with_carry(a, imm, false, sf, s) };
                    if s {
                        self.set(rd, r, sf);
                    } else {
                        self.set_sp(rd, r, sf);
                    }
                }
                4 => {
                    let size = if sf { 64 } else { 32 };
                    let Some((imm, _)) = bit_masks((i >> 22) & 1, (i >> 10) & 0x3f, (i >> 16) & 0x3f, size, true) else { return undef };
                    let a = self.xr(rn);
                    match (i >> 29) & 3 {
                        0 => self.set_sp(rd, a & imm, sf),
                        1 => self.set_sp(rd, a | imm, sf),
                        2 => self.set_sp(rd, a ^ imm, sf),
                        _ => {
                            let r = a & imm & ones(size);
                            self.logic_flags(r, sf);
                            self.set(rd, r, sf);
                        }
                    }
                }
                5 => {
                    let hw = (i >> 21) & 3;
                    let imm = (((i >> 5) & 0xffff) as u64) << (hw * 16);
                    match (i >> 29) & 3 {
                        0 => self.set(rd, !imm, sf),
                        2 => self.set(rd, imm, sf),
                        3 => {
                            let keep = self.xr(rd) & !(0xffffu64 << (hw * 16));
                            self.set(rd, keep | imm, sf);
                        }
                        _ => return undef,
                    }
                }
                6 => {
                    let size = if sf { 64 } else { 32 };
                    let (immr, imms) = ((i >> 16) & 0x3f, (i >> 10) & 0x3f);
                    let Some((w, t)) = bit_masks((i >> 22) & 1, imms, immr, size, false) else { return undef };
                    let src = self.xr(rn) & ones(size);
                    let opc = (i >> 29) & 3;
                    let dst = if opc == 1 { self.xr(rd) & ones(size) } else { 0 };
                    let bot = (dst & !w) | (ror(src, immr, size) & w);
                    let top = match opc {
                        0 => {
                            if (src >> imms) & 1 == 1 {
                                ones(size)
                            } else {
                                0
                            }
                        }
                        1 => dst,
                        2 => 0,
                        _ => return undef,
                    };
                    self.set(rd, ((top & !t) | (bot & t)) & ones(size), sf);
                }
                7 => {
                    let lsb = (i >> 10) & 0x3f;
                    let size = if sf { 64 } else { 32 };
                    let (hi, lo) = (self.xr(rn) & ones(size), self.xr(rm) & ones(size));
                    let r = if lsb == 0 { lo } else { ((lo >> lsb) | (hi << (size - lsb))) & ones(size) };
                    self.set(rd, r, sf);
                }
                _ => return undef,
            },
            // ---------------- branches, exceptions, system
            0b1010 | 0b1011 => {
                if i & 0x7c00_0000 == 0x1400_0000 {
                    let off = sext((i & 0x03ff_ffff) as u64, 26) << 2;
                    if sf {
                        self.x[30] = self.pc + 4;
                    }
                    self.pc = self.pc.wrapping_add(off as u64);
                    return Ok(true);
                }
                if i & 0xff00_0010 == 0x5400_0000 {
                    if self.cond(i & 15) {
                        self.pc = self.pc.wrapping_add((sext(((i >> 5) & 0x7ffff) as u64, 19) << 2) as u64);
                        return Ok(true);
                    }
                    return Ok(false);
                }
                if i & 0x7e00_0000 == 0x3400_0000 {
                    let v = if sf { self.xr(rd) } else { self.xr(rd) & 0xffff_ffff };
                    if (v == 0) != ((i >> 24) & 1 == 1) {
                        self.pc = self.pc.wrapping_add((sext(((i >> 5) & 0x7ffff) as u64, 19) << 2) as u64);
                        return Ok(true);
                    }
                    return Ok(false);
                }
                if i & 0x7e00_0000 == 0x3600_0000 {
                    let bit = ((i >> 31) << 5) | ((i >> 19) & 31);
                    let set = (self.xr(rd) >> bit) & 1 == 1;
                    if set == ((i >> 24) & 1 == 1) {
                        self.pc = self.pc.wrapping_add((sext(((i >> 5) & 0x3fff) as u64, 14) << 2) as u64);
                        return Ok(true);
                    }
                    return Ok(false);
                }
                if i & 0xfe00_0000 == 0xd600_0000 {
                    let opc = (i >> 21) & 0xf;
                    let op3 = (i >> 10) & 0x3f;
                    if (i >> 16) & 31 != 31 {
                        return undef;
                    }
                    let target = if opc == 2 && op3 != 0 { self.x[30] } else { self.xr(rn) };
                    match opc {
                        0 | 8 => {}
                        1 | 9 => self.x[30] = self.pc + 4,
                        2 => {}
                        _ => return undef,
                    }
                    self.pc = target;
                    return Ok(true);
                }
                if i & 0xffe0_001f == 0xd400_0001 {
                    return Err(Stop::Svc(((i >> 5) & 0xffff) as u16));
                }
                if i & 0xffe0_001f == 0xd420_0000 {
                    return Err(Stop::Brk);
                }
                if i & 0xffff_f01f == 0xd503_201f || i & 0xffff_f01f == 0xd503_301f || i == 0xd500_401f {
                    return Ok(false); // hints (incl. PAC/BTI), barriers, clrex
                }
                if i & 0xfff0_0000 == 0xd530_0000 {
                    let v = match (i >> 5) & 0x7fff {
                        0x5e82 => self.tpidr,
                        0x5e83 => self.tpidrro,
                        0x5a10 => (self.n as u64) << 31 | (self.z as u64) << 30 | (self.c as u64) << 29 | (self.v as u64) << 28,
                        0x5a20 => self.fpcr,
                        0x5a21 => self.fpsr,
                        0x5f02 | 0x5f01 => self.icount,
                        0x5f00 => 1_000_000_000,
                        0x5807 => 0x10,
                        0x4006 => 0, // ID registers read as zero
                        _ => return undef,
                    };
                    self.set(rd, v, true);
                    return Ok(false);
                }
                if i & 0xfff0_0000 == 0xd510_0000 {
                    let v = self.xr(rd);
                    match (i >> 5) & 0x7fff {
                        0x5e82 => self.tpidr = v,
                        0x5a10 => {
                            self.n = v >> 31 & 1 == 1;
                            self.z = v >> 30 & 1 == 1;
                            self.c = v >> 29 & 1 == 1;
                            self.v = v >> 28 & 1 == 1;
                        }
                        0x5a20 => self.fpcr = v,
                        0x5a21 => self.fpsr = v,
                        _ => return undef,
                    }
                    return Ok(false);
                }
                if i & 0xfff8_0000 == 0xd508_0000 {
                    // SYS (DC ZVA, IC IVAU, DC CVAU...): DC ZVA zeroes a 64-byte block, the rest are no-ops here.
                    if (i >> 5) & 0x3fff == 0x1ba1 {
                        let a = self.xr(rd) & !63;
                        for k in 0..8 {
                            self.store(mem, a + k * 8, 8, 0)?;
                        }
                    }
                    return Ok(false);
                }
                return undef;
            }
            // ---------------- loads and stores
            0b0100 | 0b0110 | 0b1100 | 0b1110 => return self.ldst(mem, i).map(|_| false),
            // ---------------- data processing (register)
            0b0101 | 0b1101 => self.dp_reg(i)?,
            // ---------------- SIMD & floating point
            0b0111 | 0b1111 => self.fp(i)?,
            _ => return undef,
        }
        Ok(false)
    }

    fn dp_reg(&mut self, i: u32) -> R<()> {
        let rd = i & 31;
        let rn = (i >> 5) & 31;
        let rm = (i >> 16) & 31;
        let sf = i >> 31 == 1;
        let size = if sf { 64 } else { 32 };
        let undef = Err(Stop::Undefined(i));
        // Pointer authentication. Prism loads arm64e code with unsigned pointers (no PAC keys), so
        // signing and authenticating leave the pointer as it is and stripping is a no-op.
        if i & 0xffff_0000 == 0xdac1_0000 && (i >> 10) & 0x3f <= 0x11 {
            return Ok(()); // PACIA/PACIB/PACDA/PACDB, AUTxx, the Z forms, XPACI/XPACD
        }
        if i & 0xffe0_fc00 == 0x9ac0_3000 {
            // PACGA: a generic signature in the top 32 bits.
            let v = (self.xr(rn) ^ self.xr(rm)) & 0xffff_ffff_0000_0000;
            self.set(rd, v, true);
            return Ok(());
        }
        if i & 0x1f00_0000 == 0x0a00_0000 {
            let mut b = Self::shift(self.xr(rm), (i >> 22) & 3, (i >> 10) & 0x3f, sf);
            if (i >> 21) & 1 == 1 {
                b = !b & ones(size);
            }
            let a = self.xr(rn) & ones(size);
            let r = match (i >> 29) & 3 {
                0 => a & b,
                1 => a | b,
                2 => a ^ b,
                _ => {
                    let r = a & b;
                    self.logic_flags(r, sf);
                    r
                }
            };
            self.set(rd, r, sf);
            return Ok(());
        }
        if i & 0x1f20_0000 == 0x0b00_0000 || i & 0x1f20_0000 == 0x0b20_0000 {
            let ext = (i >> 21) & 1 == 1;
            let (sub, s) = ((i >> 30) & 1 == 1, (i >> 29) & 1 == 1);
            let (a, b) = if ext {
                (self.xs(rn), Self::extend(self.xr(rm), (i >> 13) & 7, (i >> 10) & 7))
            } else {
                (self.xr(rn), Self::shift(self.xr(rm), (i >> 22) & 3, (i >> 10) & 0x3f, sf))
            };
            let r = if sub { self.add_with_carry(a, !b, true, sf, s) } else { self.add_with_carry(a, b, false, sf, s) };
            if ext && !s {
                self.set_sp(rd, r, sf);
            } else {
                self.set(rd, r, sf);
            }
            return Ok(());
        }
        if i & 0x1fe0_fc00 == 0x1a00_0000 {
            let (sub, s) = ((i >> 30) & 1 == 1, (i >> 29) & 1 == 1);
            let (a, b) = (self.xr(rn), self.xr(rm));
            let c = self.c;
            let r = self.add_with_carry(a, if sub { !b } else { b }, c, sf, s);
            self.set(rd, r, sf);
            return Ok(());
        }
        if i & 0x1fe0_0400 == 0x1a40_0000 && (i >> 29) & 1 == 1 {
            // CCMN / CCMP (register or immediate)
            let b = if (i >> 11) & 1 == 1 { rm as u64 } else { self.xr(rm) };
            if self.cond((i >> 12) & 15) {
                let a = self.xr(rn);
                if (i >> 30) & 1 == 1 {
                    self.add_with_carry(a, !b, true, sf, true);
                } else {
                    self.add_with_carry(a, b, false, sf, true);
                }
            } else {
                let f = i & 15;
                self.n = f & 8 != 0;
                self.z = f & 4 != 0;
                self.c = f & 2 != 0;
                self.v = f & 1 != 0;
            }
            return Ok(());
        }
        if i & 0x1fe0_0000 == 0x1a80_0000 {
            let (op, op2) = ((i >> 30) & 1, (i >> 10) & 3);
            let r = if self.cond((i >> 12) & 15) {
                self.xr(rn)
            } else {
                let b = self.xr(rm);
                match (op, op2) {
                    (0, 0) => b,
                    (0, 1) => b.wrapping_add(1),
                    (1, 0) => !b,
                    (1, 1) => b.wrapping_neg(),
                    _ => return undef,
                }
            };
            self.set(rd, r, sf);
            return Ok(());
        }
        if i & 0x5fe0_0000 == 0x1ac0_0000 {
            let (a, b) = (self.xr(rn) & ones(size), self.xr(rm) & ones(size));
            let r = match (i >> 10) & 0x3f {
                2 => a.checked_div(b).unwrap_or(0),
                3 => {
                    if b == 0 {
                        0
                    } else if sf {
                        (a as i64).wrapping_div(b as i64) as u64
                    } else {
                        ((a as i32).wrapping_div(b as i32)) as u32 as u64
                    }
                }
                8 => Self::shift(a, 0, (b % size as u64) as u32, sf),
                9 => Self::shift(a, 1, (b % size as u64) as u32, sf),
                10 => Self::shift(a, 2, (b % size as u64) as u32, sf),
                11 => Self::shift(a, 3, (b % size as u64) as u32, sf),
                _ => return undef,
            };
            self.set(rd, r, sf);
            return Ok(());
        }
        if i & 0x5fe0_0000 == 0x5ac0_0000 {
            let a = self.xr(rn) & ones(size);
            let r = match (i >> 10) & 0x3f {
                0 => {
                    if sf {
                        a.reverse_bits()
                    } else {
                        (a as u32).reverse_bits() as u64
                    }
                }
                1 => {
                    let mut r = 0;
                    for k in (0..size).step_by(16) {
                        r |= (((a >> k) as u16).swap_bytes() as u64) << k;
                    }
                    r
                }
                2 => {
                    if sf {
                        ((a as u32).swap_bytes() as u64) | (((a >> 32) as u32).swap_bytes() as u64) << 32
                    } else {
                        (a as u32).swap_bytes() as u64
                    }
                }
                3 => a.swap_bytes(),
                4 => {
                    if sf {
                        a.leading_zeros() as u64
                    } else {
                        (a as u32).leading_zeros() as u64
                    }
                }
                5 => {
                    if sf {
                        ((a as i64 ^ ((a as i64) >> 1)).leading_zeros() as u64).saturating_sub(1).max(0)
                    } else {
                        (((a as i32) ^ ((a as i32) >> 1)).leading_zeros() as u64).saturating_sub(1)
                    }
                }
                _ => return undef,
            };
            self.set(rd, r, sf);
            return Ok(());
        }
        if i & 0x1f00_0000 == 0x1b00_0000 {
            let ra = (i >> 10) & 31;
            let (a, b, c) = (self.xr(rn), self.xr(rm), self.xr(ra));
            let sub = (i >> 15) & 1 == 1;
            let r = match (i >> 21) & 7 {
                0 => {
                    let p = a.wrapping_mul(b);
                    if sub { c.wrapping_sub(p) } else { c.wrapping_add(p) }
                }
                1 => {
                    let p = (a as i32 as i64).wrapping_mul(b as i32 as i64) as u64;
                    if sub { c.wrapping_sub(p) } else { c.wrapping_add(p) }
                }
                5 => {
                    let p = (a as u32 as u64).wrapping_mul(b as u32 as u64);
                    if sub { c.wrapping_sub(p) } else { c.wrapping_add(p) }
                }
                2 => ((a as i64 as i128 * b as i64 as i128) >> 64) as u64,
                6 => ((a as u128 * b as u128) >> 64) as u64,
                _ => return undef,
            };
            self.set(rd, r, sf || matches!((i >> 21) & 7, 1 | 2 | 5 | 6));
            return Ok(());
        }
        undef
    }

    fn ldst(&mut self, mem: &mut UserMem, i: u32) -> R<()> {
        if i & 0xff20_0400 == 0xf820_0400 {
            // LDRAA/LDRAB: authenticate the (unsigned) base, load 64 bits at base + simm10 * 8.
            let (rt, rn) = (i & 31, (i >> 5) & 31);
            let imm = ((i >> 22) & 1) << 9 | (i >> 12) & 0x1ff;
            let off = (sext(imm as u64, 10) << 3) as u64;
            let addr = self.xs(rn).wrapping_add(off);
            let v = self.load(mem, addr, 8)?;
            if (i >> 11) & 1 == 1 {
                self.set_sp(rn, addr, true);
            }
            self.set(rt, v, true);
            return Ok(());
        }
        let rt = i & 31;
        let rn = (i >> 5) & 31;
        let undef = Err(Stop::Undefined(i));
        let vec = (i >> 26) & 1 == 1;
        // Load/store pair.
        if i & 0x3a00_0000 == 0x2800_0000 {
            let opc = i >> 30;
            let load = (i >> 22) & 1 == 1;
            let rt2 = (i >> 10) & 31;
            let (size, signed) = if vec {
                (4u32 << opc, false)
            } else {
                match opc {
                    0 => (4, false),
                    1 => (4, true),
                    2 => (8, false),
                    _ => return undef,
                }
            };
            let off = sext(((i >> 15) & 0x7f) as u64, 7) * size as i64;
            let mode = (i >> 23) & 3;
            let base = self.xs(rn);
            let addr = if mode == 1 { base } else { base.wrapping_add(off as u64) };
            if vec {
                if load {
                    let a = self.load_q(mem, addr, size)?;
                    let b = self.load_q(mem, addr + size as u64, size)?;
                    self.q[rt as usize] = a;
                    self.q[rt2 as usize] = b;
                } else {
                    let (a, b) = (self.q[rt as usize], self.q[rt2 as usize]);
                    self.store_q(mem, addr, size, a)?;
                    self.store_q(mem, addr + size as u64, size, b)?;
                }
            } else if load {
                let mut a = self.load(mem, addr, size)?;
                let mut b = self.load(mem, addr + size as u64, size)?;
                if signed {
                    a = a as i32 as i64 as u64;
                    b = b as i32 as i64 as u64;
                }
                self.set(rt, a, true);
                self.set(rt2, b, true);
            } else {
                let (a, b) = (self.xr(rt), self.xr(rt2));
                self.store(mem, addr, size, a)?;
                self.store(mem, addr + size as u64, size, b)?;
            }
            if mode == 1 || mode == 3 {
                self.set_sp(rn, base.wrapping_add(off as u64), true);
            }
            return Ok(());
        }
        // Load literal.
        if i & 0x3b00_0000 == 0x1800_0000 {
            let addr = self.pc.wrapping_add((sext(((i >> 5) & 0x7ffff) as u64, 19) << 2) as u64);
            let opc = i >> 30;
            if vec {
                let size = 4u32 << opc;
                self.q[rt as usize] = self.load_q(mem, addr, size)?;
            } else {
                match opc {
                    0 => {
                        let v = self.load(mem, addr, 4)?;
                        self.set(rt, v, true);
                    }
                    1 => {
                        let v = self.load(mem, addr, 8)?;
                        self.set(rt, v, true);
                    }
                    2 => {
                        let v = self.load(mem, addr, 4)? as i32 as i64 as u64;
                        self.set(rt, v, true);
                    }
                    _ => {} // PRFM
                }
            }
            return Ok(());
        }
        // Exclusives, load-acquire/store-release, compare-and-swap.
        if i & 0x3f00_0000 == 0x0800_0000 {
            let size = 1u32 << (i >> 30);
            let (o2, l, o1) = ((i >> 23) & 1, (i >> 22) & 1, (i >> 21) & 1);
            let rs = (i >> 16) & 31;
            let rt2 = (i >> 10) & 31;
            let addr = self.xs(rn);
            match (o2, o1) {
                (0, _) => {
                    let pair = o1 == 1;
                    if l == 1 {
                        let v = self.load(mem, addr, size)?;
                        self.set(rt, v, true);
                        if pair {
                            let w = self.load(mem, addr + size as u64, size)?;
                            self.set(rt2, w, true);
                        }
                        self.excl = Some(addr);
                    } else {
                        // One thread per process: the monitor only breaks on our own stores.
                        let ok = self.excl == Some(addr);
                        if ok {
                            let v = self.xr(rt);
                            self.store(mem, addr, size, v)?;
                            if pair {
                                let w = self.xr(rt2);
                                self.store(mem, addr + size as u64, size, w)?;
                            }
                        }
                        self.excl = None;
                        self.set(rs, (!ok) as u64, false);
                    }
                }
                (1, 0) => {
                    if l == 1 {
                        let v = self.load(mem, addr, size)?;
                        self.set(rt, v, true);
                    } else {
                        let v = self.xr(rt);
                        self.store(mem, addr, size, v)?;
                    }
                }
                _ => {
                    // CAS{A,L,AL}{B,H,} : Rs = compare value (and receives the old value), Rt = new value.
                    let old = self.load(mem, addr, size)?;
                    if old == self.xr(rs) & ones(size * 8) {
                        let v = self.xr(rt);
                        self.store(mem, addr, size, v)?;
                    }
                    self.set(rs, old, true);
                }
            }
            return Ok(());
        }
        if i & 0x3a00_0000 != 0x3800_0000 {
            return undef;
        }
        let size_bits = i >> 30;
        let opc = (i >> 22) & 3;
        // (bytes, load, signed-to: 0 none / 32 / 64)
        let (bytes, load, sx) = if vec {
            let b = if opc & 2 != 0 { if size_bits == 0 { 16 } else { return undef } } else { 1u32 << size_bits };
            (b, opc & 1 == 1, 0)
        } else {
            match opc {
                0 => (1u32 << size_bits, false, 0),
                1 => (1u32 << size_bits, true, 0),
                2 if size_bits == 3 => return Ok(()), // PRFM
                2 => (1u32 << size_bits, true, 64),
                _ => {
                    if size_bits >= 2 {
                        return undef;
                    }
                    (1u32 << size_bits, true, 32)
                }
            }
        };
        let scale = bytes.trailing_zeros();
        let base = self.xs(rn);
        let (addr, wb) = if i & 0x0100_0000 != 0 {
            (base.wrapping_add((((i >> 10) & 0xfff) as u64) << scale), None)
        } else if (i >> 21) & 1 == 1 {
            match (i >> 10) & 3 {
                2 => {
                    let s = if (i >> 12) & 1 == 1 { scale } else { 0 };
                    (base.wrapping_add(Self::extend(self.xr((i >> 16) & 31), (i >> 13) & 7, s)), None)
                }
                0 if !vec => return self.atomic(mem, i, bytes),
                _ => return undef,
            }
        } else {
            let imm = sext(((i >> 12) & 0x1ff) as u64, 9) as u64;
            match (i >> 10) & 3 {
                0 | 2 => (base.wrapping_add(imm), None),
                1 => (base, Some(base.wrapping_add(imm))),
                _ => (base.wrapping_add(imm), Some(base.wrapping_add(imm))),
            }
        };
        if vec {
            if load {
                self.q[rt as usize] = self.load_q(mem, addr, bytes)?;
            } else {
                let v = self.q[rt as usize];
                self.store_q(mem, addr, bytes, v)?;
            }
        } else if load {
            let v = self.load(mem, addr, bytes)?;
            let v = match sx {
                0 => v,
                bits => {
                    let s = sext(v, bytes * 8) as u64;
                    if bits == 32 { s & 0xffff_ffff } else { s }
                }
            };
            self.set(rt, v, true);
        } else {
            let v = self.xr(rt);
            self.store(mem, addr, bytes, v)?;
        }
        if let Some(n) = wb {
            self.set_sp(rn, n, true);
        }
        Ok(())
    }

    fn atomic(&mut self, mem: &mut UserMem, i: u32, bytes: u32) -> R<()> {
        let rt = i & 31;
        let rs = (i >> 16) & 31;
        let addr = self.xs((i >> 5) & 31);
        let old = self.load(mem, addr, bytes)?;
        let s = self.xr(rs) & ones(bytes * 8);
        let bits = bytes * 8;
        let (so, ss) = (sext(old, bits), sext(s, bits));
        let new = match ((i >> 15) & 1, (i >> 12) & 7) {
            (0, 0) => old.wrapping_add(s),
            (0, 1) => old & !s,
            (0, 2) => old ^ s,
            (0, 3) => old | s,
            (0, 4) => if so > ss { old } else { s },
            (0, 5) => if so < ss { old } else { s },
            (0, 6) => old.max(s),
            (0, 7) => old.min(s),
            (1, 0) => s,
            _ => return Err(Stop::Undefined(i)),
        };
        self.store(mem, addr, bytes, new)?;
        self.set(rt, old, true);
        Ok(())
    }

    fn fp(&mut self, i: u32) -> R<()> {
        let rd = (i & 31) as usize;
        let rn = ((i >> 5) & 31) as usize;
        let rm = ((i >> 16) & 31) as usize;
        let undef = Err(Stop::Undefined(i));
        // Scalar FP (types 00 single, 01 double).
        if i & 0x5f20_0000 == 0x1e20_0000 {
            let dbl = match (i >> 22) & 3 {
                0 => false,
                1 => true,
                _ => return undef,
            };
            let get = |s: &Self, r: usize| if dbl { f64::from_bits(s.q[r] as u64) } else { f32::from_bits(s.q[r] as u32) as f64 };
            let put = |s: &mut Self, r: usize, v: f64| s.q[r] = if dbl { v.to_bits() as u128 } else { (v as f32).to_bits() as u128 };
            if (i >> 10) & 0x3f == 0 {
                // int <-> fp conversions and moves
                let sf = i >> 31 == 1;
                let (rmode, op) = ((i >> 19) & 3, (i >> 16) & 7);
                match (rmode, op) {
                    (0, 2) | (0, 3) => {
                        let x = self.xr(rn as u32);
                        let v = match (op == 2, sf) {
                            (true, true) => x as i64 as f64,
                            (true, false) => x as i32 as f64,
                            (false, true) => x as f64,
                            (false, false) => x as u32 as f64,
                        };
                        put(self, rd, v);
                    }
                    (_, 0) | (_, 1) | (0, 4) | (0, 5) => {
                        let x = get(self, rn);
                        let x = match (rmode, op) {
                            (0, _) => round_half_away(x),
                            (1, _) => -floor(-x),
                            (2, _) => floor(x),
                            _ => x,
                        };
                        let unsigned = op & 1 == 1;
                        let v = match (unsigned, sf) {
                            (false, true) => x as i64 as u64,
                            (false, false) => x as i32 as u32 as u64,
                            (true, true) => x as u64,
                            (true, false) => x as u32 as u64,
                        };
                        self.set(rd as u32, v, true);
                    }
                    (0, 6) => {
                        let v = if sf { self.q[rn] as u64 } else { self.q[rn] as u32 as u64 };
                        self.set(rd as u32, v, true);
                    }
                    (0, 7) => self.q[rd] = if sf { self.xr(rn as u32) as u128 } else { self.xr(rn as u32) as u32 as u128 },
                    _ => return undef,
                }
                return Ok(());
            }
            if (i >> 10) & 0x1f == 0x10 {
                let x = get(self, rn);
                match (i >> 15) & 0x3f {
                    0 => self.q[rd] = if dbl { self.q[rn] as u64 as u128 } else { self.q[rn] as u32 as u128 },
                    1 => put(self, rd, if x < 0.0 { -x } else { x }),
                    2 => put(self, rd, -x),
                    3 => put(self, rd, sqrt(x)),
                    4 => self.q[rd] = (x as f32).to_bits() as u128,
                    5 => self.q[rd] = x.to_bits() as u128,
                    8..=15 => {
                        let r = match (i >> 15) & 7 {
                            0 | 4 => round_half_away(x),
                            1 => -floor(-x),
                            2 => floor(x),
                            3 => x as i64 as f64,
                            _ => {
                                let f = floor(x);
                                if x - f > 0.5 || (x - f == 0.5 && (f as i64) & 1 == 1) { f + 1.0 } else { f }
                            }
                        };
                        put(self, rd, r);
                    }
                    _ => return undef,
                }
                return Ok(());
            }
            if (i >> 10) & 0xf == 0x8 && (i >> 14) & 3 == 0 {
                let a = get(self, rn);
                let b = if i & 8 != 0 { 0.0 } else { get(self, rm) };
                let (n, z, c, v) = if a.is_nan() || b.is_nan() {
                    (false, false, true, true)
                } else if a == b {
                    (false, true, true, false)
                } else if a < b {
                    (true, false, false, false)
                } else {
                    (false, false, true, false)
                };
                self.n = n;
                self.z = z;
                self.c = c;
                self.v = v;
                return Ok(());
            }
            if (i >> 10) & 7 == 4 {
                let imm8 = (i >> 13) & 0xff;
                let sign = if imm8 & 0x80 != 0 { -1.0 } else { 1.0 };
                let exp = (((imm8 >> 4) & 7) ^ 4) as i32 - 3; // NOT(b):b:b... decoded to an unbiased exponent
                let frac = 1.0 + (imm8 & 15) as f64 / 16.0;
                let mut v = frac;
                if exp >= 0 {
                    for _ in 0..exp {
                        v *= 2.0;
                    }
                } else {
                    for _ in 0..-exp {
                        v /= 2.0;
                    }
                }
                put(self, rd, sign * v);
                return Ok(());
            }
            if (i >> 10) & 3 == 3 {
                let v = if self.cond((i >> 12) & 15) { self.q[rn] } else { self.q[rm] };
                self.q[rd] = if dbl { v as u64 as u128 } else { v as u32 as u128 };
                return Ok(());
            }
            if (i >> 10) & 3 == 2 {
                let (a, b) = (get(self, rn), get(self, rm));
                let r = match (i >> 12) & 15 {
                    0 => a * b,
                    1 => a / b,
                    2 => a + b,
                    3 => a - b,
                    4 | 6 => if a > b || b.is_nan() { a } else { b },
                    5 | 7 => if a < b || b.is_nan() { a } else { b },
                    8 => -(a * b),
                    _ => return undef,
                };
                put(self, rd, r);
                return Ok(());
            }
            if (i >> 10) & 3 == 1 {
                // FCCMP / FCCMPE
                if self.cond((i >> 12) & 15) {
                    let (a, b) = (get(self, rn), get(self, rm));
                    self.n = a < b;
                    self.z = a == b;
                    self.c = !(a < b);
                    self.v = a.is_nan() || b.is_nan();
                } else {
                    let f = i & 15;
                    self.n = f & 8 != 0;
                    self.z = f & 4 != 0;
                    self.c = f & 2 != 0;
                    self.v = f & 1 != 0;
                }
                return Ok(());
            }
            return undef;
        }
        if i & 0x5f00_0000 == 0x1f00_0000 {
            let dbl = (i >> 22) & 3 == 1;
            let get = |s: &Self, r: usize| if dbl { f64::from_bits(s.q[r] as u64) } else { f32::from_bits(s.q[r] as u32) as f64 };
            let ra = ((i >> 10) & 31) as usize;
            let (a, n, m) = (get(self, ra), get(self, rn), get(self, rm));
            let r = match ((i >> 21) & 1, (i >> 15) & 1) {
                (0, 0) => a + n * m,
                (0, _) => a - n * m,
                (1, 0) => -a - n * m,
                _ => -a + n * m,
            };
            self.q[rd] = if dbl { r.to_bits() as u128 } else { (r as f32).to_bits() as u128 };
            return Ok(());
        }
        // SIMD moves.
        let qbit = (i >> 30) & 1 == 1;
        let mask = if qbit { u128::MAX } else { u64::MAX as u128 };
        if i & 0x9ff8_0400 == 0x0f00_0400 {
            let imm8 = (((i >> 16) & 7) << 5 | (i >> 5) & 31) as u64;
            let (op, cmode) = ((i >> 29) & 1, (i >> 12) & 15);
            let v64: u64 = match cmode {
                0 | 1 => replicate(imm8, 32),
                2 | 3 => replicate(imm8 << 8, 32),
                4 | 5 => replicate(imm8 << 16, 32),
                6 | 7 => replicate(imm8 << 24, 32),
                8 | 9 => replicate(imm8, 16),
                10 | 11 => replicate(imm8 << 8, 16),
                12 => replicate(imm8 << 8 | 0xff, 32),
                13 => replicate(imm8 << 16 | 0xffff, 32),
                14 if op == 0 => replicate(imm8, 8),
                14 => (0..8).fold(0, |acc, b| acc | if imm8 >> b & 1 == 1 { 0xffu64 << (b * 8) } else { 0 }),
                _ => return undef, // FMOV (vector, immediate)
            };
            let orr_bic = cmode < 12 && cmode & 1 == 1;
            let v = if op == 1 && cmode != 14 { !v64 } else { v64 };
            let full = (v as u128) | (v as u128) << 64;
            self.q[rd] = if orr_bic {
                if op == 0 { self.q[rd] | full } else { self.q[rd] & full }
            } else {
                full
            } & mask;
            return Ok(());
        }
        if i & 0xbfe0_fc00 == 0x0ea0_1c00 && rm == rn {
            self.q[rd] = self.q[rn] & mask;
            return Ok(());
        }
        if i & 0xbf20_fc00 == 0x0e20_1c00 {
            // AND/BIC/ORR/ORN/EOR (vector)
            let (a, b) = (self.q[rn], self.q[rm]);
            let r = match ((i >> 29) & 1, (i >> 22) & 3) {
                (0, 0) => a & b,
                (0, 1) => a & !b,
                (0, 2) => a | b,
                (0, 3) => a | !b,
                (1, 0) => a ^ b,
                _ => return undef,
            };
            self.q[rd] = r & mask;
            return Ok(());
        }
        if i & 0xbfe0_fc00 == 0x0e00_0c00 {
            let imm5 = (i >> 16) & 31;
            let esize = 8u32 << imm5.trailing_zeros().min(3);
            let v = replicate(self.xr(rn as u32) & ones(esize), esize);
            self.q[rd] = ((v as u128) | (v as u128) << 64) & mask;
            return Ok(());
        }
        if i & 0xffe0_fc00 == 0x4e00_1c00 {
            let imm5 = (i >> 16) & 31;
            let sh = imm5.trailing_zeros().min(3);
            let esize = 8u32 << sh;
            let idx = imm5 >> (sh + 1);
            let pos = idx * esize;
            let m = (ones(esize) as u128) << pos;
            self.q[rd] = (self.q[rd] & !m) | (((self.xr(rn as u32) & ones(esize)) as u128) << pos);
            return Ok(());
        }
        if i & 0xbfe0_fc00 == 0x0e00_3c00 {
            let imm5 = (i >> 16) & 31;
            let sh = imm5.trailing_zeros().min(3);
            let esize = 8u32 << sh;
            let idx = imm5 >> (sh + 1);
            let v = (self.q[rn] >> (idx * esize)) as u64 & ones(esize);
            self.set(rd as u32, v, true);
            return Ok(());
        }
        if i & 0xbf3f_fc00 == 0x0e20_5800 {
            // CNT (popcount per byte), used by clang for __builtin_popcount
            let v = self.q[rn];
            let mut r = 0u128;
            for b in 0..16 {
                r |= (((v >> (b * 8)) as u8).count_ones() as u128) << (b * 8);
            }
            self.q[rd] = r & mask;
            return Ok(());
        }
        if i & 0x9f3e_0c00 == 0x0e20_0800 && matches!((i >> 12) & 0x1f, 0b01000 | 0b01001 | 0b01010) {
            // Compare each lane with zero: CMGT/CMEQ/CMLT (U=0), CMGE/CMLE (U=1).
            let es = 8u32 << ((i >> 22) & 3);
            let lanes = if qbit { 128 / es } else { 64 / es };
            let kind = ((i >> 29) & 1, (i >> 12) & 0x1f);
            let mut r = 0u128;
            for l in 0..lanes {
                let v = sext((self.q[rn] >> (l * es)) as u64 & ones(es), es);
                let t = match kind {
                    (0, 0b01000) => v > 0,
                    (0, 0b01001) => v == 0,
                    (0, 0b01010) => v < 0,
                    (1, 0b01000) => v >= 0,
                    (1, 0b01001) => v <= 0,
                    _ => return undef,
                };
                if t {
                    r |= (ones(es) as u128) << (l * es);
                }
            }
            self.q[rd] = r;
            return Ok(());
        }
        if i & 0xffff_fc00 == 0x5ef1_b800 {
            // ADDP (scalar): Dd = Vn.D[0] + Vn.D[1]
            let v = self.q[rn];
            self.q[rd] = ((v as u64).wrapping_add((v >> 64) as u64)) as u128;
            return Ok(());
        }
        if i & 0x9f3e_0c00 == 0x0e30_0800 {
            // Across lanes: ADDV, SADDLV/UADDLV, SMAXV/UMAXV, SMINV/UMINV.
            let es = 8u32 << ((i >> 22) & 3);
            let unsigned = (i >> 29) & 1 == 1;
            let op = (i >> 12) & 0x1f;
            let lanes = if qbit { 128 / es } else { 64 / es };
            let lane = |l: u32| {
                let v = (self.q[rn] >> (l * es)) as u64 & ones(es);
                if unsigned { v as i128 } else { sext(v, es) as i128 }
            };
            let vals = (0..lanes).map(lane);
            let (r, width) = match op {
                0b11011 => (vals.sum::<i128>(), es),
                0b00011 => (vals.sum::<i128>(), es * 2),
                0b01010 => (vals.max().unwrap_or(0), es),
                0b11010 => (vals.min().unwrap_or(0), es),
                _ => return undef,
            };
            self.q[rd] = (r as u128) & (ones(width) as u128);
            return Ok(());
        }
        if i & 0xff20_fc00 == 0x5e20_8400 || i & 0xbf20_fc00 == 0x0e20_8400 {
            // ADD (vector): lanes of 8 << size bits
            let es = 8u32 << ((i >> 22) & 3);
            let (a, b) = (self.q[rn], self.q[rm]);
            let lanes = if i >> 28 == 5 { 1 } else if qbit { 128 / es } else { 64 / es };
            let mut r = 0u128;
            for l in 0..lanes {
                let m = ones(es) as u128;
                let x = ((a >> (l * es)) & m).wrapping_add((b >> (l * es)) & m) & m;
                r |= x << (l * es);
            }
            self.q[rd] = r;
            return Ok(());
        }
        undef
    }
}

/// libSystem argument access over an interpreter thread (Apple ARM64 ABI).
pub struct ArmCall<'a> {
    /// The thread.
    pub cpu: &'a mut Cpu,
    gi: usize,
    fi: usize,
    va: bool,
    stack: u64,
    /// The callee redirected control ([`Abi::jump`]).
    pub jumped: bool,
}

impl<'a> ArmCall<'a> {
    /// Arguments of a call just made by `cpu` (pc in the stub range, return address in x30).
    pub fn new(cpu: &'a mut Cpu) -> Self {
        let stack = cpu.sp;
        Self { cpu, gi: 0, fi: 0, va: false, stack, jumped: false }
    }
    fn slot(&mut self, mem: &mut UserMem) -> u64 {
        let v = mem.read_u64(self.stack).unwrap_or(0);
        self.stack += 8;
        v
    }
}

impl Args for ArmCall<'_> {
    fn int(&mut self, mem: &mut UserMem) -> u64 {
        if !self.va && self.gi < 8 {
            self.gi += 1;
            self.cpu.x[self.gi - 1]
        } else {
            self.slot(mem)
        }
    }
    fn float(&mut self, mem: &mut UserMem) -> f64 {
        if !self.va && self.fi < 8 {
            self.fi += 1;
            f64::from_bits(self.cpu.q[self.fi - 1] as u64)
        } else {
            f64::from_bits(self.slot(mem))
        }
    }
}

impl Abi for ArmCall<'_> {
    fn va_start(&mut self) {
        self.va = true;
    }
    fn ret(&mut self, v: u64) {
        self.cpu.x[0] = v;
    }
    fn set_arg(&mut self, i: usize, v: u64) {
        self.cpu.x[i] = v;
    }
    fn jump(&mut self, _mem: &mut UserMem, target: u64, ret_to: Option<u64>) -> Result<(), i64> {
        self.cpu.pc = target;
        if let Some(r) = ret_to {
            self.cpu.x[30] = r;
        }
        self.jumped = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loos_lin::mem::{PROT_EXEC, PROT_READ, PROT_WRITE};

    fn run(code: &[u32], setup: impl FnOnce(&mut Cpu)) -> (Cpu, Stop) {
        let mut mem = UserMem::new();
        let bytes: alloc::vec::Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        mem.map_fixed(0x10000, 0x1000, PROT_READ | PROT_EXEC, &bytes, "code").unwrap();
        mem.map_fixed(0x20000, 0x1000, PROT_READ | PROT_WRITE, &[], "stack").unwrap();
        let mut c = Cpu::new(0x10000, 0x21000);
        setup(&mut c);
        let s = c.run(&mut mem, 1000);
        (c, s)
    }

    #[test]
    fn integer_and_branches() {
        // mov x0,#5; mov x1,#0; loop: add x1,x1,x0; subs x0,x0,#1; b.ne loop; brk #0
        let (c, s) = run(&[0xd28000a0, 0xd2800001, 0x8b000021, 0xf1000400, 0x54ffffc1, 0xd4200000], |_| {});
        assert_eq!(s, Stop::Brk);
        assert_eq!(c.x[1], 15);
    }

    #[test]
    fn stack_pairs_and_calls() {
        // stp x29,x30,[sp,#-16]!; mov x0,#7; bl f; ldp x29,x30,[sp],#16; brk; f: lsl x0,x0,#3; ret
        let (c, s) = run(&[0xa9bf7bfd, 0xd28000e0, 0x94000003, 0xa8c17bfd, 0xd4200000, 0xd37df000, 0xd65f03c0], |c| c.x[30] = 0x1234);
        assert_eq!(s, Stop::Brk);
        assert_eq!(c.x[0], 56);
        assert_eq!(c.x[30], 0x1234);
        assert_eq!(c.sp, 0x21000);
    }

    #[test]
    fn logical_immediates_and_bitfields() {
        // mov x0,#0xff00ff00ff00ff00 (orr); ubfx x1,x0,#8,#8; sbfx x2,x0,#8,#8; and w3,w0,#0xf0f0f0f0; brk
        let (c, _) = run(&[0xb2089fe0, 0xd3483c01, 0x93483c02, 0x1204cc03, 0xd4200000], |_| {});
        assert_eq!(c.x[0], 0xff00_ff00_ff00_ff00);
        assert_eq!(c.x[1], 0xff);
        assert_eq!(c.x[2], u64::MAX);
        assert_eq!(c.x[3], 0xf000_f000);
    }

    #[test]
    fn float_math() {
        // fmov d0,#1.5; scvtf d1,x0 (x0=4); fmul d2,d0,d1; fcvtzs x3,d2; brk
        let (c, _) = run(&[0x1e6f1000, 0x9e620001, 0x1e610802, 0x9e780043, 0xd4200000], |c| c.x[0] = 4);
        assert_eq!(c.x[3], 6);
    }

    #[test]
    fn shim_and_svc_stops() {
        let (_, s) = run(&[0xd4001001], |_| {});
        assert_eq!(s, Stop::Svc(0x80));
        let (_, s) = run(&[0xd61f0200], |c| c.x[16] = SHIM_BASE + 3 * STUB_SIZE);
        assert_eq!(s, Stop::Shim(3));
    }
}
