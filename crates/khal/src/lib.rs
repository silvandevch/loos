//! x86-64 HAL.
//!
//! Everything that can be expressed as data (descriptor encodings, APIC register maths, MSR numbers)
//! is plain safe code unit-tested on the host. Privileged instructions live in [`cpu`], the only
//! module with `unsafe`, and only exist when building for the bare-metal target.
#![no_std]

pub mod apic;
pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod msr;
#[cfg(target_arch = "x86_64")]
pub mod switch;

/// Exception / interrupt vector layout shared by the IDT, the APIC and the scheduler.
pub mod vectors {
    /// First vector available for external interrupts (0..32 are CPU exceptions).
    pub const IRQ_BASE: u8 = 32;
    /// Local APIC timer (scheduler tick).
    pub const TIMER: u8 = 0xec;
    /// Inter-processor interrupt: reschedule.
    pub const IPI_RESCHEDULE: u8 = 0xfd;
    /// Inter-processor interrupt: TLB shootdown.
    pub const IPI_TLB: u8 = 0xfe;
    /// Spurious interrupt vector.
    pub const SPURIOUS: u8 = 0xff;
}
