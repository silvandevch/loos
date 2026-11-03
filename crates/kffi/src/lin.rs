//! Kernel side of the Linux subsystem: builds user page tables from `loos_lin::mem::UserMem`,
//! enters ring 3 through `run_user` (usermode.S) and services SYSCALLs / exceptions with
//! `LinSystem`. Single CPU, cooperative: a thread runs until it traps; there is no preemption.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::asm;
use loos_lin::fs::MemFs;
use loos_lin::mem::{UserMem, PROT_EXEC, PROT_WRITE};
use loos_lin::sys::{Delivery, LinSystem, Pid, Sys, TState};
use loos_win32k::{Hwnd, WS_OVERLAPPEDWINDOW, WS_VISIBLE};

#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub(crate) struct UFrame {
    pub(crate) gpr: [u64; 16],
    pub(crate) vector: u64,
    pub(crate) error: u64,
    pub(crate) rip: u64,
    pub(crate) cs: u64,
    pub(crate) rflags: u64,
    pub(crate) rsp: u64,
    pub(crate) ss: u64,
    pub(crate) cr3: u64,
    pub(crate) _pad: u64,
    pub(crate) fx: [u8; 512],
}

extern "C" {
    pub(crate) fn run_user(f: *mut UFrame) -> u64;
    fn kalloc(size: u64, align: u64) -> *mut u8;
    fn serial_putc(c: i8);
    fn serial_puts(s: *const u8);
    fn serial_dec(v: u64);
    pub(crate) fn time_us() -> u64;
    pub(crate) fn user_init();
}

const P: u64 = 1;
const W: u64 = 2;
const U: u64 = 4;
const NX: u64 = 1 << 63;
pub(crate) const MSR_FS_BASE: u32 = 0xC000_0100;

pub(crate) fn say(s: &str) {
    for b in s.bytes() {
        // SAFETY: serial_putc is plain port I/O.
        unsafe { serial_putc(b as i8) };
    }
}

unsafe fn new_table() -> *mut u64 {
    // SAFETY: kalloc returns identity-mapped zero-unknown memory or null.
    unsafe {
        let p = kalloc(4096, 4096) as *mut u64;
        if !p.is_null() {
            core::ptr::write_bytes(p, 0, 512);
        }
        p
    }
}

fn read_cr3() -> u64 {
    let v: u64;
    // SAFETY: reading CR3 is side-effect free.
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack)) };
    v
}

pub(crate) fn read_cr2() -> u64 {
    let v: u64;
    // SAFETY: reading CR2 is side-effect free.
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack)) };
    v
}

pub(crate) unsafe fn wrmsr(msr: u32, v: u64) {
    // SAFETY: caller passes a valid MSR.
    unsafe { asm!("wrmsr", in("ecx") msr, in("eax") v as u32, in("edx") (v >> 32) as u32, options(nomem, nostack)) };
}

/// Build a PML4 mapping every page of `mem` as user memory; entry 511 (kernel image) is shared.
pub(crate) fn build_pml4(mem: &UserMem) -> u64 {
    // SAFETY: tables come from kalloc (identity mapped); indices are masked to 0..512.
    unsafe {
        let pml4 = new_table();
        if pml4.is_null() {
            return 0;
        }
        let kernel = read_cr3() & 0x000f_ffff_ffff_f000;
        *pml4.add(511) = *(kernel as *const u64).add(511);
        mem.for_each_page(|va, pa, prot| {
            if prot == 0 {
                return;
            }
            let mut t = pml4;
            for shift in [39u32, 30, 21] {
                let e = t.add(((va >> shift) & 511) as usize);
                if *e & P == 0 {
                    let n = new_table();
                    if n.is_null() {
                        return;
                    }
                    *e = n as u64 | P | W | U;
                }
                t = (*e & 0x000f_ffff_ffff_f000) as *mut u64;
            }
            let leaf = pa | P | U | if prot & PROT_WRITE != 0 { W } else { 0 } | if prot & PROT_EXEC == 0 { NX } else { 0 };
            *t.add(((va >> 12) & 511) as usize) = leaf;
        });
        pml4 as u64
    }
}

fn signal_for_vector(v: u64) -> u32 {
    match v {
        0 | 16 | 19 => 8,
        1 | 3 => 5,
        6 => 4,
        17 => 7,
        _ => 11,
    }
}

struct Lin {
    sys: LinSystem<MemFs>,
    pml4: BTreeMap<Pid, (u64, u64)>,
    last: Pid,
    desk: alloc::rc::Rc<core::cell::RefCell<loos_win32k::Win32k>>,
    wayland_windows: BTreeMap<u32, Hwnd>,
}

impl Lin {
    fn flush_console(&mut self) {
        for b in self.sys.console_take() {
            // SAFETY: port I/O.
            unsafe { serial_putc(b as i8) };
        }
    }

    fn cr3_for(&mut self, pid: Pid) -> u64 {
        let Some(p) = self.sys.process(pid) else { return 0 };
        let ver = p.mem.version;
        if let Some(&(v, cr3)) = self.pml4.get(&pid) {
            if v == ver {
                return cr3;
            }
        }
        let cr3 = build_pml4(&p.mem);
        self.pml4.insert(pid, (ver, cr3));
        cr3
    }

    fn pick(&mut self) -> Option<Pid> {
        let ids = self.sys.thread_ids();
        let runnable = |s: &LinSystem<MemFs>, t: Pid| s.thread(t).is_some_and(|t| t.state == TState::Runnable);
        ids.iter().copied().find(|&t| t > self.last && runnable(&self.sys, t)).or_else(|| ids.iter().copied().find(|&t| runnable(&self.sys, t)))
    }

    fn present_wayland(&mut self) {
        for frame in self.sys.take_wayland_frames() {
            let hwnd = if let Some(&h) = self.wayland_windows.get(&frame.surface) {
                h
            } else {
                let mut desk = self.desk.borrow_mut();
                let Ok(h) = desk.create_window_ex(
                    0,
                    "LooSLinux",
                    core::str::from_utf8(&frame.title).unwrap_or("Wayland application"),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    40,
                    40,
                    frame.width as i32,
                    frame.height as i32,
                    0,
                    0,
                    0,
                ) else { continue };
                self.wayland_windows.insert(frame.surface, h);
                h
            };
            self.desk.borrow_mut().import_window_pixels(hwnd, frame.width as i32, frame.height as i32, &frame.pixels);
        }
    }

    /// Run until process `main` ends. Returns its wait status (exit code << 8, or signal number).
    fn run(&mut self, main: Pid) -> i32 {
        let mut clock = unsafe { time_us() };
        let mut status = None;
        let mut idle_spins = 0u32;
        let mut frame: UFrame = UFrame { gpr: [0; 16], vector: 0, error: 0, rip: 0, cs: 0x23, rflags: 2, rsp: 0, ss: 0x1b, cr3: 0, _pad: 0, fx: [0; 512] };
        // Default x87/SSE state: FCW=0x37f, MXCSR=0x1f80.
        frame.fx[0] = 0x7f;
        frame.fx[1] = 0x03;
        frame.fx[24..28].copy_from_slice(&0x1f80u32.to_le_bytes());
        while status.is_none() {
            // SAFETY: monotonic microsecond clock.
            let now = unsafe { time_us() };
            self.sys.advance((now - clock) * 1000);
            clock = now;
            let Some(tid) = self.pick() else {
                match self.sys.next_deadline() {
                    Some(d) => {
                        let wait = d.saturating_sub(self.sys.now_ns);
                        self.sys.advance(wait.max(1));
                        idle_spins = 0;
                        continue;
                    }
                    None => {
                        say("lin: deadlock (all threads blocked)\n");
                        return -1;
                    }
                }
            };
            self.last = tid;
            let pid = self.sys.thread(tid).unwrap().pid;
            match self.sys.deliver_signal(tid) {
                Delivery::Killed(sig) => {
                    if pid == main {
                        status = Some(sig as i32);
                    }
                    self.flush_console();
                    continue;
                }
                Delivery::Handler(_) | Delivery::None => {}
            }
            let cr3 = self.cr3_for(pid);
            if cr3 == 0 {
                say("lin: out of memory building page tables\n");
                return -1;
            }
            let t = self.sys.thread(tid).unwrap();
            let r = t.regs;
            frame.gpr = [r.rax, r.rcx, r.rdx, r.rbx, 0, r.rbp, r.rsi, r.rdi, r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15];
            frame.rip = r.rip;
            frame.rsp = r.rsp;
            frame.rflags = (r.rflags & 0xcd5) | 2;
            frame.cr3 = cr3;
            // SAFETY: FS base MSR; run_user enters ring 3 with a fully initialised frame and returns on trap.
            let vector = unsafe {
                wrmsr(MSR_FS_BASE, r.fs_base);
                run_user(&mut frame)
            };
            let g = &frame.gpr;
            {
                let t = self.sys.thread_mut(tid).unwrap();
                let r = &mut t.regs;
                [r.rax, r.rcx, r.rdx, r.rbx, _, r.rbp, r.rsi, r.rdi, r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15] = *g;
                r.rip = frame.rip;
                r.rsp = frame.rsp;
                r.rflags = frame.rflags;
            }
            if vector == 0x100 {
                idle_spins = 0;
                let (nr, a) = (g[0], [g[7], g[6], g[2], g[10], g[8], g[9]]);
                match self.sys.syscall(tid, nr, a) {
                    Sys::ProcessExit(st) if pid == main => status = Some(st),
                    _ => {}
                }
                self.present_wayland();
            } else {
                let cr2 = read_cr2();
                let fixed = vector == 14 && self.sys.process_mut(pid).is_some_and(|p| p.mem.handle_fault(cr2));
                if !fixed {
                    idle_spins += 1;
                    if idle_spins > 8 {
                        say("lin: unrecoverable fault loop\n");
                        return -2;
                    }
                    say("lin: fault vector ");
                    // SAFETY: serial output.
                    unsafe { serial_dec(vector) };
                    say(" rip ");
                    unsafe { serial_dec(frame.rip) };
                    say(" cr2 ");
                    unsafe { serial_dec(cr2) };
                    say("\n");
                    self.sys.send_signal(pid, Some(tid), signal_for_vector(vector));
                }
            }
            self.flush_console();
            if status.is_none() && self.sys.process(main).is_none() {
                status = Some(0);
            }
        }
        self.flush_console();
        status.unwrap_or(0)
    }
}

/// Boot entry: `cmd` is `path,arg1,arg2,...`; the root filesystem is the tar in `initrd`.
/// When `back` is supplied, the composed Linux desktop is presented into that
/// kernel video backbuffer at the supplied dimensions before returning.
/// Returns the program's wait status (`exit << 8`), or a negative value on setup failure.
#[no_mangle]
pub unsafe extern "C" fn loos_lin_boot(
    initrd: *const u8,
    len: usize,
    cmd: *const u8,
    back: *mut u32,
    width: i32,
    height: i32,
) -> i32 {
    // SAFETY: initrd/len describe the boot module; cmd is NUL terminated.
    let (tar, cmd) = unsafe {
        user_init();
        let mut n = 0;
        while *cmd.add(n) != 0 && *cmd.add(n) != b' ' {
            n += 1;
        }
        (if initrd.is_null() { &[][..] } else { core::slice::from_raw_parts(initrd, len) }, String::from_utf8_lossy(core::slice::from_raw_parts(cmd, n)).into_owned())
    };
    let mut fs = MemFs::new();
    for d in ["/bin", "/usr/bin", "/lib", "/lib64", "/etc", "/tmp", "/dev", "/proc", "/home", "/run", "/run/user", "/run/user/1000"] {
        fs.add_dir(d);
    }
    let files = fs.load_tar(tar);
    say("lin: rootfs ");
    // SAFETY: serial output.
    unsafe { serial_dec(files as u64) };
    say(" entries\n");
    let argv: Vec<String> = cmd.split(',').map(String::from).collect();
    let mut sys = LinSystem::new(fs);
    sys.stdin_eof = true;
    // liblooswin: Linux programs reach the LoOS window system through bridge syscalls.
    let screen_w = width.max(1);
    let screen_h = height.max(1);
    let desk = alloc::rc::Rc::new(core::cell::RefCell::new(loos_win32k::Win32k::new(screen_w, screen_h)));
    sys.set_bridge(alloc::boxed::Box::new(loos_lin::bridge::Win32Bridge::new(desk.clone())));
    let env = [
        String::from("PATH=/bin:/usr/bin"),
        String::from("HOME=/home"),
        String::from("TERM=linux"),
        String::from("XDG_RUNTIME_DIR=/run/user/1000"),
        String::from("WAYLAND_DISPLAY=wayland-0"),
        String::from("GDK_BACKEND=wayland"),
    ];
    let pid = match sys.spawn(&argv[0], &argv, &env) {
        Ok(p) => p,
        Err(e) => {
            say("lin: cannot execute (errno ");
            // SAFETY: serial output.
            unsafe { serial_dec(e as u64) };
            say(")\n");
            return -1;
        }
    };
    let mut lin = Lin { sys, pml4: BTreeMap::new(), last: 0, desk: desk.clone(), wayland_windows: BTreeMap::new() };
    let st = lin.run(pid);
    // Compose the Win32 bridge's desktop into the real kernel video backbuffer.
    // The Linux GUI corpus paints a known green client rectangle; report its
    // pixels so the headless QEMU proof checks rendering, not just syscalls.
    let pixels = (screen_w as usize).saturating_mul(screen_h as usize);
    let frame = if back.is_null() {
        &mut []
    } else {
        // SAFETY: the kernel owns a writable `width * height` ARGB backbuffer.
        unsafe { core::slice::from_raw_parts_mut(back, pixels) }
    };
    desk.borrow_mut().compose(frame);
    let green = frame.iter().filter(|&&p| p == 0xff00_c800).count();
    say("lin: gui composed green pixels ");
    // SAFETY: serial output.
    unsafe { serial_dec(green as u64) };
    say("\n");
    // SAFETY: NUL-terminated literal.
    unsafe { serial_puts(b"lin: program finished, status \0".as_ptr()) };
    unsafe { serial_dec(st as u32 as u64) };
    say("\n");
    st
}
