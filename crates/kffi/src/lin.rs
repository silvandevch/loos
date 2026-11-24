//! Kernel side of the Linux subsystem: builds user page tables from `loos_lin::mem::UserMem`,
//! enters ring 3 through `run_user` (usermode.S) and services SYSCALLs / exceptions with
//! `LinSystem`. Single CPU, cooperative: a thread runs until it traps; there is no preemption.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::arch::asm;
use loos_lin::fs::{LinFs, MemFs};
use loos_lin::mem::{UserMem, PROT_EXEC, PROT_WRITE};
use loos_lin::sys::{Delivery, LinSystem, Pid, Sys, TState};
use loos_win32k::adjust_window_rect;
use loos_win32k::{Hwnd, Point, Rect, SC_CLOSE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSCOMMAND, WS_OVERLAPPEDWINDOW, WS_VISIBLE};

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
    /// FXSAVE image; usermode.S does `fxsave`/`fxrstor 192(frame)`.
    pub(crate) fx: [u8; 512],
}

const _: () = assert!(core::mem::offset_of!(UFrame, fx) == 192, "UFrame.fx must sit where usermode.S saves the FPU state");

extern "C" {
    pub(crate) fn run_user(f: *mut UFrame) -> u64;
    fn serial_putc(c: i8);
    fn serial_puts(s: *const u8);
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
const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;

/// Decimal / hex number on the serial line and the on-screen console.
fn dec(v: u64) {
    say(&alloc::format!("{v}"));
}
fn hex(v: u64) {
    say(&alloc::format!("{v:#018x}"));
}

pub(crate) fn say(s: &str) {
    for b in s.bytes() {
        // SAFETY: serial_putc is plain port I/O.
        unsafe { serial_putc(b as i8) };
    }
    console_log(s.as_bytes());
}

/// The on-screen console: the last lines of serial output (program output and `lin:` messages),
/// so a running Linux program is visibly alive before it opens a window.
struct ConsoleLog {
    lines: alloc::collections::VecDeque<Vec<u8>>,
    changed: bool,
}

struct LogCell(core::cell::UnsafeCell<Option<ConsoleLog>>);
// SAFETY: the kernel runs this code on one CPU, never from interrupts.
unsafe impl Sync for LogCell {}
static LOG: LogCell = LogCell(core::cell::UnsafeCell::new(None));
const LOG_LINES: usize = 40;

fn console_log(bytes: &[u8]) {
    // SAFETY: single-threaded kernel (see LogCell).
    let log = unsafe { &mut *LOG.0.get() }.get_or_insert_with(|| ConsoleLog { lines: alloc::collections::VecDeque::from([Vec::new()]), changed: true });
    for &b in bytes {
        match b {
            b'\n' => {
                // Detail lines (indented diagnostics: thread and memory dumps, syscall traces) stay on the
                // serial log; drawing them all slows the program down.
                let l = log.lines.back_mut().unwrap();
                if l.starts_with(b"lin:   ") || l.starts_with(b" ") {
                    l.clear();
                    continue;
                }
                log.lines.push_back(Vec::new());
                if log.lines.len() > LOG_LINES {
                    log.lines.pop_front();
                }
            }
            b'\r' => {}
            _ => {
                let l = log.lines.back_mut().unwrap();
                if l.len() < 240 {
                    l.push(if b == b'\t' { b' ' } else { b });
                }
            }
        }
    }
    log.changed = true;
}

/// Draw the console panel over the bottom of `fb`. Returns false when there is nothing to draw.
fn draw_console(fb: &mut [u32], screen: (i32, i32)) {
    use loos_shell::draw::{Canvas, Rect as SRect};
    // SAFETY: single-threaded kernel (see LogCell).
    let Some(log) = (unsafe { &mut *LOG.0.get() }) else { return };
    log.changed = false;
    let (w, h) = screen;
    let line_h = 16;
    let rows = log.lines.len() as i32;
    let panel_h = (rows * line_h + 34).min(h * 2 / 3);
    let mut c = Canvas { px: &mut fb[..(w * h) as usize], w, h, stride: w, scale: 0 };
    let all = SRect::new(0, 0, w, h);
    let top = h - panel_h;
    c.fill(all, SRect::new(0, top, w, panel_h), 0xE0101418);
    let font = loos_shell::font_data::regular();
    loos_shell::text::draw(&mut c, all, 12, top + 6, b"Linux console  (F12 hides/shows)", 0xFF8FB4FF, &font, 13);
    let visible = ((panel_h - 28) / line_h).max(0) as usize;
    let first = log.lines.len().saturating_sub(visible);
    for (i, l) in log.lines.iter().skip(first).enumerate() {
        let color = if l.starts_with(b"lin:") { 0xFFFFD27F } else { 0xFFE6E6E6 };
        loos_shell::text::draw(&mut c, all, 12, top + 26 + i as i32 * line_h, l, color, &font, 13);
    }
}

fn console_changed() -> bool {
    // SAFETY: single-threaded kernel (see LogCell).
    unsafe { &*LOG.0.get() }.as_ref().is_some_and(|l| l.changed)
}

const TABLE: core::alloc::Layout = match core::alloc::Layout::from_size_align(4096, 4096) {
    Ok(l) => l,
    Err(_) => panic!(),
};

/// A zeroed page-table page from the kernel heap (identity mapped), or null.
unsafe fn new_table() -> *mut u64 {
    // SAFETY: non-zero-sized layout.
    unsafe { alloc::alloc::alloc_zeroed(TABLE) as *mut u64 }
}

/// Free a user page-table tree from [`build_pml4`]: every table below the user half, never the
/// shared kernel entry (511). The CPU must not be using it (run_user switches back to the kernel CR3).
pub(crate) fn free_pml4(pml4: u64) {
    unsafe fn level(t: *mut u64, depth: u32) {
        // SAFETY: `t` is a table from new_table; present non-leaf entries point at tables we allocated.
        unsafe {
            if depth < 3 {
                for i in 0..512 {
                    let e = *t.add(i);
                    if e & P != 0 {
                        level((e & 0x000f_ffff_ffff_f000) as *mut u64, depth + 1);
                    }
                }
            }
            alloc::alloc::dealloc(t as *mut u8, TABLE);
        }
    }
    if pml4 == 0 {
        return;
    }
    let t = pml4 as *mut u64;
    // SAFETY: as above; entry 511 is the kernel's, cleared so it is not walked.
    unsafe {
        *t.add(511) = 0;
        level(t, 0);
    }
}

fn read_cr3() -> u64 {
    let v: u64;
    // SAFETY: reading CR3 is side-effect free.
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack)) };
    v
}

pub(crate) fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: rdtsc only reads the timestamp counter.
    unsafe { asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack)) };
    (u64::from(hi) << 32) | u64::from(lo)
}

/// Nanoseconds per TSC tick in 32.32 fixed point, measured against the kernel clock over 20 ms.
fn tsc_mult() -> u64 {
    // SAFETY: monotonic microsecond clock.
    let (u0, t0) = (unsafe { time_us() }, rdtsc());
    let mut u1 = u0;
    while u1 - u0 < 20_000 {
        // SAFETY: as above.
        u1 = unsafe { time_us() };
    }
    let ticks = rdtsc().wrapping_sub(t0).max(1);
    ((((u1 - u0) * 1000) as u128) << 32).checked_div(ticks as u128).unwrap_or(1 << 32) as u64
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
            free_pml4(pml4 as u64);
            return 0;
        }
        pml4 as u64
    }
}

/// The leaf entry for `va` in `pml4` (0 when unmapped).
fn leaf_of(pml4: u64, va: u64) -> u64 {
    // SAFETY: tables come from kalloc (identity mapped); indices are masked to 0..512.
    unsafe {
        let mut t = pml4 as *const u64;
        for shift in [39u32, 30, 21] {
            let e = *t.add(((va >> shift) & 511) as usize);
            if e & P == 0 {
                return 0;
            }
            t = (e & 0x000f_ffff_ffff_f000) as *const u64;
        }
        *t.add(((va >> 12) & 511) as usize)
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

/// The Linux layer's view of the kernel network: the NIC-backed stack and its pump.
struct KLinNet;

impl loos_lin::sys::LinNet for KLinNet {
    fn stack(&mut self) -> Option<&mut loos_net::stack::Stack> {
        crate::sys::net_stack()
    }
    fn pump(&mut self, now_ms: u64) {
        crate::sys::loos_net_poll(now_ms);
    }
    fn dns_server(&self) -> Option<[u8; 4]> {
        crate::sys::dns_server()
    }
}

struct Lin {
    sys: LinSystem<MemFs>,
    /// LoOS Prism: macOS (Mach-O) processes living in `sys`.
    prism: loos_prism::Prism,
    pml4: BTreeMap<Pid, (u64, u64)>,
    last: Pid,
    desk: alloc::rc::Rc<core::cell::RefCell<loos_win32k::Win32k>>,
    /// LoOS windows showing Wayland surfaces, keyed by (connection, wl_surface id).
    wayland_windows: BTreeMap<(usize, u32), Hwnd>,
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
    /// The console panel is shown (until the first window appears; F12 toggles).
    show_console: bool,
    /// Heartbeat: when the run started, when the last status line went out, syscalls so far.
    started_us: u64,
    beat_us: u64,
    syscalls: u64,
    /// Syscalls in the current heartbeat window, by (thread, number).
    hot: BTreeMap<(Pid, u64), u32>,
    /// TSC cycles per phase since the last heartbeat: loop, user, syscall, fault, present; then fault count.
    prof: [u64; 6],
    /// TSC cycles spent handling each syscall number since the last heartbeat.
    sys_cycles: BTreeMap<u64, u64>,
    /// Wayland frames shown so far.
    frames_shown: u64,
    presents: u64,
    /// Input diagnostics: raw (mouse packets, keys) and what reached Wayland clients (pointer, keys).
    input_stats: [u64; 4],
}

impl Lin {
    fn flush_console(&mut self) {
        if !self.sys.diag.is_empty() {
            say(&core::mem::take(&mut self.sys.diag));
        }
        let out = self.sys.console_take();
        for &b in &out {
            // SAFETY: port I/O.
            unsafe { serial_putc(b as i8) };
        }
        if !out.is_empty() {
            console_log(&out);
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
        if let Some((_, old)) = self.pml4.insert(pid, (ver, cr3)) {
            free_pml4(old);
        }
        // Tables of processes that are gone.
        let sys = &self.sys;
        let dead: Vec<Pid> = self.pml4.keys().copied().filter(|p| sys.process(*p).is_none()).collect();
        for d in dead {
            free_pml4(self.pml4.remove(&d).unwrap().1);
        }
        cr3
    }

    fn pick(&mut self) -> Option<Pid> {
        let ids = self.sys.thread_ids();
        let runnable = |s: &LinSystem<MemFs>, t: Pid| s.thread(t).is_some_and(|t| t.state == TState::Runnable);
        ids.iter().copied().find(|&t| t > self.last && runnable(&self.sys, t)).or_else(|| ids.iter().copied().find(|&t| runnable(&self.sys, t)))
    }

    fn present_wayland(&mut self) {
        for (conn, frame) in self.sys.take_wayland_frames(self.back.is_null()) {
            let key = (conn, frame.surface);
            let hwnd = if let Some(&h) = self.wayland_windows.get(&key) {
                h
            } else {
                let title = core::str::from_utf8(if frame.title.is_empty() { &frame.app_id } else { &frame.title }).unwrap_or("").to_string();
                let style = if frame.toplevel { WS_OVERLAPPEDWINDOW | WS_VISIBLE } else { WS_VISIBLE };
                let n = self.wayland_windows.len() as i32;
                let outer = adjust_window_rect(style, false, Rect { left: 0, top: 0, right: frame.width as i32, bottom: frame.height as i32 });
                let Ok(h) = self.desk.borrow_mut().create_window_ex(0, "LooSLinux", if title.is_empty() { "Linux application" } else { &title }, style, (40 + 24 * n).min(self.screen.0 - (outer.right - outer.left)).max(0), (40 + 24 * n).min(self.screen.1 - (outer.bottom - outer.top)).max(0), outer.right - outer.left, outer.bottom - outer.top, 0, 0, 0) else { continue };
                if self.wayland_windows.is_empty() {
                    self.show_console = false; // the app is up: give it the screen (F12 brings the log back)
                }
                self.wayland_windows.insert(key, h);
                if frame.toplevel {
                    // A new top-level window gets the focus, like on any desktop.
                    self.desk.borrow_mut().set_active_window(h);
                    let surface = frame.surface;
                    self.sys.wayland_input(conn, |w| w.keyboard_focus(surface));
                }
                let o = self.desk.borrow().client_to_screen(h, Point { x: 0, y: 0 }).unwrap_or_default();
                say(&alloc::format!("lin: wayland window \"{title}\" {}x{} client at {},{}\n", frame.width, frame.height, o.x, o.y));
                h
            };
            {
                // Follow client-side resizes: keep the client area equal to the buffer.
                let mut d = self.desk.borrow_mut();
                if let (Ok(c), Ok(w)) = (d.get_client_rect(hwnd), d.get_window_rect(hwnd)) {
                    let (dw, dh) = (frame.width as i32 - (c.right - c.left), frame.height as i32 - (c.bottom - c.top));
                    if dw != 0 || dh != 0 {
                        let (nw, nh) = (w.right - w.left + dw, w.bottom - w.top + dh);
                        // Keep the grown window on screen (a maximized client fills the work area).
                        let (x, y) = (w.left.min(self.screen.0 - nw).max(0), w.top.min(self.screen.1 - nh).max(0));
                        d.move_window(hwnd, x, y, nw, nh).ok();
                    }
                }
                d.import_window_pixels(hwnd, frame.width as i32, frame.height as i32, &frame.pixels);
            }
            self.dirty = true;
            self.frames_shown += 1;
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
                self.input_stats[0] += 1;
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
                self.input_stats[1] += 1;
                self.key(ev.key, ev.down != 0);
            }
        }
        if any {
            self.route_window_messages();
        }
    }

    fn key(&mut self, code: u32, down: bool) {
        if code == 88 {
            if down {
                self.show_console = !self.show_console; // F12: console panel on/off
                self.dirty = true;
            }
            return;
        }
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
        self.input_stats[3] += 1;
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
                if m.message != WM_PAINT {
                    self.input_stats[2] += 1;
                }
                match m.message {
                    WM_MOUSEMOVE => self.sys.wayland_input(conn, |w| w.pointer_motion(surface, x, y, ms)),
                    _ if button.is_some() => self.sys.wayland_input(conn, |w| {
                        w.pointer_motion(surface, x, y, ms);
                        w.pointer_button(button.unwrap(), pressed, ms);
                    }),
                    // The pixels come from the client's buffer: nothing to repaint, and an unvalidated
                    // window would get WM_PAINT forever.
                    WM_PAINT => {
                        self.desk.borrow_mut().validate_rect(h).ok();
                    }
                    WM_SYSCOMMAND if m.wparam & 0xfff0 == SC_CLOSE => self.sys.wayland_input(conn, |w| w.close(surface)),
                    _ => {}
                }
            }
        }
    }

    /// One status line every 10 s, so a long start-up is visibly alive (serial and on-screen console).
    fn heartbeat(&mut self, now: u64) {
        let (procs, threads) = self.sys.counts();
        say(&alloc::format!(
            "lin: {}s running, {} syscalls, {} processes, {} threads, {} windows, {} frames, {} screen updates, input mouse/keys {}/{} -> app pointer/keys {}/{}\n",
            (now - self.started_us) / 1_000_000,
            self.syscalls,
            procs,
            threads,
            self.wayland_windows.len(),
            self.frames_shown,
            self.presents,
            self.input_stats[0],
            self.input_stats[1],
            self.input_stats[2],
            self.input_stats[3]
        ));
        // The busiest thread/syscall pairs of the last 10 s: shows what a busy-but-stuck program spins on.
        let mut hot: Vec<((Pid, u64), u32)> = core::mem::take(&mut self.hot).into_iter().collect();
        hot.sort_by(|a, b| b.1.cmp(&a.1));
        let top: Vec<String> = hot.iter().take(5).map(|((t, nr), n)| alloc::format!("{}x {}#{} [{}]", n, loos_lin::syscalls_generated::syscall_name(*nr), nr, t)).collect();
        say(&alloc::format!("lin:   busiest: {}\n", top.join(", ")));
        // Live processes by program: a growing count shows what keeps getting started.
        let mut names: BTreeMap<String, Vec<Pid>> = BTreeMap::new();
        for p in self.sys.live_processes() {
            let n = p.argv.iter().rev().find(|a| a.contains(".exe")).or(p.argv.first()).map(|a| a.rsplit(['/', '\\']).next().unwrap_or(a)).unwrap_or("?");
            names.entry(n.into()).or_default().push(p.pid);
        }
        let names: Vec<String> = names.iter().map(|(n, ps)| alloc::format!("{n}{ps:?}")).collect();
        say(&alloc::format!("lin:   live: {}\n", names.join(" ")));
        // Where syscall time goes (share of all syscall time), costliest first.
        let mut cost: Vec<(u64, u64)> = core::mem::take(&mut self.sys_cycles).into_iter().collect();
        cost.sort_by(|a, b| b.1.cmp(&a.1));
        let total = cost.iter().map(|c| c.1).sum::<u64>().max(1);
        let top: Vec<String> = cost.iter().take(6).map(|(nr, c)| alloc::format!("{}#{} {}%", loos_lin::syscalls_generated::syscall_name(*nr), nr, c * 100 / total)).collect();
        say(&alloc::format!("lin:   syscall time: {}\n", top.join(", ")));
        let p = core::mem::take(&mut self.prof);
        let all = p[..5].iter().sum::<u64>().max(1);
        let pct = |i: usize| p[i] * 100 / all;
        say(&alloc::format!("lin:   time: loop {}%, user {}%, syscall {}%, fault {}% ({} faults), present {}%\n", pct(0), pct(1), pct(2), pct(3), p[5], pct(4)));
        // Every process's share, so a quiet process (a renderer waiting on something) is visible too.
        let mut per: BTreeMap<Pid, u32> = BTreeMap::new();
        for ((t, _), n) in &hot {
            if let Some(th) = self.sys.thread(*t) {
                *per.entry(th.pid).or_default() += n;
            }
        }
        let procs: Vec<String> = per.iter().map(|(pid, n)| {
            let kind = self.sys.process(*pid).and_then(|p| p.argv.iter().find_map(|a| a.strip_prefix("--type=")).map(String::from).or_else(|| p.argv.iter().rev().find(|a| a.contains(".exe")).or(p.argv.first()).map(|a| a.rsplit(['/', '\\']).next().unwrap_or(a).into()))).unwrap_or_else(|| "?".into());
            alloc::format!("{pid}:{kind}={n}")
        }).collect();
        say(&alloc::format!("lin:   per process: {}\n", procs.join(" ")));
        for (pid, n) in &per {
            if *n < 50 {
                let recent = self.sys.recent_syscalls(*pid);
                let lines: Vec<&str> = recent.lines().collect();
                for l in &lines[lines.len().saturating_sub(3)..] {
                    say(&alloc::format!("{l}\n"));
                }
            }
        }
        // Threads of the busiest process that made no syscall in these 10 s, with what they sleep in: a
        // renderer or compositor thread stuck on a lost wake-up shows here while the main thread keeps running.
        if let Some(pid) = hot.first().and_then(|((t, _), _)| self.sys.thread(*t)).map(|t| t.pid) {
            let quiet: Vec<String> = self.sys.threads().filter(|t| t.pid == pid && !hot.iter().any(|((h, _), _)| *h == t.tid)).map(|t| {
                let last = t.recent.back().map_or(String::from("-"), |(nr, a, _)| alloc::format!("{}({:#x},{:#x})", loos_lin::syscalls_generated::syscall_name(*nr), a[0], a[1]));
                let st = if matches!(t.state, loos_lin::sys::TState::Blocked { .. }) { "blocked" } else { "run" };
                alloc::format!("{}:{}:{st}:{last}", t.tid, t.name)
            }).collect();
            say(&alloc::format!("lin:   quiet threads of {pid}: {}\n", quiet.join(" ")));
        }
        // Who the busiest thread is and its last few calls (enough to tell a loop's shape).
        if let Some(((t, _), _)) = hot.first() {
            if let Some(p) = self.sys.thread(*t).and_then(|th| self.sys.process(th.pid)) {
                say(&alloc::format!("lin:   [{t}] is {}\n", p.argv.join(" ").chars().take(160).collect::<String>()));
            }
            let recent = self.sys.recent_syscalls(*t);
            let lines: Vec<&str> = recent.lines().collect();
            for l in &lines[lines.len().saturating_sub(6)..] {
                say(&alloc::format!("{l}\n"));
            }
        }
        // What the busiest threads' processes last saw as ready (a poll loop spinning on one fd shows here).
        let mut shown = Vec::new();
        for ((t, _), _) in hot.iter().take(3) {
            let Some(pid) = self.sys.thread(*t).map(|t| t.pid) else { continue };
            if shown.contains(&pid) {
                continue;
            }
            shown.push(pid);
            if let Some(r) = self.sys.last_ready.get(&pid) {
                say(&alloc::format!("lin:   pid {pid} last ready: {r}\n"));
            }
        }
    }

    /// Compose the desktop onto the screen (at most ~60 times a second).
    fn show(&mut self) {
        // SAFETY: monotonic clock.
        let now = unsafe { time_us() };
        // A console-only change redraws at most 4 times a second (window changes still get ~60).
        if self.show_console && console_changed() && now.saturating_sub(self.last_present_us) >= 250_000 {
            self.dirty = true;
        }
        if !self.dirty || self.back.is_null() || now.saturating_sub(self.last_present_us) < 16_000 {
            return;
        }
        // SAFETY: the kernel owns a writable `w * h` backbuffer for the whole run.
        let frame = unsafe { core::slice::from_raw_parts_mut(self.back, (self.screen.0 * self.screen.1) as usize) };
        self.desk.borrow_mut().compose(frame);
        if self.show_console {
            draw_console(frame, self.screen);
        }
        if self.input {
            draw_cursor(frame, self.screen, self.cursor);
        }
        // SAFETY: plain kernel call; gui_vid was set before loos_lin_boot.
        unsafe { gui_present() };
        self.presents += 1;
        // The frames are on screen now: Wayland clients may draw their next one.
        self.sys.wayland_presented();
        self.dirty = false;
        self.last_present_us = now;
    }

    /// Run until process `main` ends. Returns its wait status (exit code << 8, or signal number).
    fn run(&mut self, main: Pid) -> i32 {
        // The Linux clock runs on the TSC (the vDSO reads the same counter): calibrate it once.
        let mult = tsc_mult();
        let mut clock = rdtsc();
        let mut status = None;
        let mut idle_spins = 0u32;
        let mut last_io_us = 0u64;
        let mut frame: UFrame = UFrame { gpr: [0; 16], vector: 0, error: 0, rip: 0, cs: loos_lin::sys::USER_CS as u64, rflags: 2, rsp: 0, ss: loos_lin::sys::USER_SS as u64, cr3: 0, fx: [0; 512] };
        // Default x87/SSE state: FCW=0x37f, MXCSR=0x1f80.
        frame.fx[0] = 0x7f;
        frame.fx[1] = 0x03;
        frame.fx[24..28].copy_from_slice(&0x1f80u32.to_le_bytes());
        while status.is_none() {
            let t_loop = rdtsc();
            // SAFETY: monotonic microsecond clock.
            let now = unsafe { time_us() };
            let t = rdtsc();
            self.sys.advance(((t.wrapping_sub(clock) as u128 * mult as u128) >> 32) as u64);
            clock = t;
            // The NIC and PS/2 are hardware reads (VM exits), and poll_net re-checks every thread blocked in
            // poll(): once a millisecond is plenty. Doing it per syscall made a hundred idle poll() waiters
            // spin and cost most of the run time.
            if now.wrapping_sub(last_io_us) >= 1000 {
                last_io_us = now;
                if self.sys.net_attached() {
                    self.sys.poll_net(now / 1000);
                }
                self.poll_input();
            }
            if now.saturating_sub(self.beat_us) >= 10_000_000 {
                self.beat_us = now;
                self.heartbeat(now);
            }
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
            if self.prism.is_arm(pid) {
                // Apple Silicon code never enters ring 3: the interpreter runs it against the same address space.
                idle_spins = 0;
                if let Some(Sys::ProcessExit(st)) = self.prism.run_arm(&mut self.sys, tid, 200_000) {
                    if pid == main {
                        status = Some(st);
                    }
                    self.prism.reap(&self.sys);
                }
                self.present_wayland();
                self.flush_console();
                if status.is_none() && self.sys.process(main).is_none() {
                    status = Some(0);
                }
                continue;
            }
            if self.prism.owns(pid) {
                self.prism.resume(&mut self.sys, tid);
                // Finishing a call can exec (posix_spawn's child): the process may now be interpreted.
                if self.prism.is_arm(pid) || self.sys.thread(tid).is_none() {
                    continue;
                }
            }
            let prism = self.prism.owns(pid);
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
            frame.fx = *self.sys.thread(tid).unwrap().fx;
            // vDSO clock: now_ns at TSC `clock`; advance() above uses the same reference, so it is continuous.
            self.sys.set_clock_page(clock, mult);
            // SAFETY: FS base MSR; run_user enters ring 3 with a fully initialised frame and returns on trap.
            let vector = unsafe {
                wrmsr(MSR_FS_BASE, r.fs_base);
                // arch_prctl(ARCH_SET_GS) / Darwin's TSD base (Wine keeps the Windows TEB there): becomes GS on the
                // swapgs into ring 3.
                wrmsr(MSR_KERNEL_GS_BASE, r.gs_base);
                let t_in = rdtsc();
                self.prof[0] += t_in.wrapping_sub(t_loop);
                let v = run_user(&mut frame);
                self.prof[1] += rdtsc().wrapping_sub(t_in);
                v
            };
            let t_out = rdtsc();
            let g = &frame.gpr;
            {
                let t = self.sys.thread_mut(tid).unwrap();
                let r = &mut t.regs;
                [r.rax, r.rcx, r.rdx, r.rbx, _, r.rbp, r.rsi, r.rdi, r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15] = *g;
                r.rip = frame.rip;
                r.rsp = frame.rsp;
                r.rflags = frame.rflags;
                *t.fx = frame.fx;
            }
            if vector == 0x100 {
                idle_spins = 0;
                let (nr, a) = (g[0], [g[7], g[6], g[2], g[10], g[8], g[9]]);
                self.syscalls += 1;
                *self.hot.entry((tid, nr)).or_insert(0) += 1;
                let r = if prism { self.prism.syscall(&mut self.sys, tid, nr, a).unwrap_or(Sys::Ret(0)) } else { self.sys.syscall(tid, nr, a) };
                // A Linux program exec'ing a Mach-O: Prism loads it into the process.
                let r = if r == Sys::ExecForeign { self.prism.exec_foreign(&mut self.sys) } else { r };
                match r {
                    Sys::ProcessExit(st) if pid == main => status = Some(st),
                    _ => {}
                }
                if prism && matches!(r, Sys::ProcessExit(_)) {
                    self.prism.reap(&self.sys);
                }
                let t_sys = rdtsc();
                self.prof[2] += t_sys.wrapping_sub(t_out);
                *self.sys_cycles.entry(nr).or_default() += t_sys.wrapping_sub(t_out);
                // New Wayland frames only arrive through socket I/O (the heartbeat and idle paths present too).
                if !LinSystem::<MemFs>::io_neutral(nr) {
                    self.present_wayland();
                }
                self.prof[4] += rdtsc().wrapping_sub(t_sys);
            } else {
                self.prof[5] += 1;
                let cr2 = read_cr2();
                let write = frame.error & 2 != 0;
                let fixed = vector == 14
                    && match self.sys.process_mut(pid).and_then(|p| p.mem.handle_fault_kind(cr2, write, frame.error & 0x10 != 0)) {
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
                    dec(vector);
                    say(" rip ");
                    dec(frame.rip);
                    say(" cr2 ");
                    dec(cr2);
                    if let Some((name, off)) = self.sys.process(pid).and_then(|p| p.mem.locate(frame.rip)) {
                        say(" in ");
                        say(name);
                        say(" +");
                        // SAFETY: serial output.
                        hex(off);
                    }
                    // The faulting instruction's bytes (decode with e.g. `xxd -r -p | objdump -D -b binary -mi386:x86-64`).
                    if let Some(p) = self.sys.process_mut(pid) {
                        if let Ok(code) = p.mem.read_vec(frame.rip, 16) {
                            say(" code ");
                            say(&code.iter().map(|b| alloc::format!("{b:02x}")).collect::<String>());
                        }
                    }
                    say(" rsp ");
                    hex(frame.rsp);
                    if let Some(p) = self.sys.process_mut(pid) {
                        let top: Vec<String> = (0..4).filter_map(|i| p.mem.read_u64(frame.rsp + 8 * i).ok()).map(|w| alloc::format!("{w:#x}")).collect();
                        say(&alloc::format!(" [rsp] {}", top.join(" ")));
                        // DEBUG-FF
                        let probe: Vec<String> = [0x14006da18u64, 0x14006d000, 0x14006d008, 0x14006c000, 0x14006e000].iter().map(|&a| alloc::format!("{a:#x}={:#x}", p.mem.read_u64(a).unwrap_or(0xdead))).collect();
                        say(&alloc::format!(" probe {} reg {:?}", probe.join(" "), p.mem.locate(0x14006da18)));
                    }
                    say("\n");
                    // The code addresses on the stack, nearest first: a rough backtrace.
                    if let Some(p) = self.sys.process_mut(pid) {
                        let (mut shown, mut last) = (0, 0);
                        for i in 0..4096u64 {
                            let Ok(w) = p.mem.read_u64(frame.rsp + 8 * i) else { break };
                            if w == last {
                                continue;
                            }
                            if let Some((name, off)) = p.mem.locate(w) {
                                last = w;
                                say("lin:   at ");
                                say(name);
                                say(" +");
                                // SAFETY: serial output.
                                hex(off);
                                say("\n");
                                shown += 1;
                                if shown == 24 {
                                    break;
                                }
                            }
                        }
                    }
                    // Page-table entries that disagree with the process's memory (a stale page would show here).
                    if let (Some(p), Some(&(_, cr3))) = (self.sys.process(pid), self.pml4.get(&pid)) {
                        let mut bad = Vec::new();
                        p.mem.for_each_page(|va, pa, _| {
                            let e = leaf_of(cr3, va);
                            if e & P != 0 && e & 0x000f_ffff_ffff_f000 != pa && bad.len() < 8 {
                                bad.push(alloc::format!("{va:#x}"));
                            }
                        });
                        say(&alloc::format!("lin:   stale ptes: {}\n", bad.join(" ")));
                    }
                    let recent = self.sys.recent_syscalls(tid);
                    say(&recent);
                    self.sys.fault(tid, vector, frame.error, cr2);
                }
            }
            if vector != 0x100 {
                self.prof[3] += rdtsc().wrapping_sub(t_out);
            }
            let t_fl = rdtsc();
            self.flush_console();
            self.prof[4] += rdtsc().wrapping_sub(t_fl);
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
        let mut n = 0;
        while *cmd.add(n) != 0 && *cmd.add(n) != b' ' {
            n += 1;
        }
        // The boot module is never freed: the root filesystem borrows file contents from it.
        (if initrd.is_null() { &[][..] } else { core::slice::from_raw_parts::<'static, u8>(initrd, len) }, String::from_utf8_lossy(core::slice::from_raw_parts(cmd, n)).into_owned())
    };
    let fs = rootfs(tar);
    // The kernel command line ends at a space, so arguments spell space/comma as %20 / %2C.
    let mut argv: Vec<String> = cmd.split(',').map(|a| a.replace("%20", " ").replace("%2C", ",").replace("%25", "%")).collect();
    let fs_args = argv.first().filter(|a| a.starts_with('@')).map(|a| a[1..].to_string());
    // loos.lin=@/file: one entry per line (strace, VAR=value..., program, arguments) for command lines
    // longer than the kernel's.
    if let Some(path) = fs_args {
        match fs.file(&path) {
            Some(text) => argv = String::from_utf8_lossy(text).lines().filter(|l| !l.is_empty()).map(String::from).collect(),
            None => {
                say("lin: argument file not in the rootfs\n");
                return -1;
            }
        }
    }
    session(fs, argv, back, width, height).0
}

/// The compat runtime's root file system from the boot module (`initrd`): the Linux root, Wine's prefix in
/// /home/.wine and macOS bundles in /Applications. Contents stay borrowed from the module until written.
pub(crate) fn rootfs(tar: &'static [u8]) -> MemFs {
    let mut fs = MemFs::new();
    for d in ["/bin", "/usr/bin", "/lib", "/lib64", "/etc", "/tmp", "/dev", "/proc", "/home", "/run", "/run/user", "/run/user/1000"] {
        fs.add_dir(d);
    }
    let files = fs.load_tar_static(tar);
    // One home (/home) for Linux, Wine (its prefix is /home/.wine) and macOS programs; macOS ones expect the
    // standard ~/Library folders to exist (NSUserDefaults, caches, Application Support).
    for d in ["/home/Library/Preferences", "/home/Library/Application Support", "/home/Library/Caches", "/home/Documents", "/home/Downloads", "/home/Desktop"] {
        if fs.lstat(d).is_err() {
            fs.add_dir(d);
        }
    }
    say("lin: rootfs ");
    dec(files as u64);
    say(" entries\n");
    fs
}

/// Run one program (and everything it starts) in the compat runtime on `fs` until it exits; `argv` is
/// `[strace[:nrs]] [VAR=value...] program args...`. Mach-O programs and bundles go to Prism, the rest to the
/// Linux layer (Wine is a Linux program). Returns the wait status (negative when it could not start) and the
/// file system, so what the program wrote stays for the next one.
pub(crate) fn session(fs: MemFs, mut argv: Vec<String>, back: *mut u32, width: i32, height: i32) -> (i32, MemFs) {
    // SAFETY: (re)initialises the user-mode entry state; idempotent.
    unsafe { user_init() };
    if argv.is_empty() {
        return (-1, fs);
    }
    // `strace` traces every syscall, `strace:46,47` only those numbers.
    let trace_arg = argv.first().filter(|a| *a == "strace" || a.starts_with("strace:") || a.starts_with("strace@")).cloned();
    let trace = trace_arg.is_some();
    if trace {
        argv.remove(0);
    }
    // `strace[:nrs][@text]`: `@text` limits tracing to the first process whose argv contains `text`.
    let trace_proc = trace_arg.as_ref().and_then(|a| a.split_once('@').map(|(_, t)| String::from(t)));
    let trace_arg = trace_arg.map(|a| String::from(a.split('@').next().unwrap()));
    let trace_only = trace_arg.and_then(|a| a.strip_prefix("strace:").map(|l| l.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse::<u64>().ok()).collect::<Vec<u64>>()));
    let mut sys = LinSystem::new(fs);
    sys.trace = trace;
    sys.trace_only = trace_only;
    sys.trace_proc = trace_proc;
    if !back.is_null() {
        sys.wayland_screen = (width.max(1) as u32, height.max(1) as u32);
        // A maximized window's client area: the screen minus the frame the window manager adds.
        let f = adjust_window_rect(WS_OVERLAPPEDWINDOW | WS_VISIBLE, false, Rect { left: 0, top: 0, right: 0, bottom: 0 });
        sys.wayland_work_area = ((width - (f.right - f.left)).max(1) as u32, (height - (f.bottom - f.top)).max(1) as u32);
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
    // Leading `VAR=value` entries are extra environment: FOO=1,/usr/bin/app,arg
    while argv.first().is_some_and(|a| a.contains('=') && !a.starts_with('/')) {
        let var = argv.remove(0);
        let name = &var[..=var.find('=').unwrap_or(0)];
        // An entry replaces the default of the same name, so getenv and environ agree.
        env.retain(|e| !e.starts_with(name));
        env.push(var);
    }
    if argv.is_empty() {
        return (-1, core::mem::take(&mut sys.fs));
    }
    // Mach-O files and .app bundles go to LoOS Prism; everything else is a Linux program.
    let mut prism = loos_prism::Prism::new();
    let spawned = if loos_prism::recognizes(&mut sys.fs, &argv[0]) {
        say("prism: starting macOS program ");
        say(&argv[0]);
        say("\n");
        let r = prism.spawn(&mut sys, &argv[0], &argv, &env);
        let d = core::mem::take(&mut sys.diag);
        say(&d);
        r
    } else {
        sys.spawn(&argv[0], &argv, &env)
    };
    let pid = match spawned {
        Ok(p) => p,
        Err(e) => {
            say("lin: cannot execute (errno ");
            dec(e as u64);
            say(")\n");
            return (-1, core::mem::take(&mut sys.fs));
        }
    };
    let mut lin = Lin { sys, prism, pml4: BTreeMap::new(), last: 0, desk: desk.clone(), wayland_windows: BTreeMap::new(), back, screen: (screen_w, screen_h), dirty: false, last_present_us: 0, input: !back.is_null(), cursor: (screen_w / 2, screen_h / 2), buttons: 0, mods: 0, caps: false, show_console: true, started_us: unsafe { time_us() }, beat_us: unsafe { time_us() }, syscalls: 0, hot: BTreeMap::new(), prof: [0; 6], sys_cycles: BTreeMap::new(), frames_shown: 0, presents: 0, input_stats: [0; 4] };
    if lin.input {
        // SAFETY: polled i8042 setup, as the desktop does.
        unsafe { ps2_init() };
    }
    let st = lin.run(pid);
    if !lin.sys.missing.is_empty() {
        say("lin: unimplemented syscalls:");
        for (nr, n) in &lin.sys.missing {
            say(" ");
            dec(*nr);
            say("x");
            dec(*n as u64);
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
    dec(green as u64);
    say("\n");
    // SAFETY: NUL-terminated literal.
    unsafe { serial_puts(b"lin: program finished, status \0".as_ptr()) };
    dec(st as u32 as u64);
    say("\n");
    (st, core::mem::take(&mut lin.sys.fs))
}
