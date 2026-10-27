//! Bugcheck (stop) codes.

/// A kernel stop: code plus four code-specific parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BugCheck {
    /// Stop code (see the `*` constants).
    pub code: u32,
    /// Code-specific parameters.
    pub params: [u64; 4],
}

/// IRQL_NOT_LESS_OR_EQUAL.
pub const IRQL_NOT_LESS_OR_EQUAL: u32 = 0x0A;
/// KMODE_EXCEPTION_NOT_HANDLED.
pub const KMODE_EXCEPTION_NOT_HANDLED: u32 = 0x1E;
/// PAGE_FAULT_IN_NONPAGED_AREA.
pub const PAGE_FAULT_IN_NONPAGED_AREA: u32 = 0x50;
/// MANUALLY_INITIATED_CRASH.
pub const MANUALLY_INITIATED_CRASH: u32 = 0xE2;
/// SYSTEM_SERVICE_EXCEPTION.
pub const SYSTEM_SERVICE_EXCEPTION: u32 = 0x3B;
/// UNEXPECTED_KERNEL_MODE_TRAP.
pub const UNEXPECTED_KERNEL_MODE_TRAP: u32 = 0x7F;
/// KERNEL_DATA_INPAGE_ERROR.
pub const KERNEL_DATA_INPAGE_ERROR: u32 = 0x7A;
/// CRITICAL_PROCESS_DIED.
pub const CRITICAL_PROCESS_DIED: u32 = 0xEF;
/// DRIVER_IRQL_NOT_LESS_OR_EQUAL.
pub const DRIVER_IRQL_NOT_LESS_OR_EQUAL: u32 = 0xD1;
/// DPC_WATCHDOG_VIOLATION.
pub const DPC_WATCHDOG_VIOLATION: u32 = 0x133;
/// KERNEL_SECURITY_CHECK_FAILURE.
pub const KERNEL_SECURITY_CHECK_FAILURE: u32 = 0x139;
/// KERNEL_PANIC (LoOS: Rust panic in the kernel).
pub const KERNEL_PANIC: u32 = 0x1000_0001;
/// BTRFS_CORRUPTION (LoOS: unrecoverable metadata checksum failure on the system volume).
pub const BTRFS_CORRUPTION: u32 = 0x1000_0002;

const NAMES: &[(u32, &str)] = &[
    (IRQL_NOT_LESS_OR_EQUAL, "IRQL_NOT_LESS_OR_EQUAL"),
    (KMODE_EXCEPTION_NOT_HANDLED, "KMODE_EXCEPTION_NOT_HANDLED"),
    (SYSTEM_SERVICE_EXCEPTION, "SYSTEM_SERVICE_EXCEPTION"),
    (PAGE_FAULT_IN_NONPAGED_AREA, "PAGE_FAULT_IN_NONPAGED_AREA"),
    (UNEXPECTED_KERNEL_MODE_TRAP, "UNEXPECTED_KERNEL_MODE_TRAP"),
    (KERNEL_DATA_INPAGE_ERROR, "KERNEL_DATA_INPAGE_ERROR"),
    (DRIVER_IRQL_NOT_LESS_OR_EQUAL, "DRIVER_IRQL_NOT_LESS_OR_EQUAL"),
    (MANUALLY_INITIATED_CRASH, "MANUALLY_INITIATED_CRASH"),
    (CRITICAL_PROCESS_DIED, "CRITICAL_PROCESS_DIED"),
    (DPC_WATCHDOG_VIOLATION, "DPC_WATCHDOG_VIOLATION"),
    (KERNEL_SECURITY_CHECK_FAILURE, "KERNEL_SECURITY_CHECK_FAILURE"),
    (KERNEL_PANIC, "KERNEL_PANIC"),
    (BTRFS_CORRUPTION, "BTRFS_CORRUPTION"),
];

/// Symbolic name of a stop code, or `"UNKNOWN_BUGCHECK"`.
pub fn name(code: u32) -> &'static str {
    NAMES.iter().find(|(c, _)| *c == code).map(|(_, n)| *n).unwrap_or("UNKNOWN_BUGCHECK")
}

impl BugCheck {
    /// Build a bugcheck.
    pub const fn new(code: u32, params: [u64; 4]) -> Self {
        Self { code, params }
    }

    /// Symbolic name.
    pub fn name(&self) -> &'static str {
        name(self.code)
    }

    /// Short description of what `params[i]` means for this code.
    pub fn param_meaning(&self, i: usize) -> &'static str {
        match (self.code, i) {
            (IRQL_NOT_LESS_OR_EQUAL | DRIVER_IRQL_NOT_LESS_OR_EQUAL, 0) => "address referenced",
            (IRQL_NOT_LESS_OR_EQUAL | DRIVER_IRQL_NOT_LESS_OR_EQUAL, 1) => "IRQL at time of fault",
            (IRQL_NOT_LESS_OR_EQUAL | DRIVER_IRQL_NOT_LESS_OR_EQUAL, 2) => "0=read 1=write",
            (IRQL_NOT_LESS_OR_EQUAL | DRIVER_IRQL_NOT_LESS_OR_EQUAL, 3) => "instruction address",
            (PAGE_FAULT_IN_NONPAGED_AREA, 0) => "address referenced",
            (PAGE_FAULT_IN_NONPAGED_AREA, 1) => "0=read 1=write",
            (PAGE_FAULT_IN_NONPAGED_AREA, 2) => "instruction address",
            (KMODE_EXCEPTION_NOT_HANDLED, 0) => "exception code",
            (KMODE_EXCEPTION_NOT_HANDLED, 1) => "exception address",
            (UNEXPECTED_KERNEL_MODE_TRAP, 0) => "trap vector",
            (CRITICAL_PROCESS_DIED, 0) => "process object",
            (KERNEL_SECURITY_CHECK_FAILURE, 0) => "failure type",
            (_, _) => "",
        }
    }
}
