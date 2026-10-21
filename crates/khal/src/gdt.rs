//! GDT and TSS encoding for long mode.

/// Null descriptor.
pub const NULL: u64 = 0;

/// Selector indices in the table built by [`Gdt::new`].
pub mod sel {
    /// Kernel code (ring 0).
    pub const KERNEL_CS: u16 = 0x08;
    /// Kernel data.
    pub const KERNEL_DS: u16 = 0x10;
    /// User data (ring 3) – placed before user code so `sysret` works with STAR.
    pub const USER_DS: u16 = 0x18 | 3;
    /// User code (ring 3).
    pub const USER_CS: u16 = 0x20 | 3;
    /// TSS (two slots).
    pub const TSS: u16 = 0x28;
}

const fn seg(access: u8, flags: u8) -> u64 {
    // base = 0, limit = 0xfffff; long mode ignores both for code/data.
    0xffff | (access as u64) << 40 | (0xf_u64) << 48 | (flags as u64) << 52
}

/// Long-mode code descriptor for ring `dpl`.
pub const fn code(dpl: u8) -> u64 {
    seg(0x9a | (dpl << 5), 0b1010)            // present, code, readable; G=1, L=1
}

/// Data descriptor for ring `dpl`.
pub const fn data(dpl: u8) -> u64 {
    seg(0x92 | (dpl << 5), 0b1100)          // present, writable; G=1, D/B=1
}

/// 64-bit Task State Segment.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Tss {
    _r0: u32,
    /// Stack pointers for privilege levels 0..2.
    pub rsp: [u64; 3],
    _r1: u64,
    /// Interrupt stack table entries 1..7.
    pub ist: [u64; 7],
    _r2: u64,
    _r3: u16,
    /// Offset of the I/O permission bitmap; `size_of::<Tss>()` means "none".
    pub iomap_base: u16,
}

impl Tss {
    /// A TSS with no I/O bitmap and all stacks zero.
    pub const fn new() -> Self {
        Self { _r0: 0, rsp: [0; 3], _r1: 0, ist: [0; 7], _r2: 0, _r3: 0, iomap_base: core::mem::size_of::<Self>() as u16 }
    }
}

impl Default for Tss {
    fn default() -> Self {
        Self::new()
    }
}

/// The two descriptor words of an available 64-bit TSS at `base` with `limit`.
pub const fn tss_descriptor(base: u64, limit: u32) -> [u64; 2] {
    let lo = (limit as u64 & 0xffff)
        | (base & 0xffff) << 16
        | ((base >> 16) & 0xff) << 32
        | 0x89u64 << 40
        | ((limit as u64 >> 16) & 0xf) << 48
        | ((base >> 24) & 0xff) << 56;
    [lo, base >> 32]
}

/// A complete GDT: null, kernel code/data, user data/code, TSS.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct Gdt {
    /// Raw descriptor words (7 slots, TSS uses the last two).
    pub entries: [u64; 7],
}

impl Gdt {
    /// Builds the table for a TSS located at `tss_base`.
    pub const fn new(tss_base: u64) -> Self {
        let t = tss_descriptor(tss_base, core::mem::size_of::<Tss>() as u32 - 1);
        Self { entries: [NULL, code(0), data(0), data(3), code(3), t[0], t[1]] }
    }
}

/// Operand of `lgdt`.
#[repr(C, packed)]
pub struct DescriptorPointer {
    /// Table size minus one.
    pub limit: u16,
    /// Linear address.
    pub base: u64,
}
