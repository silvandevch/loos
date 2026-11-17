//! Windows process runtime: loads x64 PE images into a [`loos_lin::mem::UserMem`] address space, binds
//! their imports to tiny `syscall` stubs, and implements the kernel32/ntdll calls those stubs invoke.
//!
//! No host or hardware access here: the kernel (`crates/kffi/src/win.rs`) builds page tables from the
//! address space, enters ring 3, and forwards each `syscall` to [`Process::syscall`]. Scope: console
//! subsystem programs that import kernel32/ntdll only (no CRT DLLs, no threads, no relocations -
//! images load at their preferred base).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod adv;
pub mod api;
pub mod com;
pub mod dwrite;
pub mod ttf;
pub mod cio;
pub mod crt;
pub mod dib;
pub mod dll;
pub mod files;
pub mod fmt;
pub mod gl;
pub mod gui;
pub mod k32;
pub mod pe;
pub mod rsrc;
pub mod seh;
pub mod thread;
pub mod ws2;

use alloc::string::String;
use alloc::vec::Vec;
use loos_lin::mem::{UserMem, PROT_READ, PROT_WRITE};

/// Why a process could not be created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeError {
    /// Truncated or inconsistent headers.
    Malformed(&'static str),
    /// Not a 64-bit x86-64 PE.
    Unsupported(&'static str),
    /// Address space exhausted or base address unusable.
    NoMemory,
}

/// What a syscall asks the kernel to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ret {
    /// Resume the process with this value in `rax`.
    Value(u64),
    /// Resume with this `FLOAT` (bits) in `xmm0`: methods whose return type is floating point.
    Float(u32),
    /// Process ended with this exit code.
    Exit(u32),
    /// Process went to sleep forever (a resident service): stop running it.
    Park,
    /// GUI thread is blocked in `GetMessage` with an empty queue.
    Idle,
    /// Run the queued `DllMain` calls (see [`Process::next_init`]), then resume with this value in `rax`.
    Init(u64),
    /// The call cannot finish yet: another thread should run and the call is retried later.
    Yield,
    /// The calling thread ended with this exit code.
    ThreadExit(u32),
    /// `RaiseException`: dispatch a software exception to the guest's SEH handlers.
    Raise {
        /// Exception code.
        code: u32,
        /// Exception flags.
        flags: u32,
        /// Number of parameters.
        nargs: u32,
        /// Guest pointer to the parameters.
        args: u64,
    },
    /// `RtlUnwindEx`: unwind to `frame` and continue at `ip` with `ret` in `rax`.
    Unwind {
        /// Target establisher frame (0: unwind everything).
        frame: u64,
        /// Continuation address.
        ip: u64,
        /// Exception record.
        rec: u64,
        /// Value for `rax`.
        ret: u64,
    },
}

/// One Windows process.
pub struct Process {
    /// Process id.
    pub pid: u64,
    /// Session id.
    pub session: u32,
    /// Address space.
    pub mem: UserMem,
    /// Entry point (virtual address).
    pub entry: u64,
    /// Initial stack pointer.
    pub rsp: u64,
    /// Image base.
    pub base: u64,
    /// Image file name.
    pub name: String,
    /// Bytes written to stdout/stderr since the last [`Process::take_console`].
    console: Vec<u8>,
    /// Typed console input not yet read (`ReadFile`/`ReadConsole` on the stdin handle).
    stdin: Vec<u8>,
    /// A console window feeds stdin: reads without a full line block instead of returning end of file.
    pub console_host: bool,
    /// Imports from DLL files not bound yet (see [`Process::link_imports`]).
    deferred: Vec<pe::Deferred>,
    /// DirectWrite fonts and COM objects.
    pub(crate) dw: dwrite::State,
    /// Names given to `RegisterWindowMessageW`, by message number - 0xC000.
    pub(crate) gui_atoms: Vec<String>,
    /// Title set by `SetConsoleTitleW`.
    pub console_title: String,
    cmdline_addr: u64,
    /// Stub whose `syscall` ends a window-procedure callback.
    pub cb_stub: u64,
    /// Unimplemented imports return 0 (logged once) instead of terminating the process.
    pub soft_stubs: bool,
    /// Log every user32/gdi32 call.
    pub trace: bool,
    /// Threads (index 0 is the initial thread) and the one currently executing.
    pub threads: Vec<thread::Thread>,
    /// Index of the executing thread (set by the kernel before each syscall).
    pub cur: usize,
    /// User stack pointer at the current syscall (stack arguments live above it).
    pub cur_rsp: u64,
    /// General registers at the current syscall (kernel-provided).
    pub regs: [u64; 16],
    /// Return address of the current syscall's stub call.
    pub ret_addr: u64,
    /// Images with unwind tables (the executable first).
    pub images: Vec<seh::Image>,
    /// `SetUnhandledExceptionFilter` callback.
    pub unhandled_filter: u64,
    /// XMM0-3 at the current syscall (floating-point arguments).
    pub xmm: [[u8; 16]; 4],
    pub(crate) gls: Vec<Option<gl::Gl>>,
    pub(crate) gl_cur: Option<usize>,
    pub(crate) net: Option<alloc::boxed::Box<dyn ws2::NetOps>>,
    pub(crate) socks: Vec<Option<ws2::Sock>>,
    pub(crate) ws_err: i32,
    pub(crate) ws_init: u32,
    pub(crate) adv: adv::State,
    /// Modal dialog loops in progress: `(call key, dialog, owner)`.
    /// Resource data sizes by address (`FindResource` handles).
    pub(crate) rsrc: alloc::collections::BTreeMap<u64, u32>,
    /// Check-box/radio states set by `CheckDlgButton`.
    pub(crate) checks: alloc::collections::BTreeMap<u32, u32>,
    pub(crate) crt: crt::State,
    pub(crate) data: crt::DataArena,
    pub(crate) modal: Vec<([u64; 4], u32, u32)>,
    /// DIB sections: bitmap handle -> `(bits address, header bytes)`.
    pub(crate) dibs: alloc::collections::BTreeMap<u32, (u64, Vec<u8>)>,
    pub(crate) dns: Option<ws2::Dns>,
    pub(crate) dns_seq: u16,
    pub(crate) scratch: u64,
    pub(crate) scratch_next: u64,
    pub(crate) spawned: Vec<usize>,
    pub(crate) objs: Vec<Option<thread::Obj>>,
    pub(crate) tls_count: u32,
    /// Stub that ends a thread when its start routine returns.
    pub thread_exit_stub: u64,
    /// Guest volume: normalised path -> contents.
    pub vfs: alloc::collections::BTreeMap<String, Vec<u8>>,
    pub(crate) open: Vec<Option<files::OpenFile>>,
    pub(crate) finds: Vec<Option<(Vec<String>, usize)>>,
    /// DLLs mapped at run time.
    pub modules: Vec<dll::Module>,
    pub(crate) next_dll_base: u64,
    pub(crate) pending_init: Vec<(u64, u64)>,
    pub(crate) stub_cache: alloc::collections::BTreeMap<u32, u64>,
    pub(crate) stub_next: u64,
    pub(crate) stub_end: u64,
    /// File name the open/save common dialogs answer with (headless policy; empty = the dialog is cancelled).
    pub dialog_open: String,
    /// Same for the save dialog.
    pub dialog_save: String,
    /// Menus built through CreateMenu/AppendMenu.
    pub(crate) menus: alloc::collections::BTreeMap<u64, Vec<(u32, u64, String)>>,
    /// Thread environment block (user GS base).
    pub teb: u64,
    pub(crate) heap_base: u64,
    pub(crate) heap_next: u64,
    pub(crate) tls: Vec<u64>,
    pub(crate) cmdline_w: u64,
    pub(crate) env_w: u64,
    logged: Vec<String>,
    /// Imports that had no implementation (each stub reports its index through the syscall number).
    pub unresolved: Vec<String>,
}

impl Process {
    /// Loads `image` (PE file bytes) and prepares its stack and command line.
    pub fn create(pid: u64, session: u32, name: &str, cmdline: &str, image: &[u8]) -> Result<Process, PeError> {
        let mut mem = UserMem::new();
        let loaded = pe::load(&mut mem, image)?;
        let top = mem.map_anywhere(1 << 20, PROT_READ | PROT_WRITE, &[], "stack").map_err(|_| PeError::NoMemory)? + (1 << 20);
        let rsp = top - 0x28; // 16-byte aligned stack, minus the return address a CALL would have pushed
        mem.write(rsp, &loaded.exit_stub.to_le_bytes()).map_err(|_| PeError::NoMemory)?;
        let mut line = String::from(name);
        if !cmdline.is_empty() {
            line.push(' ');
            line.push_str(cmdline);
        }
        line.push('\0');
        let cmdline_addr = mem.map_anywhere(line.len() as u64, PROT_READ | PROT_WRITE, line.as_bytes(), "cmdline").map_err(|_| PeError::NoMemory)?;
        // TEB at +0, PEB at +0x1000: StackBase/Limit, Self, ClientId, PEB pointer, ImageBaseAddress.
        let mut tb = alloc::vec![0u8; 0x3000];
        let teb = mem.map_anywhere(0x3000, PROT_READ | PROT_WRITE, &[], "teb").map_err(|_| PeError::NoMemory)?;
        let put = |b: &mut Vec<u8>, o: usize, v: u64| b[o..o + 8].copy_from_slice(&v.to_le_bytes());
        put(&mut tb, 0x08, top);
        put(&mut tb, 0x10, top - (1 << 20));
        put(&mut tb, 0x30, teb);
        put(&mut tb, 0x40, pid);
        put(&mut tb, 0x48, pid);
        put(&mut tb, 0x60, teb + 0x1000);
        put(&mut tb, 0x1000 + 0x10, loaded.base);
        put(&mut tb, 0x1000 + 0x20, teb + 0x2000); // PEB.ProcessParameters
        tb[0x2004..0x2008].copy_from_slice(&0x400u32.to_le_bytes());
        mem.write(teb, &tb).map_err(|_| PeError::NoMemory)?;
        let mut p = Process { teb, pid, session, mem, entry: loaded.entry, rsp, base: loaded.base, name: name.into(), console: Vec::new(), stdin: Vec::new(), console_host: false, console_title: String::new(), deferred: loaded.deferred, gui_atoms: Vec::new(), dw: Default::default(), cmdline_addr, cb_stub: loaded.cb_stub, soft_stubs: false, trace: false, threads: alloc::vec![thread::Thread::main(pid, teb)], cur: 0, cur_rsp: 0, regs: [0; 16], ret_addr: 0, images: Vec::new(), unhandled_filter: 0, xmm: [[0; 16]; 4], gls: Vec::new(), gl_cur: None, net: None, socks: Vec::new(), ws_err: 0, ws_init: 0, adv: Default::default(), modal: Vec::new(), crt: Default::default(), data: loaded.arena, rsrc: Default::default(), checks: Default::default(), dibs: Default::default(), dns: None, dns_seq: 0, scratch: 0, scratch_next: 0, spawned: Vec::new(), objs: Vec::new(), tls_count: 1, thread_exit_stub: loaded.thread_exit_stub, vfs: Default::default(), open: Vec::new(), finds: Vec::new(), modules: Vec::new(), next_dll_base: Process::dll_area(), pending_init: Vec::new(), stub_cache: Default::default(), stub_next: 0, stub_end: 0, dialog_open: String::new(), dialog_save: String::new(), menus: Default::default(), heap_base: 0, heap_next: 0, tls: alloc::vec![0; 1], cmdline_w: 0, env_w: 0, logged: Vec::new(), unresolved: loaded.unresolved };
        let nt = u32::from_le_bytes(image[0x3c..0x40].try_into().unwrap()) as usize;
        let rd = |o: usize| u32::from_le_bytes(image[o..o + 4].try_into().unwrap());
        p.register_image(loaded.base, u64::from(rd(nt + 24 + 56)), rd(nt + 24 + 112 + 24), rd(nt + 24 + 112 + 28));
        Ok(p)
    }

    /// Takes and clears pending console output.
    pub fn take_console(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.console)
    }

    /// Queues typed console input (a console window calls this with each finished line).
    pub fn push_stdin(&mut self, b: &[u8]) {
        self.stdin.extend_from_slice(b);
    }

    /// `ReadFile`/`ReadConsoleA/W` on the stdin handle: hands out at most one line. Blocks (the GUI driver retries
    /// after more input) while a console window is attached and no line is complete; otherwise reads end of file.
    pub(crate) fn console_read(&mut self, a: [u64; 4], wide: bool) -> Ret {
        let line_end = self.stdin.iter().position(|&c| c == b'\n');
        let Some(end) = line_end.or(if self.console_host { None } else { Some(self.stdin.len().saturating_sub(1)) }) else { return Ret::Idle };
        let unit = if wide { 2 } else { 1 };
        let n = if self.stdin.is_empty() { 0 } else { (end + 1).min(a[2] as usize) };
        let taken: Vec<u8> = self.stdin.drain(..n).collect();
        let bytes: Vec<u8> = if wide { taken.iter().flat_map(|&c| [c, 0]).collect() } else { taken };
        let _ = self.mem.write(a[1], &bytes);
        if a[3] != 0 {
            let _ = self.mem.write(a[3], &((bytes.len() / unit) as u32).to_le_bytes());
        }
        Ret::Value(1)
    }

    /// Whether a PE image targets the console subsystem (IMAGE_SUBSYSTEM_WINDOWS_CUI).
    pub fn is_console_image(image: &[u8]) -> bool {
        let lfanew = image.get(0x3c..0x40).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        image.get(lfanew + 0x5c..lfanew + 0x5e).is_some_and(|b| u16::from_le_bytes([b[0], b[1]]) == 3)
    }

    pub(crate) fn console_write(&mut self, b: &[u8]) {
        self.console.extend_from_slice(b);
    }

    pub(crate) fn cmdline_addr(&self) -> u64 {
        self.cmdline_addr
    }

    pub(crate) fn set_last_error(&mut self, e: u32) {
        let i = self.cur;
        self.threads[i].last_error = e;
    }

    pub(crate) fn last_error(&self) -> u32 {
        self.threads[self.cur].last_error
    }
}

/// Reads a NUL-terminated UTF-16 string from guest memory.
pub(crate) fn k32_wide(mem: &mut UserMem, a: u64) -> Option<String> {
    let mut v = Vec::new();
    for i in 0..512u64 {
        let mut b = [0u8; 2];
        mem.read(a + i * 2, &mut b).ok()?;
        let c = u16::from_le_bytes(b);
        if c == 0 {
            return Some(String::from_utf16_lossy(&v));
        }
        v.push(c);
    }
    None
}
