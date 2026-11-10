//! Kernel side of the Linux subsystem: builds user page tables from `loos_lin::mem::UserMem`,
//! enters ring 3 through `run_user` (usermode.S) and services SYSCALLs / exceptions with
//! `LinSystem`. Single CPU, cooperative: a thread runs until it traps; there is no preemption.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::arch::asm;
use loos_lin::fs::MemFs;
use loos_lin::mem::{UserMem, PROT_EXEC, PROT_WRITE};
use loos_lin::sys::{Delivery, LinSystem, Pid, Sys, TState};
use loos_win32k::adjust_window_rect;
use loos_win32k::{Hwnd, Point, Rect, SC_CLOSE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSCOMMAND, WS_OVERLAPPEDWINDOW, WS_VISIBLE};

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
    fn rtc_unix() -> u64;
    pub(crate) fn user_init();
    /// Copy the kernel video backbuffer to the screen.
    fn gui_present();
    fn ps2_init();
    fn ps2_poll_raw(out: *mut KRaw) -> i32;
}

/// `struct kraw` (kernel.h): one PS/2 event, keys as Linux evdev codes.
#[repr(C)]
#[derive(Default)]
struct KRaw {
    ty: i32,
    key: u32,
    down: i32,
    dx: i32,
    dy: i32,
    buttons: u32,
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
        let mut ok = true;
        mem.for_each_page(|va, pa, prot| ok &= map_page(pml4, va, pa, prot));
        if !ok {
            return 0;
        }
        pml4 as u64
    }
}

/// Set (or with `prot == 0`, clear) the leaf entry for `va` in `pml4`. False when out of memory.
pub(crate) fn map_page(pml4: *mut u64, va: u64, pa: u64, prot: u32) -> bool {
    // SAFETY: tables come from kalloc (identity mapped); indices are masked to 0..512.
    unsafe {
        let mut t = pml4;
        for shift in [39u32, 30, 21] {
            let e = t.add(((va >> shift) & 511) as usize);
            if *e & P == 0 {
                if prot == 0 {
                    return true;
                }
                let n = new_table();
                if n.is_null() {
                    return false;
                }
                *e = n as u64 | P | W | U;
            }
            t = (*e & 0x000f_ffff_ffff_f000) as *mut u64;
        }
        let leaf = if prot == 0 { 0 } else { pa | P | U | if prot & PROT_WRITE != 0 { W } else { 0 } | if prot & PROT_EXEC == 0 { NX } else { 0 } };
        *t.add(((va >> 12) & 511) as usize) = leaf;
        // Drop a stale translation if this table is live (others are flushed by the next CR3 load).
        if read_cr3() & 0x000f_ffff_ffff_f000 == pml4 as u64 {
            asm!("invlpg [{}]", in(reg) va, options(nostack));
        }
        true
    }
}

/// A plain arrow pointer (1 = black outline, 2 = white fill).
fn draw_cursor(fb: &mut [u32], screen: (i32, i32), at: (i32, i32)) {
    const ARROW: [&str; 16] = [
        "1", "11", "121", "1221", "12221", "122221", "1222221", "12222221", "122222221", "1222222221", "12222211111", "1221221",
        "121 1221", "11  1221", "1    1221", "     111",
    ];
    for (dy, row) in ARROW.iter().enumerate() {
        for (dx, c) in row.bytes().enumerate() {
            let (x, y) = (at.0 + dx as i32, at.1 + dy as i32);
            if x < 0 || y < 0 || x >= screen.0 || y >= screen.1 {
                continue;
            }
            let i = (y * screen.0 + x) as usize;
            match c {
                b'1' => fb[i] = 0xff00_0000,
                b'2' => fb[i] = 0xffff_ffff,
                _ => {}
            }
        }
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

/// The Linux layer's view of the kernel network: the NIC-backed stack and its pump.
struct KLinNet;

impl loos_lin::sys::LinNet for KLinNet {
    fn stack(&mut self) -> Option<&mut loos_net::stack::Stack> {
        crate::sys::net_stack()
    }
    fn pump(&mut self, now_ms: u64) {
        crate::sys::loos_net_poll(now_ms);
    }
}

struct Lin {
    sys: LinSystem<MemFs>,
    pml4: BTreeMap<Pid, (u64, u64)>,
    last: Pid,
    desk: alloc::rc::Rc<core::cell::RefCell<loos_win32k::Win32k>>,
    /// LoOS windows showing Wayland surfaces, keyed by (connection, wl_surface id).
    wayland_windows: BTreeMap<(usize, u32), Hwnd>,
    /// Saved x87/SSE state (FXSAVE image) of threads not currently loaded in the frame.
    fx: BTreeMap<Pid, alloc::boxed::Box<[u8; 512]>>,
    /// Thread whose FPU state is in the frame now; new threads/children inherit it (like Linux's copy on clone).
    fx_owner: Pid,
    /// Kernel video backbuffer (`screen.0 * screen.1` pixels) or null when headless.
    back: *mut u32,
    screen: (i32, i32),
    dirty: bool,
    last_present_us: u64,
    /// PS/2 input is routed to Linux windows (only with a screen).
    input: bool,
    cursor: (i32, i32),
    buttons: u32,
    /// Held modifiers as XKB masks (Shift 1, Ctrl 4, Alt 8, Super 64) and Caps Lock (2).
    mods: u32,
    caps: bool,
}

impl Lin {
    fn flush_console(&mut self) {
        for b in self.sys.console_take() {
            // SAFETY: port I/O.
            unsafe { serial_putc(b as i8) };
        }
    }

    fn cr3_for(&mut self, pid: Pid) -> u64 {
        let Some(p) = self.sys.process_mut(pid) else { return 0 };
        let ver = p.mem.version;
        let remap = p.mem.take_remap();
        if let Some(&(v, cr3)) = self.pml4.get(&pid) {
            if v == ver {
                // Kernel writes that allocated or un-shared pages: refresh just those entries.
                for va in remap {
                    let (pa, prot) = p.mem.pte(va).unwrap_or((0, 0));
                    if !map_page(cr3 as *mut u64, va, pa, prot) {
                        return 0;
                    }
                }
                return cr3;
            }
        }
        let p = self.sys.process(pid).unwrap();
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
        // SAFETY: monotonic clock.
        let ms = (unsafe { time_us() } / 1000) as u32;
        for (conn, frame) in self.sys.take_wayland_frames(ms) {
            let key = (conn, frame.surface);
            let hwnd = if let Some(&h) = self.wayland_windows.get(&key) {
                h
            } else {
                let title = core::str::from_utf8(if frame.title.is_empty() { &frame.app_id } else { &frame.title }).unwrap_or("").to_string();
                let style = if frame.toplevel { WS_OVERLAPPEDWINDOW | WS_VISIBLE } else { WS_VISIBLE };
                let n = self.wayland_windows.len() as i32;
                let outer = adjust_window_rect(style, false, Rect { left: 0, top: 0, right: frame.width as i32, bottom: frame.height as i32 });
                let Ok(h) = self.desk.borrow_mut().create_window_ex(0, "LooSLinux", if title.is_empty() { "Linux application" } else { &title }, style, 40 + 24 * n, 40 + 24 * n, outer.right - outer.left, outer.bottom - outer.top, 0, 0, 0) else { continue };
                self.wayland_windows.insert(key, h);
                if frame.toplevel {
                    // A new top-level window gets the focus, like on any desktop.
                    self.desk.borrow_mut().set_active_window(h);
                    let surface = frame.surface;
                    self.sys.wayland_input(conn, |w| w.keyboard_focus(surface));
                }
                say("lin: wayland window \"");
                say(&title);
                say("\" ");
                // SAFETY: serial output.
                unsafe {
                    serial_dec(frame.width as u64);
                    say("x");
                    serial_dec(frame.height as u64);
                }
                let o = self.desk.borrow().client_to_screen(h, Point { x: 0, y: 0 }).unwrap_or_default();
                say(" client at ");
                // SAFETY: serial output.
                unsafe {
                    serial_dec(o.x as u64);
                    say(",");
                    serial_dec(o.y as u64);
                }
                say("\n");
                h
            };
            {
                // Follow client-side resizes: keep the client area equal to the buffer.
                let mut d = self.desk.borrow_mut();
                if let (Ok(c), Ok(w)) = (d.get_client_rect(hwnd), d.get_window_rect(hwnd)) {
                    let (dw, dh) = (frame.width as i32 - (c.right - c.left), frame.height as i32 - (c.bottom - c.top));
                    if dw != 0 || dh != 0 {
                        d.move_window(hwnd, w.left, w.top, w.right - w.left + dw, w.bottom - w.top + dh).ok();
                    }
                }
                d.import_window_pixels(hwnd, frame.width as i32, frame.height as i32, &frame.pixels);
            }
            self.dirty = true;
        }
        let mut gone = self.sys.take_wayland_closed();
        gone.extend(self.wayland_windows.keys().filter(|k| !self.sys.wayland_alive(k.0)).copied());
        for key in gone {
            if let Some(h) = self.wayland_windows.remove(&key) {
                self.desk.borrow_mut().destroy_window(h).ok();
                self.dirty = true;
            }
        }
        self.show();
    }

    /// Route PS/2 input: the mouse drives the window manager (activation, dragging, close
    /// buttons) and becomes Wayland pointer events on Linux windows; keys go to the active one.
    fn poll_input(&mut self) {
        if !self.input {
            return;
        }
        let mut ev = KRaw::default();
        let mut any = false;
        // SAFETY: polled port I/O into a local struct.
        while unsafe { ps2_poll_raw(&mut ev) } != 0 {
            any = true;
            if ev.ty == 2 {
                self.cursor = ((self.cursor.0 + ev.dx).clamp(0, self.screen.0 - 1), (self.cursor.1 + ev.dy).clamp(0, self.screen.1 - 1));
                let mut d = self.desk.borrow_mut();
                d.inject_mouse_move(self.cursor.0, self.cursor.1);
                for (bit, b) in [(1u32, 0u32), (2, 1), (4, 2)] {
                    if (ev.buttons ^ self.buttons) & bit != 0 {
                        d.inject_mouse_button(b, ev.buttons & bit != 0);
                    }
                }
                self.buttons = ev.buttons;
                self.dirty = true;
            } else {
                self.key(ev.key, ev.down != 0);
            }
        }
        if any {
            self.route_window_messages();
        }
    }

    fn key(&mut self, code: u32, down: bool) {
        let bit = match code {
            42 | 54 => 1,
            29 | 97 => 4,
            56 | 100 => 8,
            125 | 126 => 64,
            _ => 0,
        };
        if down {
            self.mods |= bit;
        } else {
            self.mods &= !bit;
        }
        if code == 58 && down {
            self.caps = !self.caps;
        }
        let active = self.desk.borrow().get_active_window();
        let Some(&(conn, surface)) = self.wayland_windows.iter().find(|(_, &h)| h == active).map(|(k, _)| k) else { return };
        // SAFETY: monotonic clock.
        let ms = (unsafe { time_us() } / 1000) as u32;
        let (mods, lock) = (self.mods, if self.caps { 2 } else { 0 });
        self.sys.wayland_input(conn, |w| {
            w.keyboard_focus(surface);
            w.key(code, down, (mods, 0, lock), ms);
        });
    }

    /// Turn window-manager messages queued for Linux windows into Wayland events.
    fn route_window_messages(&mut self) {
        // SAFETY: monotonic clock.
        let ms = (unsafe { time_us() } / 1000) as u32;
        let wins: Vec<((usize, u32), Hwnd)> = self.wayland_windows.iter().map(|(k, h)| (*k, *h)).collect();
        for ((conn, surface), h) in wins {
            loop {
                let Some(m) = self.desk.borrow_mut().peek_message(h, 0, 0, true) else { break };
                let (x, y) = ((m.lparam & 0xffff) as i16 as i32, ((m.lparam >> 16) & 0xffff) as i16 as i32);
                let button = match m.message {
                    WM_LBUTTONDOWN | WM_LBUTTONUP => Some(0x110),
                    WM_RBUTTONDOWN | WM_RBUTTONUP => Some(0x111),
                    WM_MBUTTONDOWN | WM_MBUTTONUP => Some(0x112),
                    _ => None,
                };
                let pressed = matches!(m.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN);
                match m.message {
                    WM_MOUSEMOVE => self.sys.wayland_input(conn, |w| w.pointer_motion(surface, x, y, ms)),
                    _ if button.is_some() => self.sys.wayland_input(conn, |w| {
                        w.pointer_motion(surface, x, y, ms);
                        w.pointer_button(button.unwrap(), pressed, ms);
                    }),
                    WM_SYSCOMMAND if m.wparam & 0xfff0 == SC_CLOSE => self.sys.wayland_input(conn, |w| w.close(surface)),
                    _ => {}
                }
            }
        }
    }

    /// Compose the desktop onto the screen (at most ~60 times a second).
    fn show(&mut self) {
        // SAFETY: monotonic clock.
        let now = unsafe { time_us() };
        if !self.dirty || self.back.is_null() || now.saturating_sub(self.last_present_us) < 16_000 {
            return;
        }
        // SAFETY: the kernel owns a writable `w * h` backbuffer for the whole run.
        let frame = unsafe { core::slice::from_raw_parts_mut(self.back, (self.screen.0 * self.screen.1) as usize) };
        self.desk.borrow_mut().compose(frame);
        if self.input {
            draw_cursor(frame, self.screen, self.cursor);
        }
        // SAFETY: plain kernel call; gui_vid was set before loos_lin_boot.
        unsafe { gui_present() };
        self.dirty = false;
        self.last_present_us = now;
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
            if self.sys.net_attached() {
                self.sys.poll_net(now / 1000);
            }
            self.poll_input();
            let Some(tid) = self.pick() else {
                self.last_present_us = 0; // idle: show any frame the throttle held back
                self.show();
                if (self.sys.net_attached() && self.sys.io_waiters()) || (self.input && !self.wayland_windows.is_empty()) {
                    continue; // waiting for the network or the user: keep polling in real time
                }
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
            if self.fx_owner != tid {
                if self.sys.thread(self.fx_owner).is_some() {
                    self.fx.insert(self.fx_owner, alloc::boxed::Box::new(frame.fx));
                }
                if let Some(saved) = self.fx.remove(&tid) {
                    frame.fx = *saved;
                }
                self.fx_owner = tid;
                let sys = &self.sys;
                self.fx.retain(|t, _| sys.thread(*t).is_some());
            }
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
                let write = frame.error & 2 != 0;
                let fixed = vector == 14
                    && match self.sys.process_mut(pid).and_then(|p| p.mem.handle_fault(cr2, write)) {
                        Some((va, pa, prot)) => match self.pml4.get(&pid) {
                            Some(&(_, cr3)) => map_page(cr3 as *mut u64, va, pa, prot),
                            None => true,
                        },
                        None => false,
                    };
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
    // The kernel command line ends at a space, so arguments spell space/comma as %20 / %2C.
    let mut argv: Vec<String> = cmd.split(',').map(|a| a.replace("%20", " ").replace("%2C", ",").replace("%25", "%")).collect();
    let trace = argv.first().is_some_and(|a| a == "strace");
    if trace {
        argv.remove(0);
    }
    let mut sys = LinSystem::new(fs);
    sys.trace = trace;
    if !back.is_null() {
        sys.wayland_screen = (width.max(1) as u32, height.max(1) as u32);
    }
    sys.stdin_eof = true;
    // SAFETY: plain C accessor (CMOS clock as Unix seconds).
    sys.epoch_base_s = unsafe { rtc_unix() };
    sys.attach_net(KLinNet);
    // liblooswin: Linux programs reach the LoOS window system through bridge syscalls.
    let screen_w = width.max(1);
    let screen_h = height.max(1);
    let desk = alloc::rc::Rc::new(core::cell::RefCell::new(loos_win32k::Win32k::new(screen_w, screen_h)));
    sys.set_bridge(alloc::boxed::Box::new(loos_lin::bridge::Win32Bridge::new(desk.clone())));
    let mut env = alloc::vec![
        String::from("PATH=/bin:/usr/bin"),
        String::from("HOME=/home"),
        String::from("TERM=linux"),
        String::from("XDG_RUNTIME_DIR=/run/user/1000"),
        String::from("WAYLAND_DISPLAY=wayland-0"),
        String::from("GDK_BACKEND=wayland"),
        // No GPU driver yet: toolkits render in software into wl_shm buffers.
        String::from("GSK_RENDERER=cairo"),
        String::from("GDK_DISABLE=gl,vulkan,dmabuf"),
        String::from("LIBGL_ALWAYS_SOFTWARE=1"),
        String::from("MOZ_ENABLE_WAYLAND=1"),
        String::from("NO_AT_BRIDGE=1"),
        String::from("GTK_A11Y=none"),
        String::from("LANG=C.UTF-8"),
    ];
    // Leading `VAR=value` entries of loos.lin= are extra environment: loos.lin=FOO=1,/usr/bin/app,arg
    while argv.first().is_some_and(|a| a.contains('=') && !a.starts_with('/')) {
        env.push(argv.remove(0));
    }
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
    let mut lin = Lin { sys, pml4: BTreeMap::new(), last: 0, desk: desk.clone(), wayland_windows: BTreeMap::new(), fx: BTreeMap::new(), fx_owner: 0, back, screen: (screen_w, screen_h), dirty: false, last_present_us: 0, input: !back.is_null(), cursor: (screen_w / 2, screen_h / 2), buttons: 0, mods: 0, caps: false };
    if lin.input {
        // SAFETY: polled i8042 setup, as the desktop does.
        unsafe { ps2_init() };
    }
    let st = lin.run(pid);
    if !lin.sys.missing.is_empty() {
        say("lin: unimplemented syscalls:");
        for (nr, n) in &lin.sys.missing {
            say(" ");
            // SAFETY: serial output.
            unsafe { serial_dec(*nr) };
            say("x");
            unsafe { serial_dec(*n as u64) };
        }
        say("\n");
    }
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
