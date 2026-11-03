//! user32/gdi32 entry points: decodes the Win64 calling convention (four registers plus stack arguments) and
//! forwards to `loos-win32k`. Window procedures registered by the program are reached through the kernel's upcall hook.

use crate::{Process, Ret};
use alloc::string::String;
use alloc::vec::Vec;
use loos_lin::mem::UserMem;
use loos_win32k::{GuestMem, Proc, Win32k, WndClass, SVC_BASE, WOULD_BLOCK};

/// First syscall number of the user32/gdi32 range.
pub const GUI_BASE: u32 = 0x100;
/// Syscall number the callback-return stub uses (window procedure finished).
pub const CB_RET: u32 = 0x0ff;
const EXPLICIT: u32 = 0x200;

/// Win32 name -> win32k service offset (arguments already match the service ABI).
const DIRECT: &[(&str, u32)] = &[
    ("ShowWindow", 2), ("DestroyWindow", 3), ("GetMessageW", 4), ("GetMessageA", 4), ("PeekMessageW", 5), ("PeekMessageA", 5),
    ("TranslateMessage", 6), ("DispatchMessageW", 7), ("DispatchMessageA", 7), ("PostQuitMessage", 8),
    ("DefWindowProcW", 9), ("DefWindowProcA", 9), ("PostMessageW", 10), ("PostMessageA", 10), ("SendMessageW", 11),
    ("SendMessageA", 11), ("BeginPaint", 12), ("EndPaint", 13), ("GetDC", 14), ("ReleaseDC", 15), ("FillRect", 16),
    ("TextOutW", 17), ("SetTextColor", 18), ("SetBkColor", 19), ("SetBkMode", 20), ("CreateSolidBrush", 21),
    ("CreatePen", 22), ("SelectObject", 23), ("DeleteObject", 24), ("InvalidateRect", 25), ("UpdateWindow", 26),
    ("GetClientRect", 27), ("GetWindowRect", 28), ("SetTimer", 29), ("KillTimer", 30), ("SetWindowTextW", 32),
    ("MoveWindow", 33), ("LineTo", 34), ("MoveToEx", 35), ("Rectangle", 36), ("GetStockObject", 37), ("SetFocus", 38),
];
/// Names handled by explicit decoders below.
const SPECIAL: &[&str] = &[
    "RegisterClassW", "RegisterClassExW", "RegisterClassA", "RegisterClassExA", "CreateWindowExW", "CreateWindowExA",
    "GetSystemMetrics", "LoadCursorW", "LoadCursorA", "LoadIconW", "LoadIconA", "TextOutA", "SetWindowTextA",
    "MessageBoxW", "MessageBoxA", "GetModuleHandleW", "SetCursor", "GetKeyState", "GetAsyncKeyState", "ShowCursor",
    "SetProcessDPIAware", "IsWindow", "GetForegroundWindow", "CreateMenu", "CreatePopupMenu", "AppendMenuW", "SetMenu", "GetSubMenu",
    "DestroyMenu", "DrawMenuBar", "EnableMenuItem", "CheckMenuItem", "GetMenu", "GetWindowTextW", "GetWindowTextLengthW",
];

/// Syscall number for a user32/gdi32 import.
/// Import name for a user32/gdi32 syscall number (tracing).
pub fn name_of(nr: u32) -> &'static str {
    if (GUI_BASE..GUI_BASE + 64).contains(&nr) {
        return DIRECT.iter().find(|(_, o)| GUI_BASE + o == nr).map_or("?", |(n, _)| n);
    }
    SPECIAL.get(nr.wrapping_sub(EXPLICIT) as usize).copied().unwrap_or("?")
}

/// Syscall number for an imported function name.
pub fn lookup(name: &str) -> Option<u32> {
    if let Some((_, o)) = DIRECT.iter().find(|(n, _)| *n == name) {
        return Some(GUI_BASE + o);
    }
    SPECIAL.iter().position(|n| *n == name).map(|i| EXPLICIT + i as u32)
}

struct Gm<'a>(&'a mut UserMem);
impl GuestMem for Gm<'_> {
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> bool {
        self.0.read(addr, buf).is_ok()
    }
    fn write(&mut self, addr: u64, data: &[u8]) -> bool {
        self.0.write(addr, data).is_ok()
    }
}

fn wide(mem: &mut UserMem, a: u64) -> Option<String> {
    if a < 0x1_0000 {
        return None;
    }
    let mut v = Vec::new();
    for i in 0..4096u64 {
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
fn narrow(mem: &mut UserMem, a: u64) -> Option<String> {
    if a < 0x1_0000 { None } else { mem.read_cstr(a, 4096).ok() }
}
fn to_utf16(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    v.extend_from_slice(&[0, 0]);
    v
}

impl Process {
    fn menu_items(&self, h: u64) -> Vec<loos_win32k::MenuItem> {
        let Some(m) = self.menus.get(&h) else { return Vec::new() };
        m.iter()
            .map(|(flags, id, text)| {
                // '&' marks the mnemonic; a tab separates the accelerator hint (kept as part of the label).
                let label = text.replace('&', "").replace('\t', "   ");
                if flags & 0x10 != 0 { loos_win32k::MenuItem::popup(&label, self.menu_items(*id)) } else { loos_win32k::MenuItem::item(*id as u32, &label) }
            })
            .collect()
    }

    fn arg(&mut self, rsp: u64, n: usize) -> u64 {
        self.mem.read_u64(rsp + 0x28 + 8 * (n as u64 - 5)).unwrap_or(0)
    }

    /// Scratch buffer in the guest for services that take wide strings (ANSI entry points convert into it).
    fn scratch(&mut self, s: &str) -> u64 {
        let b = to_utf16(s);
        self.mem.map_anywhere(b.len() as u64, 3, &b, "scratch").unwrap_or(0)
    }

    /// Like [`Process::gui_syscall_inner`], logging each call and result when `trace` is set.
    pub fn gui_syscall(&mut self, nr: u32, a: [u64; 4], rsp: u64, now_ms: u64, w: &mut Win32k) -> Ret {
        let r = self.gui_syscall_inner(nr, a, rsp, now_ms, w);
        if self.trace && nr >= crate::thread::THR_BASE {
            let msg = alloc::format!("[trace] t{} nr {:#x} ({:#x}, {:#x}) = {:?}\n", self.cur, nr, a[0], a[1], r);
            self.console_write(msg.as_bytes());
        }
        if self.trace && nr < crate::k32::K32_BASE {
            let msg = alloc::format!("[trace] {}({:#x}, {:#x}, {:#x}, {:#x}) = {:?}\n", name_of(nr), a[0], a[1], a[2], a[3], r);
            self.console_write(msg.as_bytes());
        }
        r
    }

    /// Handles a user32/gdi32/shell syscall. `rsp` is the user stack pointer at the `syscall` (return address on top).
    fn gui_syscall_inner(&mut self, nr: u32, a: [u64; 4], rsp: u64, now_ms: u64, w: &mut Win32k) -> Ret {
        let v = |x: i64| Ret::Value(x as u64);
        w.set_current_thread(self.threads[self.cur].tid as u32);
        if (crate::gl::GL_BASE..crate::gl::GL_BASE + 0x100).contains(&nr) {
            self.cur_rsp = rsp;
            return self.gl_syscall(nr, a, rsp, w);
        }
        if nr >= crate::k32::K32_BASE {
            self.cur_rsp = rsp;
            return self.thread_syscall(nr, a, rsp, now_ms).or_else(|| self.ws_syscall(nr, a, rsp, now_ms)).or_else(|| self.k32_syscall(nr, a, rsp, now_ms)).unwrap_or(v(0));
        }
        // WM_SETTEXT / WM_GETTEXT / WM_GETTEXTLENGTH carry guest pointers: serve them here, against the window's text.
        if matches!(name_of(nr), "SendMessageW" | "SendMessageA" | "DefWindowProcW" | "DefWindowProcA") && matches!(a[1], 0xC | 0xD | 0xE) {
            let h = a[0] as u32;
            return match a[1] {
                0xC => {
                    let t = if name_of(nr).ends_with('W') { wide(&mut self.mem, a[3]) } else { narrow(&mut self.mem, a[3]) }.unwrap_or_default();
                    v(w.set_window_text(h, &t).is_ok() as i64)
                }
                0xE => v(w.get_window_text(h).map_or(0, |t| t.encode_utf16().count() as i64)),
                _ => {
                    let t = w.get_window_text(h).unwrap_or_default();
                    let n = (a[2] as usize).saturating_sub(1).min(t.encode_utf16().count());
                    let mut b: Vec<u8> = t.encode_utf16().take(n).flat_map(u16::to_le_bytes).collect();
                    b.extend_from_slice(&[0, 0]);
                    let _ = self.mem.write(a[3], &b);
                    v(n as i64)
                }
            };
        }
        if (GUI_BASE..GUI_BASE + 64).contains(&nr) {
            let a5 = self.arg(rsp, 5);
            let r = w.service(self.threads[self.cur].tid as u32, SVC_BASE + (nr - GUI_BASE), [a[0], a[1], a[2], a[3], a5, 0], &mut Gm(&mut self.mem));
            return if r == WOULD_BLOCK { Ret::Idle } else { v(r) };
        }
        let name = SPECIAL.get((nr - EXPLICIT) as usize).copied().unwrap_or("");
        match name {
            "RegisterClassW" | "RegisterClassExW" | "RegisterClassA" | "RegisterClassExA" => {
                let ex = name.contains("Ex");
                let Ok(b) = self.mem.read_vec(a[0], 72) else { return v(0) };
                let q = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
                let d = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
                let cls = if name.ends_with('W') { wide(&mut self.mem, q(64)) } else { narrow(&mut self.mem, q(64)) };
                let Some(cls) = cls else { return v(0) };
                let def = WndClass {
                    name: cls,
                    style: d(if ex { 4 } else { 0 }),
                    proc_: Proc::User(q(8)),
                    background: q(48) as u32,
                    wnd_extra: (d(20) as i32).clamp(0, 4096) as usize / 8 + 1,
                };
                v(w.register_class(def).map_or(0, i64::from))
            }
            "CreateWindowExW" | "CreateWindowExA" => {
                let wide_s = name.ends_with('W');
                let rd = |m: &mut UserMem, p: u64| if wide_s { wide(m, p) } else { narrow(m, p) };
                let Some(class) = rd(&mut self.mem, a[1]) else { return v(0) };
                let title = rd(&mut self.mem, a[2]).unwrap_or_default();
                let s: Vec<u64> = (5..=12).map(|n| self.arg(rsp, n)).collect();
                let dim = |x: u64, d: i32| if x as i32 == i32::MIN { d } else { x as i32 };
                let h = w.create_window_ex(a[0] as u32, &class, &title, a[3] as u32, dim(s[0], 80), dim(s[1], 60), dim(s[2], 640), dim(s[3], 480), s[4] as u32, s[5] as isize, s[7] as isize);
                v(h.map_or(0, i64::from))
            }
            "TextOutA" => {
                let (Some(t), n) = (narrow(&mut self.mem, a[3]), self.arg(rsp, 5)) else { return v(0) };
                let t: String = t.chars().take(n as usize).collect();
                let p = self.scratch(&t);
                let r = w.service(self.threads[self.cur].tid as u32, loos_win32k::SVC_TEXT_OUT, [a[0], a[1], a[2], p, t.chars().count() as u64, 0], &mut Gm(&mut self.mem));
                v(r)
            }
            "SetWindowTextA" => {
                let Some(t) = narrow(&mut self.mem, a[1]) else { return v(0) };
                let p = self.scratch(&t);
                v(w.service(self.threads[self.cur].tid as u32, loos_win32k::SVC_SET_WINDOW_TEXT, [a[0], p, 0, 0, 0, 0], &mut Gm(&mut self.mem)))
            }
            "MessageBoxW" | "MessageBoxA" => {
                let (t, c) = if name.ends_with('W') { (wide(&mut self.mem, a[1]), wide(&mut self.mem, a[2])) } else { (narrow(&mut self.mem, a[1]), narrow(&mut self.mem, a[2])) };
                let _ = w.message_box(a[0] as u32, &t.unwrap_or_default(), &c.unwrap_or_default(), a[3] as u32);
                v(1)
            }
            "GetSystemMetrics" => v(match a[0] as i32 {
                0 => 1024,
                1 => 768,
                4 => 19,
                5 | 6 => 1,
                _ => 0,
            }),
            "GetModuleHandleW" => {
                let h = if a[0] == 0 { self.base } else { wide(&mut self.mem, a[0]).and_then(|n| self.module_handle(&n)).unwrap_or(0) };
                v(h as i64)
            }
            "CreateMenu" | "CreatePopupMenu" => {
                let h = 0x5000 + self.menus.len() as u64;
                self.menus.insert(h, Vec::new());
                v(h as i64)
            }
            "AppendMenuW" => {
                // (hMenu, flags, id or submenu, text): MF_POPUP 0x10, MF_SEPARATOR 0x800
                let text = if a[1] & 0x800 != 0 { String::from("-") } else { wide(&mut self.mem, a[3]).unwrap_or_default() };
                match self.menus.get_mut(&a[0]) {
                    Some(m) => {
                        m.push((a[1] as u32, a[2], text));
                        v(1)
                    }
                    None => v(0),
                }
            }
            "GetSubMenu" => {
                let sub = self.menus.get(&a[0]).and_then(|m| m.iter().filter(|e| e.0 & 0x10 != 0).nth(a[1] as usize)).map(|e| e.1);
                v(sub.unwrap_or(0) as i64)
            }
            "SetMenu" => {
                let items = self.menu_items(a[1]);
                v(w.set_menu(a[0] as u32, items).is_ok() as i64)
            }
            "GetWindowTextLengthW" => v(w.get_window_text(a[0] as u32).map_or(0, |t| t.encode_utf16().count() as i64)),
            "GetWindowTextW" => {
                let t = w.get_window_text(a[0] as u32).unwrap_or_default();
                let n = (a[2] as usize).saturating_sub(1).min(t.encode_utf16().count());
                let mut b: Vec<u8> = t.encode_utf16().take(n).flat_map(u16::to_le_bytes).collect();
                b.extend_from_slice(&[0, 0]);
                let _ = self.mem.write(a[1], &b);
                v(n as i64)
            }
            "DestroyMenu" | "DrawMenuBar" | "EnableMenuItem" | "CheckMenuItem" => v(1),
            "GetMenu" => v(0),
            "GetKeyState" | "GetAsyncKeyState" | "SetCursor" | "ShowCursor" | "GetForegroundWindow" => v(0),
            "SetProcessDPIAware" => v(1),
            "IsWindow" => v(1),
            // Stock cursor/icon ids are small integers: any non-zero handle works.
            _ => v(1),
        }
    }
}
