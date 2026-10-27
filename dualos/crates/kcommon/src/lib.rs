//! Code shared by both kernels: serial logging, QEMU exit, CPUID feature probe.
#![no_std]

use core::arch::asm;
use core::fmt;

#[inline]
pub unsafe fn outb(port: u16, v: u8) {
    asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags));
}
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack, preserves_flags));
    v
}

pub mod serial {
    use super::*;
    const COM1: u16 = 0x3F8;

    pub fn init() {
        unsafe {
            outb(COM1 + 1, 0x00); // no interrupts
            outb(COM1 + 3, 0x80); // DLAB on
            outb(COM1, 0x01); // 115200 baud
            outb(COM1 + 1, 0x00);
            outb(COM1 + 3, 0x03); // 8N1
            outb(COM1 + 2, 0xC7); // FIFO on
            outb(COM1 + 4, 0x03);
        }
    }
    pub struct Com1;
    impl fmt::Write for Com1 {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            for b in s.bytes() {
                unsafe {
                    while inb(COM1 + 5) & 0x20 == 0 {}
                    outb(COM1, b);
                }
            }
            Ok(())
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($a:tt)*) => {{
        use core::fmt::Write;
        let _ = writeln!($crate::serial::Com1, $($a)*);
    }};
}

/// Exit QEMU via isa-debug-exit (iobase 0xf4); status = (code << 1) | 1. No-op on real hardware.
pub fn qemu_exit(code: u8) -> ! {
    unsafe { outb(0xF4, code) };
    halt()
}

pub fn halt() -> ! {
    loop {
        unsafe { asm!("cli; hlt", options(nomem, nostack)) };
    }
}

pub mod cpu {
    #[cfg(target_arch = "x86")]
    use core::arch::asm;

    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::__cpuid;
    #[cfg(target_arch = "x86")]
    use core::arch::x86::__cpuid;

    /// CPUID exists iff EFLAGS.ID (bit 21) can be toggled. Always true on x86_64.
    pub fn has_cpuid() -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            true
        }
        #[cfg(target_arch = "x86")]
        unsafe {
            let (a, b): (u32, u32);
            asm!(
                "pushfd", "pop {a}", "mov {b}, {a}", "xor {b}, 0x200000",
                "push {b}", "popfd", "pushfd", "pop {b}", "push {a}", "popfd",
                a = out(reg) a, b = out(reg) b,
            );
            (a ^ b) & 0x200000 != 0
        }
    }

    pub struct Features {
        pub cpuid: bool,
        pub vendor: [u8; 12],
        pub max_leaf: u32,
        pub edx1: u32,
        pub long_mode: bool,
    }

    pub fn probe() -> Features {
        let mut f = Features { cpuid: has_cpuid(), vendor: *b"unknown     ", max_leaf: 0, edx1: 0, long_mode: false };
        if !f.cpuid {
            return f;
        }
        let l0 = __cpuid(0);
        f.max_leaf = l0.eax;
        f.vendor[0..4].copy_from_slice(&l0.ebx.to_le_bytes());
        f.vendor[4..8].copy_from_slice(&l0.edx.to_le_bytes());
        f.vendor[8..12].copy_from_slice(&l0.ecx.to_le_bytes());
        if f.max_leaf >= 1 {
            f.edx1 = __cpuid(1).edx;
        }
        if __cpuid(0x8000_0000).eax >= 0x8000_0001 {
            f.long_mode = __cpuid(0x8000_0001).edx & (1 << 29) != 0;
        }
        f
    }

    /// Print the boot banner the QEMU tests grep for.
    pub fn banner(kernel: &str, mode: &str, arch: &str, f: &Features) {
        let v = core::str::from_utf8(&f.vendor).unwrap_or("?");
        crate::log!(
            "dualos {kernel}: mode={mode} arch={arch} cpuid={} lm={} vendor='{v}' maxleaf={} cx8={} sse2={}",
            f.cpuid as u8, f.long_mode as u8, f.max_leaf, (f.edx1 >> 8) & 1, (f.edx1 >> 26) & 1
        );
    }
}
