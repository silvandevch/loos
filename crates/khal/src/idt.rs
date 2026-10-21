//! Interrupt descriptor table.

/// Gate types.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GateType {
    /// Clears IF on entry.
    Interrupt = 0xe,
    /// Leaves IF unchanged.
    Trap = 0xf,
}

/// One 16-byte long-mode gate.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Gate {
    off_lo: u16,
    selector: u16,
    ist: u8,
    attr: u8,
    off_mid: u16,
    off_hi: u32,
    _zero: u32,
}

impl Gate {
    /// A not-present gate.
    pub const MISSING: Gate = Gate { off_lo: 0, selector: 0, ist: 0, attr: 0, off_mid: 0, off_hi: 0, _zero: 0 };

    /// Builds a present gate. `ist` is 0 (no stack switch) or 1..=7; `dpl` 3 lets user mode use `int n`.
    pub const fn new(handler: u64, selector: u16, ty: GateType, dpl: u8, ist: u8) -> Gate {
        Gate {
            off_lo: handler as u16,
            selector,
            ist: ist & 7,
            attr: 0x80 | (dpl & 3) << 5 | ty as u8,
            off_mid: (handler >> 16) as u16,
            off_hi: (handler >> 32) as u32,
            _zero: 0,
        }
    }

    /// Handler address.
    pub const fn handler(&self) -> u64 {
        self.off_lo as u64 | (self.off_mid as u64) << 16 | (self.off_hi as u64) << 32
    }

    /// Whether the present bit is set.
    pub const fn present(&self) -> bool {
        self.attr & 0x80 != 0
    }

    /// Descriptor privilege level.
    pub const fn dpl(&self) -> u8 {
        (self.attr >> 5) & 3
    }

    /// IST slot.
    pub const fn ist(&self) -> u8 {
        self.ist
    }
}

/// All 256 vectors.
#[repr(C, align(16))]
pub struct Idt {
    gates: [Gate; 256],
}

impl Idt {
    /// Every vector not present.
    pub const fn new() -> Self {
        Self { gates: [Gate::MISSING; 256] }
    }

    /// Installs a gate.
    pub fn set(&mut self, vector: u8, gate: Gate) {
        self.gates[vector as usize] = gate;
    }

    /// Reads a gate.
    pub fn get(&self, vector: u8) -> Gate {
        self.gates[vector as usize]
    }

    /// Table size for `lidt` (limit field).
    pub const fn limit() -> u16 {
        (256 * core::mem::size_of::<Gate>() - 1) as u16
    }
}

impl Default for Idt {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the CPU pushes an error code for this exception vector.
pub const fn has_error_code(vector: u8) -> bool {
    matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17 | 21 | 29 | 30)
}

#[allow(missing_docs)]
/// Saved register frame built by the entry stubs (`isr.S`), lowest address first.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TrapFrame {
    /// General-purpose registers in push order reversed (r15 first).
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64, pub r11: u64, pub r10: u64, pub r9: u64, pub r8: u64,
    /// More GPRs.
    pub rbp: u64, pub rdi: u64, pub rsi: u64, pub rdx: u64, pub rcx: u64, pub rbx: u64, pub rax: u64,
    /// Vector number pushed by the stub.
    pub vector: u64,
    /// Hardware or synthesised (0) error code.
    pub error: u64,
    /// Hardware frame.
    pub rip: u64, pub cs: u64, pub rflags: u64, pub rsp: u64, pub ss: u64,
}
