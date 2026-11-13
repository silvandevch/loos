//! Darwin system calls, translated onto the LoOS Linux-layer syscalls.
//!
//! x86-64: `rax = class << 24 | number` (class 2 BSD, 1 Mach trap, 3 machine-dependent),
//! arguments in rdi, rsi, rdx, r10, r8, r9; ARM64: number in x16 (negative = Mach trap),
//! `svc #0x80`. Errors set the carry flag and return a positive Darwin errno.

use crate::libsys::Post;

/// How a Darwin syscall is carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Run this LoOS (Linux-numbered) syscall.
    Linux(u64, [u64; 6], Post),
    /// Succeed with this value without a LoOS call.
    Value(u64),
    /// Fail with this Linux errno.
    Error(i64),
}

/// Map a Linux errno to Darwin's numbering (1-34 are identical).
pub fn linux_to_darwin_errno(e: i64) -> i64 {
    match e {
        11 => 35,
        35 => 11,
        36 => 63,
        38 => 78,
        39 => 66,
        40 => 62,
        61 => 96,
        62 => 101,
        71 => 100,
        75 => 84,
        84 => 92,
        88 => 38,
        90 => 40,
        95 => 45,
        97 => 47,
        98 => 48,
        99 => 49,
        100 => 50,
        101 => 51,
        103 => 53,
        104 => 54,
        105 => 55,
        106 => 56,
        107 => 57,
        110 => 60,
        111 => 61,
        113 => 65,
        114 => 37,
        115 => 36,
        125 => 89,
        e => e,
    }
}

/// Darwin `open` flags → Linux.
pub fn open_flags(f: u64) -> u64 {
    let mut l = f & 3; // O_RDONLY/O_WRONLY/O_RDWR
    for (d, lx) in [(0x4, 0x800), (0x8, 0x400), (0x200, 0x40), (0x400, 0x200), (0x800, 0x80), (0x2_0000, 0x100), (0x10_0000, 0x1_0000), (0x100_0000, 0x8_0000), (0x80, 0x10_1000), (0x100, 0x2_0000)] {
        if f & d != 0 {
            l |= lx;
        }
    }
    l
}

pub fn mmap_flags(f: u64) -> u64 {
    // MAP_SHARED 1, MAP_PRIVATE 2, MAP_FIXED 0x10 are shared; MAP_ANON 0x1000 → 0x20; MAP_JIT/NORESERVE dropped.
    (f & 0x13) | if f & 0x1000 != 0 { 0x20 } else { 0 }
}

/// Darwin `AT_FDCWD` (-2) → Linux (-100).
pub fn dirfd(fd: u64) -> u64 {
    if fd as i64 as i32 == -2 {
        (-100i64) as u64
    } else {
        fd
    }
}

/// Scratch offset where a translated `stat` is staged.
pub const STAT_STAGE: u64 = crate::loader::SCRATCH + 0x8000;

/// Translate BSD syscall `n`.
pub fn bsd(n: u64, a: [u64; 6]) -> Plan {
    let lin = |nr: u64, args: [u64; 6]| Plan::Linux(nr, args, Post::Darwin);
    let [a0, a1, a2, a3, a4, a5] = a;
    match n {
        1 => lin(231, [a0, 0, 0, 0, 0, 0]),
        3 | 396 => lin(0, a),
        4 | 397 => lin(1, a),
        5 | 398 => lin(2, [a0, open_flags(a1), a2, 0, 0, 0]),
        6 | 399 => lin(3, a),
        9 => lin(86, a),
        10 => lin(87, a),
        12 => lin(80, a),
        13 => lin(81, a),
        15 => lin(90, a),
        20 => lin(39, a),
        24 | 25 => Plan::Value(501),
        43 | 47 => Plan::Value(20),
        33 => lin(21, a),
        37 => lin(62, a),
        39 => lin(110, a),
        41 => lin(32, a),
        42 => Plan::Linux(22, [crate::loader::SCRATCH, 0, 0, 0, 0, 0], Post::DarwinPipe),
        // ioctl: TIOCGETA → TCGETS (staged; the layouts differ, callers here only test for a terminal)
        54 if a1 == 0x4048_7413 => lin(16, [a0, 0x5401, STAT_STAGE, 0, 0, 0]),
        54 => lin(16, a),
        57 => lin(88, a),
        58 => lin(89, a),
        60 => lin(95, a),
        73 => lin(11, a),
        74 => lin(10, a),
        75 => Plan::Value(0), // madvise: advice only
        90 => lin(33, a),
        92 | 406 => lin(72, a),
        95 | 408 => lin(74, a),
        116 => lin(96, a),
        128 => lin(82, a),
        136 => lin(83, a),
        137 => lin(84, a),
        153 | 414 => lin(17, a),
        154 | 415 => lin(18, a),
        194 => lin(97, a),
        195 => lin(160, a),
        197 => lin(9, [a0, a1, a2, mmap_flags(a3), a4, a5]),
        199 => lin(8, a),
        200 => lin(76, a),
        201 => lin(77, a),
        338 => Plan::Linux(4, [a0, STAT_STAGE, 0, 0, 0, 0], Post::DarwinStat(a1)),
        339 => Plan::Linux(5, [a0, STAT_STAGE, 0, 0, 0, 0], Post::DarwinStat(a1)),
        340 => Plan::Linux(6, [a0, STAT_STAGE, 0, 0, 0, 0], Post::DarwinStat(a1)),
        463 => lin(257, [dirfd(a0), a1, open_flags(a2), a3, 0, 0]),
        _ => Plan::Error(38),
    }
}

/// Mach traps (x86-64 class 1 / ARM64 negative x16). The task, thread and host ports are fixed names.
pub fn mach(n: u64, _a: [u64; 6]) -> Plan {
    use crate::libsys::TASK_PORT;
    match n {
        26 => Plan::Value(TASK_PORT + 12), // mach_reply_port
        27 => Plan::Value(TASK_PORT + 4),  // thread_self_trap
        28 => Plan::Value(TASK_PORT),      // task_self_trap
        29 => Plan::Value(TASK_PORT + 8),  // host_self_trap
        // mach_msg: no Mach servers exist yet; report the destination as invalid.
        31 | 32 => Plan::Value(0x1000_0003),
        _ => Plan::Value(0x10000003),
    }
}

/// Copy a Linux `struct stat` (144 bytes) into Darwin's `struct stat64` layout.
pub fn stat_to_darwin(l: &[u8; 144]) -> [u8; 144] {
    let u64at = |o: usize| u64::from_le_bytes(l[o..o + 8].try_into().unwrap());
    let u32at = |o: usize| u32::from_le_bytes(l[o..o + 4].try_into().unwrap());
    let mut d = [0u8; 144];
    let mut put = |o: usize, b: &[u8]| d[o..o + b.len()].copy_from_slice(b);
    put(0, &(u64at(0) as u32).to_le_bytes()); // st_dev
    put(4, &(u32at(24) as u16).to_le_bytes()); // st_mode
    put(6, &(u64at(16) as u16).to_le_bytes()); // st_nlink
    put(8, &u64at(8).to_le_bytes()); // st_ino
    put(16, &u32at(28).to_le_bytes()); // st_uid
    put(20, &u32at(32).to_le_bytes()); // st_gid
    put(24, &(u64at(40) as u32).to_le_bytes()); // st_rdev
    put(32, &l[72..88]); // st_atimespec
    put(48, &l[88..104]); // st_mtimespec
    put(64, &l[104..120]); // st_ctimespec
    put(80, &l[104..120]); // st_birthtimespec
    put(96, &u64at(48).to_le_bytes()); // st_size
    put(104, &u64at(64).to_le_bytes()); // st_blocks
    put(112, &(u64at(56) as u32).to_le_bytes()); // st_blksize
    d
}
