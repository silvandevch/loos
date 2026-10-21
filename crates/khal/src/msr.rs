//! Model-specific register numbers and bit definitions.

/// `IA32_APIC_BASE`.
pub const APIC_BASE: u32 = 0x1b;
/// `IA32_EFER`.
pub const EFER: u32 = 0xc000_0080;
/// `IA32_STAR`: syscall segment selectors.
pub const STAR: u32 = 0xc000_0081;
/// `IA32_LSTAR`: 64-bit syscall entry.
pub const LSTAR: u32 = 0xc000_0082;
/// `IA32_FMASK`: RFLAGS bits cleared on syscall.
pub const FMASK: u32 = 0xc000_0084;
/// `IA32_FS_BASE`.
pub const FS_BASE: u32 = 0xc000_0100;
/// `IA32_GS_BASE`.
pub const GS_BASE: u32 = 0xc000_0101;
/// `IA32_KERNEL_GS_BASE` (swapgs).
pub const KERNEL_GS_BASE: u32 = 0xc000_0102;

/// EFER.SCE – syscall enable.
pub const EFER_SCE: u64 = 1 << 0;
/// EFER.LME – long mode enable.
pub const EFER_LME: u64 = 1 << 8;
/// EFER.NXE – no-execute enable.
pub const EFER_NXE: u64 = 1 << 11;

/// APIC_BASE.enable.
pub const APIC_BASE_ENABLE: u64 = 1 << 11;
/// APIC_BASE.x2APIC enable.
pub const APIC_BASE_X2APIC: u64 = 1 << 10;
/// APIC_BASE.bootstrap processor flag.
pub const APIC_BASE_BSP: u64 = 1 << 8;

/// Value for `IA32_STAR`: kernel CS selector in bits 47:32, and the selector base used for sysret in 63:48.
/// `sysret` loads CS = base+16 and SS = base+8 for 64-bit, so `user_base` is the *user data selector - 8*.
pub const fn star(kernel_cs: u16, user_base: u16) -> u64 {
    (kernel_cs as u64) << 32 | (user_base as u64) << 48
}

/// Physical APIC base address from an `IA32_APIC_BASE` value.
pub const fn apic_phys(v: u64) -> u64 {
    v & 0x000f_ffff_ffff_f000
}
