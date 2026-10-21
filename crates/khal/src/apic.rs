//! Local APIC (xAPIC MMIO and x2APIC MSR forms) register maths.

/// Register offsets in the xAPIC page; x2APIC MSR = `0x800 + (offset >> 4)`.
pub mod reg {
    /// Local APIC ID.
    pub const ID: u32 = 0x20;
    /// End of interrupt.
    pub const EOI: u32 = 0xb0;
    /// Spurious interrupt vector register.
    pub const SVR: u32 = 0xf0;
    /// Interrupt command, low dword.
    pub const ICR_LO: u32 = 0x300;
    /// Interrupt command, high dword (xAPIC only; destination).
    pub const ICR_HI: u32 = 0x310;
    /// Timer local vector table entry.
    pub const LVT_TIMER: u32 = 0x320;
    /// Timer initial count.
    pub const TIMER_INIT: u32 = 0x380;
    /// Timer current count.
    pub const TIMER_CUR: u32 = 0x390;
    /// Timer divide configuration.
    pub const TIMER_DIV: u32 = 0x3e0;
}

/// MSR number for an x2APIC register.
pub const fn x2apic_msr(offset: u32) -> u32 {
    0x800 + (offset >> 4)
}

/// SVR value enabling the APIC with the given spurious vector.
pub const fn svr_enable(spurious: u8) -> u32 {
    0x100 | spurious as u32
}

/// Timer modes (LVT bits 18:17).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimerMode {
    /// One-shot countdown.
    OneShot = 0,
    /// Periodic reload.
    Periodic = 1,
    /// Fires when TSC reaches the deadline MSR.
    TscDeadline = 2,
}

/// LVT timer entry.
pub const fn lvt_timer(vector: u8, mode: TimerMode, masked: bool) -> u32 {
    vector as u32 | (mode as u32) << 17 | (masked as u32) << 16
}

/// Divide-configuration encoding for a power-of-two divisor 1..=128.
pub const fn divide_config(div: u32) -> Option<u32> {
    match div {
        1 => Some(0b1011),
        2 => Some(0b0000),
        4 => Some(0b0001),
        8 => Some(0b0010),
        16 => Some(0b0011),
        32 => Some(0b1000),
        64 => Some(0b1001),
        128 => Some(0b1010),
        _ => None,
    }
}

/// Initial count for a timer running at `bus_hz / div` to tick at `hz`.
pub const fn timer_count(bus_hz: u64, div: u32, hz: u32) -> Option<u32> {
    if hz == 0 || div == 0 {
        return None;
    }
    let c = bus_hz / div as u64 / hz as u64;
    if c == 0 || c > u32::MAX as u64 {
        None
    } else {
        Some(c as u32)
    }
}

/// Bus frequency from a calibration run: the timer (div `div`) counted down by `elapsed` while
/// `ms` milliseconds passed on a reference clock.
pub const fn calibrate_bus_hz(elapsed: u32, div: u32, ms: u32) -> u64 {
    elapsed as u64 * div as u64 * 1000 / ms as u64
}

/// IPI delivery modes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Delivery {
    /// Fixed vector.
    Fixed = 0,
    /// Non-maskable interrupt.
    Nmi = 4,
    /// INIT (AP startup).
    Init = 5,
    /// STARTUP (AP startup, vector = page number of the real-mode entry).
    Startup = 6,
}

/// IPI destination shorthand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dest {
    /// A single APIC id.
    Id(u32),
    /// This CPU.
    SelfOnly,
    /// Every CPU including this one.
    All,
    /// Every CPU except this one.
    AllButSelf,
}

/// Encodes an ICR value (64-bit, x2APIC layout: destination in bits 63:32).
pub const fn icr(vector: u8, delivery: Delivery, dest: Dest, assert: bool) -> u64 {
    let (shorthand, id) = match dest {
        Dest::Id(i) => (0u64, i as u64),
        Dest::SelfOnly => (1, 0),
        Dest::All => (2, 0),
        Dest::AllButSelf => (3, 0),
    };
    vector as u64 | (delivery as u64) << 8 | (assert as u64) << 14 | shorthand << 18 | id << 32
}

/// Splits an x2APIC-layout ICR into the xAPIC (hi, lo) dwords: destination moves to bits 31:24 of `hi`.
pub const fn icr_xapic(v: u64) -> (u32, u32) {
    (((v >> 32) as u32 & 0xff) << 24, v as u32)
}
