//! kernel32 / ntdll entry points implemented by the kernel, keyed by the syscall number their stub loads.

use crate::{Process, Ret};
use loos_lin::mem::{ceil, PROT_READ, PROT_WRITE};

/// Terminates the process (also the entry point's return address).
pub const EXIT_PROCESS: u32 = 1;
/// First syscall number used for unimplemented imports.
pub const UNRESOLVED_BASE: u32 = 0x1000;

const ERROR_INVALID_PARAMETER: u32 = 87;
const STATUS_NOT_IMPLEMENTED: u64 = 0xc000_0002;
/// Pseudo handles the console APIs hand out.
const STDIN: u64 = 0x10;
const STDOUT: u64 = 0x14;
const STDERR: u64 = 0x18;
const INFINITE: u64 = 0xffff_ffff;

const TABLE: &[(&str, u32)] = &[
    ("ExitProcess", EXIT_PROCESS),
    ("RtlExitUserProcess", EXIT_PROCESS),
    ("TerminateProcess", 21),
    ("GetStdHandle", 2),
    ("WriteFile", 3),
    ("WriteConsoleA", 4),
    ("GetCommandLineA", 5),
    ("GetCurrentProcessId", 6),
    ("ProcessIdToSessionId", 7),
    ("GetTickCount", 9),
    ("GetTickCount64", 9),
    ("VirtualAlloc", 10),
    ("VirtualFree", 11),
    ("GetProcessHeap", 15),
    ("GetCurrentProcess", 19),
    ("NtTerminateProcess", 21),
];

/// Syscall number for an imported function name.
pub fn lookup(name: &str) -> Option<u32> {
    TABLE.iter().find(|(n, _)| *n == name).map(|(_, nr)| *nr).or_else(|| crate::gui::lookup(name)).or_else(|| crate::thread::lookup(name)).or_else(|| crate::ws2::lookup(name)).or_else(|| crate::gl::lookup(name)).or_else(|| crate::k32::lookup(name))
}

impl Process {
    /// Executes syscall `nr` with Windows x64 argument registers `a = [r10, rdx, r8, r9]`.
    pub fn syscall(&mut self, nr: u32, a: [u64; 4], now_ms: u64) -> Ret {
        let ok = |v: u64| Ret::Value(v);
        match nr {
            EXIT_PROCESS => Ret::Exit(a[0] as u32),
            21 => {
                // TerminateProcess(hProcess, code) / NtTerminateProcess: only "this process" (-1) is supported.
                if a[0] == u64::MAX || a[0] == 0 {
                    Ret::Exit(a[1] as u32)
                } else {
                    self.set_last_error(ERROR_INVALID_PARAMETER);
                    ok(0)
                }
            }
            2 => ok(match a[0] as u32 as i32 {
                -10 => STDIN,
                -11 => STDOUT,
                -12 => STDERR,
                _ => u64::MAX,
            }),
            3 | 4 => {
                if nr == 3 {
                    if let Some(r) = self.file_write(a) {
                        return r;
                    }
                }
                if a[0] != STDOUT && a[0] != STDERR {
                    self.set_last_error(6);
                    return ok(0);
                }
                let len = (a[2] as usize).min(1 << 16);
                match self.mem.read_vec(a[1], len) {
                    Ok(b) => {
                        self.console_write(&b);
                        if a[3] != 0 {
                            let _ = self.mem.write(a[3], &(len as u32).to_le_bytes());
                        }
                        ok(1)
                    }
                    Err(_) => {
                        self.set_last_error(998);
                        ok(0)
                    }
                }
            }
            5 => ok(self.cmdline_addr()),
            6 => ok(self.pid),
            7 => {
                let sid = self.session;
                if a[1] != 0 && self.mem.write(a[1], &sid.to_le_bytes()).is_ok() {
                    ok(1)
                } else {
                    ok(0)
                }
            }
            8 => {
                if a[0] & 0xffff_ffff == INFINITE {
                    Ret::Park
                } else {
                    ok(0)
                }
            }
            9 => ok(now_ms),
            10 => {
                // VirtualAlloc(addr, size, type, protect): anonymous, zeroed, always read/write.
                if a[1] == 0 || a[1] > (1 << 30) {
                    self.set_last_error(ERROR_INVALID_PARAMETER);
                    return ok(0);
                }
                ok(self.mem.map_anywhere(ceil(a[1]), PROT_READ | PROT_WRITE, &[], "valloc").unwrap_or(0))
            }
            11 => ok(1),
            12 => ok(u64::from(self.last_error())),
            13 => {
                self.set_last_error(a[0] as u32);
                ok(0)
            }
            14 => ok(if a[0] == 0 { self.base } else { 0 }),
            15 => ok(1),
            16 => {
                let n = a[2].max(1);
                if n > (1 << 28) {
                    return ok(0);
                }
                ok(self.mem.map_anywhere(ceil(n), PROT_READ | PROT_WRITE, &[], "heap").unwrap_or(0))
            }
            17 => ok(1),
            18 => ok(self.pid),
            19 => ok(u64::MAX),
            n if n >= UNRESOLVED_BASE => {
                let i = (n - UNRESOLVED_BASE) as usize;
                let name = self.unresolved.get(i).cloned().unwrap_or_default();
                if !self.logged.contains(&name) {
                    let msg = alloc::format!("[winrt] {}: unimplemented import {name}\n", self.name);
                    self.console_write(msg.as_bytes());
                    self.logged.push(name);
                }
                if self.soft_stubs { ok(0) } else { Ret::Exit(STATUS_NOT_IMPLEMENTED as u32) }
            }
            _ => self.thread_syscall(nr, a, self.cur_rsp, now_ms).or_else(|| self.ws_syscall(nr, a, self.cur_rsp, now_ms)).or_else(|| self.k32_syscall(nr, a, self.cur_rsp, now_ms)).unwrap_or(ok(0)),
        }
    }
}
