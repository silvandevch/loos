//! user32/gdi32 entry points: decodes the Win64 calling convention (four registers plus stack arguments) and
//! forwards to `loos-win32k`. Window procedures registered by the program are reached through the kernel's upcall hook.

use crate::{Process, Ret};
use alloc::string::String;
use alloc::vec::Vec;
use loos_lin::mem::UserMem;
use crate::rsrc::{self, ResId};
use crate::dib;
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
    // window manager basics
    "GetParent", "GetWindow", "GetDlgItem", "IsWindowVisible", "IsWindowEnabled", "EnableWindow", "GetFocus", "GetActiveWindow",
    "SetActiveWindow", "SetForegroundWindow", "SetCapture", "ReleaseCapture", "GetCapture", "BringWindowToTop", "ClientToScreen",
    "ScreenToClient", "FindWindowW", "GetWindowLongPtrW", "GetWindowLongW", "SetWindowLongPtrW", "SetWindowLongW", "ValidateRect",
    "GetUpdateRect", "PostThreadMessageW", "UnregisterClassW", "CallWindowProcW", "DrawTextW", "SetWindowPos", "IsIconic",
    "IsZoomed", "GetSysColorBrush", "FindWindowA",
    // gdi32 basics
    "CreateCompatibleDC", "DeleteDC", "CreateCompatibleBitmap", "BitBlt", "SetPixel", "GetPixel", "CreateFontW", "CreateFontIndirectW",
    "GetTextExtentPoint32W", "GetTextMetricsW",
    // parity: shapes, stretching, rectangles, metrics
    "Ellipse", "Polygon", "Polyline", "PatBlt", "StretchBlt", "FrameRect", "InvertRect", "GetSysColor", "GetDeviceCaps",
    "SetRect", "SetRectEmpty", "CopyRect", "OffsetRect", "InflateRect", "IsRectEmpty", "PtInRect", "EqualRect", "IntersectRect",
    "UnionRect", "AdjustWindowRect", "AdjustWindowRectEx",
    // resources, dialogs, DIBs
    "FindResourceW", "FindResourceExW", "LoadResource", "LockResource", "SizeofResource", "FreeResource", "LoadStringW",
    "CreateDialogParamW", "CreateDialogIndirectParamW", "DialogBoxParamW", "DialogBoxIndirectParamW", "EndDialog",
    "GetDlgItemTextW", "SetDlgItemTextW", "SendDlgItemMessageW", "CheckDlgButton", "IsDlgButtonChecked", "GetDialogBaseUnits",
    "MapDialogRect", "IsDialogMessageW", "GetDlgItemInt", "SetDlgItemInt", "StretchDIBits", "SetDIBitsToDevice",
    "CreateDIBSection", "CreateDIBitmap", "GetDIBits", "SetDIBits", "GetObjectW", "GdiFlush",
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
/// Nearest-neighbour rescale of top-down pixels.
fn dib_scale(px: &[u32], sw: i32, sh: i32, dw: i32, dh: i32) -> Vec<u32> {
    if sw == dw && sh == dh {
        return px.to_vec();
    }
    (0..dh).flat_map(|y| (0..dw).map(move |x| px[((y * sh / dh) * sw + x * sw / dw) as usize])).collect()
}
fn rect_bytes(r: &loos_win32k::Rect) -> [u8; 16] {
    let mut o = [0u8; 16];
    for (i, x) in [r.left, r.top, r.right, r.bottom].iter().enumerate() {
        o[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
    }
    o
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

    /// Reads a `BITMAPINFO` (header + colour table) from the guest.
    fn read_bmi(&mut self, p: u64) -> Option<(dib::Info, Vec<u8>)> {
        let h = self.mem.read_vec(p, 40).ok()?;
        let (size, bpp) = (u32::from_le_bytes(h[0..4].try_into().ok()?) as usize, u16::from_le_bytes(h[14..16].try_into().ok()?));
        let used = u32::from_le_bytes(h[32..36].try_into().ok()?) as usize;
        let table = if bpp <= 8 { 4 * if used == 0 { 1 << bpp } else { used.min(256) } } else { 12 };
        let full = self.mem.read_vec(p, size.max(40) + table).or_else(|_| self.mem.read_vec(p, size.max(40))).ok()?;
        let info = dib::parse_info(&full)?;
        Some((info, full))
    }

    /// Reads a `BITMAPINFO` and its pixel data and decodes them (top-down 0xFFRRGGBB).
    fn read_dib(&mut self, bmi: u64, bits: u64) -> Option<(dib::Info, Vec<u32>)> {
        let (info, _) = self.read_bmi(bmi)?;
        let len = dib::stride(info.w, info.bpp) * info.h as usize;
        if len > (64 << 20) {
            return None;
        }
        let data = self.mem.read_vec(bits, len).ok()?;
        let px = dib::decode(&info, &data)?;
        Some((info, px))
    }

    /// Refreshes a DIB section's bitmap from its guest memory (the program writes the bits directly).
    fn sync_dib(&mut self, w: &mut Win32k, bmp: u32) {
        let Some((addr, hdr)) = self.dibs.get(&bmp).cloned() else { return };
        let Some(info) = dib::parse_info(&hdr) else { return };
        let Ok(data) = self.mem.read_vec(addr, dib::stride(info.w, info.bpp) * info.h as usize) else { return };
        if let Some(px) = dib::decode(&info, &data) {
            w.set_bitmap_pixels(bmp, &px);
        }
    }

    /// Dialog template bytes: from a resource (`name` = id or string) or directly from guest memory.
    fn dialog_template(&mut self, indirect: bool, hinst: u64, name: u64) -> Option<Vec<u8>> {
        if indirect {
            for len in [1 << 16, 1 << 14, 1 << 12, 1 << 10, 256] {
                if let Ok(b) = self.mem.read_vec(name, len) {
                    return Some(b);
                }
            }
            return None;
        }
        let base = if hinst == 0 { self.base } else { hinst };
        let id = ResId::from_arg(&mut self.mem, name)?;
        let (rva, size) = rsrc::find(&mut self.mem, base, &ResId::Id(rsrc::RT_DIALOG), &id)?;
        self.mem.read_vec(base + u64::from(rva), size as usize).ok()
    }

    fn rect_at(&mut self, p: u64) -> Option<loos_win32k::Rect> {
        let b = self.mem.read_vec(p, 16).ok()?;
        let q = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Some(loos_win32k::Rect { left: q(0), top: q(4), right: q(8), bottom: q(12) })
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
        if nr >= crate::api::UNRESOLVED_BASE {
            return self.syscall(nr, a, now_ms);
        }
        if nr >= crate::k32::K32_BASE {
            self.cur_rsp = rsp;
            return self.thread_syscall(nr, a, rsp, now_ms).or_else(|| self.ws_syscall(nr, a, rsp, now_ms)).or_else(|| self.k32_syscall(nr, a, rsp, now_ms)).or_else(|| self.adv_syscall(nr, a, rsp, now_ms)).or_else(|| self.crt_syscall(nr, a, rsp, now_ms)).unwrap_or(v(0));
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
                0 | 16 | 61 | 78 => 1024, // SCREEN/MAXIMIZED client/MAXTRACK/VIRTUALSCREEN width
                1 | 17 | 62 | 79 => 768,
                2 | 3 | 20 | 21 => 17,    // CX/CYVSCROLL, scroll arrows
                4 => 24,                  // CYCAPTION
                5 | 6 => 1,               // CX/CYBORDER
                7 | 8 | 32 | 33 => 2,     // dialog frame, sizing frame
                11 | 12 | 13 | 14 => 32,  // ICON, CURSOR
                15 => 22,                 // CYMENU
                28 | 29 => 112,           // CXMIN/CYMIN... track minimum
                34 | 35 => 136,
                36 | 37 => 4,             // CXDOUBLECLK
                43 => 3,                  // SM_CMOUSEBUTTONS
                80 => 1,                  // SM_CMONITORS
                75 => 1,                  // SM_MOUSEPRESENT-class flags
                _ => 0,
            }),
            "GetSysColor" => v(i64::from(loos_win32k::sys_color(a[0] as i32))),
            "GetDeviceCaps" => v(match a[1] as i32 {
                4 => 270,   // HORZSIZE (mm)
                6 => 203,   // VERTSIZE
                8 => 1024,  // HORZRES
                10 => 768,  // VERTRES
                12 => 32,   // BITSPIXEL
                14 => 1,    // PLANES
                24 => -1,   // NUMCOLORS (> 256)
                38 => 0x7e99, // RASTERCAPS: BITBLT, STRETCHBLT, DIBTODEV, ...
                88 | 90 => 96, // LOGPIXELSX/Y
                104 => 0,
                _ => 0,
            }),
            "FindResourceW" | "FindResourceExW" => {
                // (hModule, type, name) / (hModule, type, name, lang)
                let base = if a[0] == 0 { self.base } else { a[0] };
                let (Some(ty), Some(nm)) = (ResId::from_arg(&mut self.mem, a[1]), ResId::from_arg(&mut self.mem, a[2])) else { return v(0) };
                match rsrc::find(&mut self.mem, base, &ty, &nm) {
                    Some((rva, size)) => {
                        self.rsrc.insert(base + u64::from(rva), size);
                        v((base + u64::from(rva)) as i64)
                    }
                    None => {
                        self.set_last_error(1814); // ERROR_RESOURCE_NAME_NOT_FOUND
                        v(0)
                    }
                }
            }
            "LoadResource" | "LockResource" => v(a[if name == "LoadResource" { 1 } else { 0 }] as i64),
            "FreeResource" => v(0),
            "SizeofResource" => v(i64::from(self.rsrc.get(&a[1]).copied().unwrap_or(0))),
            "LoadStringW" => {
                let base = if a[0] == 0 { self.base } else { a[0] };
                let Some(t) = rsrc::load_string(&mut self.mem, base, a[1] as u16) else { return v(0) };
                let cap = a[3] as usize;
                if a[2] == 0 {
                    return v(0); // cchBufferMax == 0 would return a read-only pointer in Windows: unsupported
                }
                let n = t.encode_utf16().count().min(cap.saturating_sub(1));
                let mut b: Vec<u8> = t.encode_utf16().take(n).flat_map(u16::to_le_bytes).collect();
                b.extend_from_slice(&[0, 0]);
                let _ = self.mem.write(a[2], &b);
                v(n as i64)
            }
            "CreateDialogParamW" | "CreateDialogIndirectParamW" | "DialogBoxParamW" | "DialogBoxIndirectParamW" => {
                let modal = name.starts_with("DialogBox");
                let lparam = self.arg(rsp, 5) as isize;
                let key = a;
                let mut state = self.modal.iter().rposition(|m| m.0 == key);
                if !modal || state.is_none() {
                    let Some(tpl) = self.dialog_template(name.contains("Indirect"), a[0], a[1]) else {
                        self.set_last_error(1814);
                        return v(-1);
                    };
                    let Some(d) = rsrc::parse_dialog(&tpl) else { return v(-1) };
                    let owner = a[2] as u32;
                    let Ok(dlg) = w.create_dialog(owner, &d.title, d.style, d.exstyle, d.pos, d.client, a[3], &d.items, lparam) else { return v(if modal { -1 } else { 0 }) };
                    if !modal {
                        return v(i64::from(dlg));
                    }
                    if owner != 0 {
                        let _ = w.enable_window(owner, false);
                    }
                    let _ = w.show_window(dlg, 5);
                    self.modal.push((key, dlg, owner));
                    state = Some(self.modal.len() - 1);
                }
                let (_, dlg, owner) = self.modal[state.unwrap()];
                // Nested message loop: pump this thread's queue until EndDialog records a result.
                for _ in 0..256 {
                    if let Some(r) = w.message_box_result(dlg, owner) {
                        self.modal.remove(state.unwrap());
                        return v(i64::from(r));
                    }
                    if !w.is_window(dlg) {
                        self.modal.remove(state.unwrap());
                        return v(-1);
                    }
                    match w.peek_message(0, 0, 0, true) {
                        Some(m) if m.message == 0x12 => {
                            w.post_quit_message(m.wparam as i32);
                            self.modal.remove(state.unwrap());
                            return v(0);
                        }
                        Some(m) => {
                            w.translate_message(&m);
                            w.dispatch_message(&m);
                        }
                        None => return Ret::Idle,
                    }
                }
                Ret::Yield
            }
            "EndDialog" => v(w.end_dialog(a[0] as u32, a[1] as i32) as i64),
            "GetDlgItemTextW" | "SetDlgItemTextW" | "SendDlgItemMessageW" | "CheckDlgButton" | "IsDlgButtonChecked" | "GetDlgItemInt" | "SetDlgItemInt" => {
                let ctl = w.children(a[0] as u32).into_iter().find(|c| w.get_id(*c).is_ok_and(|id| id == a[1] as i32 as isize));
                let Some(ctl) = ctl else { return v(0) };
                match name {
                    "GetDlgItemTextW" => {
                        let t = w.get_window_text(ctl).unwrap_or_default();
                        let n = (a[3] as usize).saturating_sub(1).min(t.encode_utf16().count());
                        let mut b: Vec<u8> = t.encode_utf16().take(n).flat_map(u16::to_le_bytes).collect();
                        b.extend_from_slice(&[0, 0]);
                        let _ = self.mem.write(a[2], &b);
                        v(n as i64)
                    }
                    "SetDlgItemTextW" => v(wide(&mut self.mem, a[2]).is_some_and(|t| w.set_window_text(ctl, &t).is_ok()) as i64),
                    "SendDlgItemMessageW" => {
                        let l = self.arg(rsp, 5) as isize;
                        v(w.send_message(ctl, a[2] as u32, a[3] as usize, l) as i64)
                    }
                    "CheckDlgButton" => {
                        self.checks.insert(ctl, a[2] as u32);
                        v(1)
                    }
                    "IsDlgButtonChecked" => v(i64::from(self.checks.get(&ctl).copied().unwrap_or(0))),
                    "SetDlgItemInt" => {
                        let t = if a[3] != 0 { alloc::format!("{}", a[2] as i32) } else { alloc::format!("{}", a[2] as u32) };
                        v(w.set_window_text(ctl, &t).is_ok() as i64)
                    }
                    _ => {
                        // GetDlgItemInt(dlg, id, &translated, signed)
                        let t = w.get_window_text(ctl).unwrap_or_default();
                        let n: Option<i64> = t.trim().parse().ok();
                        if a[2] != 0 {
                            let _ = self.mem.write(a[2], &u32::from(n.is_some()).to_le_bytes());
                        }
                        v(n.unwrap_or(0))
                    }
                }
            }
            "GetDialogBaseUnits" => v(i64::from(rsrc::BASE_X) | i64::from(rsrc::BASE_Y) << 16),
            "MapDialogRect" => match self.rect_at(a[1]) {
                Some(r) => {
                    let r = loos_win32k::Rect { left: r.left * rsrc::BASE_X / 4, top: r.top * rsrc::BASE_Y / 8, right: r.right * rsrc::BASE_X / 4, bottom: r.bottom * rsrc::BASE_Y / 8 };
                    v(self.mem.write(a[1], &rect_bytes(&r)).is_ok() as i64)
                }
                None => v(0),
            },
            "IsDialogMessageW" => v(0),
            "StretchDIBits" | "SetDIBitsToDevice" => {
                let s: Vec<u64> = (5..=13).map(|n| self.arg(rsp, n)).collect();
                let (bits, bmi) = if name == "StretchDIBits" { (s[5], s[6]) } else { (s[5], s[6]) };
                let Some((info, px)) = self.read_dib(bmi, bits) else { return v(0) };
                // StretchDIBits: dest (x, y, w, h) <- src (xs, ys, ws, hs); SetDIBitsToDevice copies (xs, ys, w, h) 1:1.
                let (dx, dy, dw, dh, xs, ys, ws, hs) = if name == "StretchDIBits" {
                    (a[1] as i32, a[2] as i32, a[3] as i32, s[0] as i32, s[1] as i32, s[2] as i32, s[3] as i32, s[4] as i32)
                } else {
                    (a[1] as i32, a[2] as i32, a[3] as i32, s[0] as i32, s[1] as i32, s[2] as i32, a[3] as i32, s[0] as i32)
                };
                // Source rows are numbered from the bottom for bottom-up DIBs; `px` is top-down.
                let y0 = if info.top_down { ys } else { info.h - ys - hs };
                if ws <= 0 || hs <= 0 || xs < 0 || y0 < 0 || xs + ws > info.w || y0 + hs > info.h {
                    return v(0);
                }
                let mut crop = Vec::with_capacity((ws * hs) as usize);
                for r in y0..y0 + hs {
                    crop.extend_from_slice(&px[(r * info.w + xs) as usize..(r * info.w + xs + ws) as usize]);
                }
                let ok = w.draw_pixels(a[0] as u32, dx, dy, dw, dh, &crop, ws, hs);
                v(if ok { i64::from(hs) } else { 0 })
            }
            "CreateDIBSection" => {
                // (hdc, bmi, usage, &bits, hSection[5], offset[6])
                let Some((info, _)) = self.read_bmi(a[1]) else { return v(0) };
                let Ok(bmp) = w.create_bitmap(info.w, info.h) else { return v(0) };
                let len = dib::stride(info.w, info.bpp) as u64 * info.h as u64;
                let Ok(addr) = self.mem.map_anywhere(loos_lin::mem::ceil(len.max(1)), 3, &[], "dib") else { return v(0) };
                if a[3] != 0 {
                    let _ = self.mem.write(a[3], &addr.to_le_bytes());
                }
                let hdr = self.mem.read_vec(a[1], info.header_len).unwrap_or_default();
                self.dibs.insert(bmp, (addr, hdr));
                v(i64::from(bmp))
            }
            "CreateDIBitmap" => {
                // (hdc, lpbih, fdwInit, lpbInit, lpbmi[5], usage[6])
                let Ok(h) = self.mem.read_vec(a[1], 40) else { return v(0) };
                let (bw, bh) = (i32::from_le_bytes(h[4..8].try_into().unwrap()), i32::from_le_bytes(h[8..12].try_into().unwrap()).abs());
                let Ok(bmp) = w.create_bitmap(bw, bh) else { return v(0) };
                if a[2] & 4 != 0 {
                    let bmi = self.arg(rsp, 5);
                    if let Some((info, px)) = self.read_dib(bmi, a[3]) {
                        let scaled = dib_scale(&px, info.w, info.h, bw, bh);
                        w.set_bitmap_pixels(bmp, &scaled);
                    }
                }
                v(i64::from(bmp))
            }
            "SetDIBits" => {
                // (hdc, hbm, start, lines, bits[5], bmi[6], usage[7])
                let (bits, bmi) = (self.arg(rsp, 5), self.arg(rsp, 6));
                let (Some((info, px)), Some((bw, bh))) = (self.read_dib(bmi, bits), w.bitmap_size(a[1] as u32)) else { return v(0) };
                let scaled = dib_scale(&px, info.w, info.h, bw, bh);
                v(if w.set_bitmap_pixels(a[1] as u32, &scaled) { i64::from(info.h) } else { 0 })
            }
            "GetDIBits" => {
                let (bits, bmi) = (self.arg(rsp, 5), self.arg(rsp, 6));
                self.sync_dib(w, a[1] as u32);
                let Some((bw, bh)) = w.bitmap_size(a[1] as u32) else { return v(0) };
                let Ok(mut hdr) = self.mem.read_vec(bmi, 40) else { return v(0) };
                let want_top_down = i32::from_le_bytes(hdr[8..12].try_into().unwrap()) < 0;
                let mut bpp = u16::from_le_bytes(hdr[14..16].try_into().unwrap());
                if bpp != 24 {
                    bpp = 32;
                }
                if bits == 0 {
                    // Query form: fill the header with the bitmap's geometry in 32 bpp.
                    hdr[0..4].copy_from_slice(&40u32.to_le_bytes());
                    hdr[4..8].copy_from_slice(&bw.to_le_bytes());
                    hdr[8..12].copy_from_slice(&(if want_top_down { -bh } else { bh }).to_le_bytes());
                    hdr[12..14].copy_from_slice(&1u16.to_le_bytes());
                    hdr[14..16].copy_from_slice(&32u16.to_le_bytes());
                    hdr[16..20].copy_from_slice(&0u32.to_le_bytes());
                    hdr[20..24].copy_from_slice(&((dib::stride(bw, 32) * bh as usize) as u32).to_le_bytes());
                    return v(i64::from(self.mem.write(bmi, &hdr).is_ok()));
                }
                let Some(px) = w.bitmap_pixels(a[1] as u32) else { return v(0) };
                let out = dib::encode(px, bw, bh, bpp, want_top_down);
                v(if self.mem.write(bits, &out).is_ok() { i64::from(bh) } else { 0 })
            }
            "GetObjectW" => {
                self.sync_dib(w, a[0] as u32);
                let Some((bw, bh)) = w.bitmap_size(a[0] as u32) else { return v(0) };
                if a[2] == 0 {
                    return v(32);
                }
                let mut o = [0u8; 32];
                o[4..8].copy_from_slice(&bw.to_le_bytes());
                o[8..12].copy_from_slice(&bh.to_le_bytes());
                o[12..16].copy_from_slice(&(dib::stride(bw, 32) as i32).to_le_bytes());
                o[16..18].copy_from_slice(&1u16.to_le_bytes());
                o[18..20].copy_from_slice(&32u16.to_le_bytes());
                if let Some((addr, _)) = self.dibs.get(&(a[0] as u32)) {
                    o[24..32].copy_from_slice(&addr.to_le_bytes());
                }
                v(if (a[1] as usize) >= 32 && self.mem.write(a[2], &o).is_ok() { 32 } else { 0 })
            }
            "GdiFlush" => {
                // Push GDI drawing into DIB sections' guest memory (32/24 bpp) so the program can read the bits.
                let handles: Vec<u32> = self.dibs.keys().copied().collect();
                for h in handles {
                    let (addr, hdr) = self.dibs[&h].clone();
                    let (Some(px), Some((bw, bh)), Some(info)) = (w.bitmap_pixels(h), w.bitmap_size(h), dib::parse_info(&hdr)) else { continue };
                    if matches!(info.bpp, 24 | 32) {
                        let _ = self.mem.write(addr, &dib::encode(px, bw, bh, info.bpp, info.top_down));
                    }
                }
                v(1)
            }
            "Ellipse" => v(w.ellipse(a[0] as u32, a[1] as i32, a[2] as i32, a[3] as i32, self.arg(rsp, 5) as i32) as i64),
            "Polygon" | "Polyline" => {
                let n = (a[2] as usize).min(4096);
                let Ok(b) = self.mem.read_vec(a[1], n * 8) else { return v(0) };
                let pts: Vec<loos_win32k::Point> = b
                    .chunks_exact(8)
                    .map(|c| loos_win32k::Point { x: i32::from_le_bytes(c[0..4].try_into().unwrap()), y: i32::from_le_bytes(c[4..8].try_into().unwrap()) })
                    .collect();
                v(if name == "Polygon" { w.polygon(a[0] as u32, &pts) } else { w.polyline(a[0] as u32, &pts) } as i64)
            }
            "PatBlt" => v(w.pat_blt(a[0] as u32, a[1] as i32, a[2] as i32, a[3] as i32, self.arg(rsp, 5) as i32, self.arg(rsp, 6) as u32) as i64),
            "StretchBlt" => {
                // (dst, dx, dy, dw, dh[5], src[6], sx[7], sy[8], sw[9], sh[10], rop[11])
                let s: Vec<u64> = (5..=11).map(|n| self.arg(rsp, n)).collect();
                if s[6] as u32 != 0x00CC_0020 {
                    return v(0);
                }
                self.sync_dib(w, w.dc_bitmap(s[1] as u32));
                v(w.stretch_blt(a[0] as u32, a[1] as i32, a[2] as i32, a[3] as i32, s[0] as i32, s[1] as u32, s[2] as i32, s[3] as i32, s[4] as i32, s[5] as i32) as i64)
            }
            "FrameRect" => match self.rect_at(a[1]) {
                Some(r) => v(w.frame_rect(a[0] as u32, r, a[2] as u32) as i64),
                None => v(0),
            },
            "InvertRect" => match self.rect_at(a[1]) {
                Some(r) => v(w.pat_blt(a[0] as u32, r.left, r.top, r.width(), r.height(), 0x0055_0009) as i64),
                None => v(0),
            },
            "SetRect" => {
                let r = loos_win32k::Rect { left: a[1] as i32, top: a[2] as i32, right: a[3] as i32, bottom: self.arg(rsp, 5) as i32 };
                v(self.mem.write(a[0], &rect_bytes(&r)).is_ok() as i64)
            }
            "SetRectEmpty" => v(self.mem.write(a[0], &[0u8; 16]).is_ok() as i64),
            "CopyRect" => match self.rect_at(a[1]) {
                Some(r) => v(self.mem.write(a[0], &rect_bytes(&r)).is_ok() as i64),
                None => v(0),
            },
            "OffsetRect" | "InflateRect" => match self.rect_at(a[0]) {
                Some(r) => {
                    let (dx, dy) = (a[1] as i32, a[2] as i32);
                    let r = if name == "OffsetRect" {
                        r.offset(dx, dy)
                    } else {
                        loos_win32k::Rect { left: r.left - dx, top: r.top - dy, right: r.right + dx, bottom: r.bottom + dy }
                    };
                    v(self.mem.write(a[0], &rect_bytes(&r)).is_ok() as i64)
                }
                None => v(0),
            },
            "IsRectEmpty" => v(self.rect_at(a[0]).is_none_or(|r| r.is_empty()) as i64),
            "PtInRect" => match self.rect_at(a[0]) {
                Some(r) => v(r.contains(loos_win32k::Point { x: a[1] as i32, y: (a[1] >> 32) as i32 }) as i64),
                None => v(0),
            },
            "EqualRect" => v(matches!((self.rect_at(a[0]), self.rect_at(a[1])), (Some(x), Some(y)) if x == y) as i64),
            "IntersectRect" | "UnionRect" => {
                let (Some(x), Some(y)) = (self.rect_at(a[1]), self.rect_at(a[2])) else { return v(0) };
                let r = if name == "IntersectRect" {
                    x.intersect(&y)
                } else if x.is_empty() {
                    y
                } else if y.is_empty() {
                    x
                } else {
                    x.union(&y)
                };
                let _ = self.mem.write(a[0], &rect_bytes(&if r.is_empty() && name == "IntersectRect" { loos_win32k::Rect::default() } else { r }));
                v(i64::from(!r.is_empty()))
            }
            "AdjustWindowRect" | "AdjustWindowRectEx" => match self.rect_at(a[0]) {
                Some(r) => {
                    let r = loos_win32k::adjust_window_rect(a[1] as u32, a[2] as u32 != 0, r);
                    v(self.mem.write(a[0], &rect_bytes(&r)).is_ok() as i64)
                }
                None => v(0),
            },
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
            "GetKeyState" | "GetAsyncKeyState" | "SetCursor" | "ShowCursor" => v(0),
            "SetProcessDPIAware" => v(1),
            "IsWindow" => v(w.is_window(a[0] as u32) as i64),
            "GetForegroundWindow" | "GetActiveWindow" => v(i64::from(w.get_active_window())),
            "GetParent" => v(w.get_parent(a[0] as u32).map_or(0, i64::from)),
            "GetWindow" => {
                // GW_HWNDFIRST 0, HWNDLAST 1, HWNDNEXT 2, HWNDPREV 3, OWNER 4, CHILD 5: sibling order is the child list.
                let h = a[0] as u32;
                let sib = |w: &Win32k| -> Vec<u32> {
                    match w.get_parent(h) {
                        Ok(p) if p != 0 => w.children(p),
                        _ => w.top_levels(),
                    }
                };
                let r = match a[1] {
                    5 => w.children(h).first().copied().unwrap_or(0),
                    4 => 0,
                    c => {
                        let l = sib(w);
                        let i = l.iter().position(|x| *x == h);
                        match (c, i) {
                            (0, _) => l.first().copied().unwrap_or(0),
                            (1, _) => l.last().copied().unwrap_or(0),
                            (2, Some(i)) => l.get(i + 1).copied().unwrap_or(0),
                            (3, Some(i)) if i > 0 => l[i - 1],
                            _ => 0,
                        }
                    }
                };
                v(i64::from(r))
            }
            "GetDlgItem" => v(w.children(a[0] as u32).into_iter().find(|c| w.get_id(*c).is_ok_and(|id| id == a[1] as i32 as isize)).map_or(0, i64::from)),
            "IsWindowVisible" => v(w.is_window_visible(a[0] as u32) as i64),
            "IsWindowEnabled" => v(w.is_window_enabled(a[0] as u32) as i64),
            "EnableWindow" => v(w.enable_window(a[0] as u32, a[1] as u32 != 0).map_or(0, i64::from)),
            "GetFocus" => v(i64::from(w.get_focus())),
            "SetActiveWindow" | "SetForegroundWindow" => {
                let prev = w.set_active_window(a[0] as u32);
                v(if name == "SetActiveWindow" { i64::from(prev) } else { 1 })
            }
            "SetCapture" => v(i64::from(w.set_capture(a[0] as u32))),
            "ReleaseCapture" => {
                w.release_capture();
                v(1)
            }
            "GetCapture" => v(i64::from(w.get_capture())),
            "BringWindowToTop" => v(w.bring_window_to_top(a[0] as u32).is_ok() as i64),
            "ClientToScreen" | "ScreenToClient" => {
                let Ok(b) = self.mem.read_vec(a[1], 8) else { return v(0) };
                let pt = loos_win32k::Point { x: i32::from_le_bytes(b[0..4].try_into().unwrap()), y: i32::from_le_bytes(b[4..8].try_into().unwrap()) };
                let r = if name == "ClientToScreen" { w.client_to_screen(a[0] as u32, pt) } else { w.screen_to_client(a[0] as u32, pt) };
                match r {
                    Ok(p) => {
                        let mut o = [0u8; 8];
                        o[0..4].copy_from_slice(&p.x.to_le_bytes());
                        o[4..8].copy_from_slice(&p.y.to_le_bytes());
                        v(self.mem.write(a[1], &o).is_ok() as i64)
                    }
                    Err(_) => v(0),
                }
            }
            "FindWindowW" | "FindWindowA" => {
                let rd = |m: &mut UserMem, p: u64| if name.ends_with('W') { wide(m, p) } else { narrow(m, p) };
                let (c, t) = (rd(&mut self.mem, a[0]), rd(&mut self.mem, a[1]));
                v(i64::from(w.find_window(c.as_deref(), t.as_deref())))
            }
            "GetWindowLongPtrW" | "GetWindowLongW" => {
                let (h, idx) = (a[0] as u32, a[1] as i32);
                let r: Option<i64> = match idx {
                    -21 => w.get_user_data(h).ok().map(|x| x as i64),
                    -12 => w.get_id(h).ok().map(|x| x as i64),
                    -16 => w.get_style(h).ok().map(i64::from),
                    -6 => Some(self.base as i64),
                    -4 => w.set_window_proc(h, Proc::User(0)).ok().map(|old| {
                        let _ = w.set_window_proc(h, old);
                        match old { Proc::User(p) => p as i64, Proc::Native(_) => 0 }
                    }),
                    i if i >= 0 => w.get_extra(h, i as usize / 8).ok().map(|x| x as i64),
                    _ => Some(0),
                };
                v(r.unwrap_or(0))
            }
            "SetWindowLongPtrW" | "SetWindowLongW" => {
                let (h, idx) = (a[0] as u32, a[1] as i32);
                let r = match idx {
                    -21 => w.set_user_data(h, a[2] as isize).ok().map(|x| x as i64),
                    -4 => w.set_window_proc(h, Proc::User(a[2])).ok().map(|old| match old { Proc::User(p) => p as i64, Proc::Native(_) => 0 }),
                    i if i >= 0 => w.set_extra(h, i as usize / 8, a[2] as isize).ok().map(|x| x as i64),
                    _ => Some(0), // style/id changes after creation are not supported
                };
                v(r.unwrap_or(0))
            }
            "ValidateRect" => v(w.validate_rect(a[0] as u32).is_ok() as i64),
            "GetUpdateRect" => match w.get_update_rect(a[0] as u32) {
                Ok(r) => {
                    if a[1] != 0 {
                        let r = r.unwrap_or_default();
                        let _ = self.mem.write(a[1], &rect_bytes(&r));
                    }
                    v(i64::from(r.is_some()))
                }
                Err(_) => v(0),
            },
            "PostThreadMessageW" => {
                w.post_thread_message(a[0] as u32, a[1] as u32, a[2] as usize, a[3] as isize);
                v(1)
            }
            "UnregisterClassW" => v(wide(&mut self.mem, a[0]).is_some_and(|n| w.unregister_class(&n).is_ok()) as i64),
            "CallWindowProcW" => {
                let l = self.arg(rsp, 5) as isize;
                v(w.call_window_proc(Proc::User(a[0]), a[1] as u32, a[2] as u32, a[3] as usize, l) as i64)
            }
            "DrawTextW" => {
                let Ok(b) = self.mem.read_vec(a[3], 16) else { return v(0) };
                let q = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
                let rc = loos_win32k::Rect { left: q(0), top: q(4), right: q(8), bottom: q(12) };
                let Some(mut t) = wide(&mut self.mem, a[1]) else { return v(0) };
                if (a[2] as i32) >= 0 {
                    t = t.chars().take(a[2] as i32 as usize).collect();
                }
                v(i64::from(w.draw_text(a[0] as u32, &t, rc, self.arg(rsp, 5) as u32)))
            }
            "SetWindowPos" => {
                // (h, after, x, y, cx[5], cy[6], flags[7]): SWP_NOSIZE 1, NOMOVE 2, NOZORDER 4, HIDE 0x80, SHOW 0x40
                let (h, flags) = (a[0] as u32, self.arg(rsp, 7) as u32);
                let Ok(cur) = w.get_window_rect(h) else { return v(0) };
                let (x, y) = if flags & 2 != 0 { (cur.left, cur.top) } else { (a[2] as i32, a[3] as i32) };
                let (cx, cy) = if flags & 1 != 0 { (cur.width(), cur.height()) } else { (self.arg(rsp, 5) as i32, self.arg(rsp, 6) as i32) };
                let mut ok = w.move_window(h, x, y, cx, cy).is_ok();
                if flags & 4 == 0 && a[1] == 0 {
                    ok &= w.bring_window_to_top(h).is_ok();
                }
                if flags & 0x40 != 0 {
                    let _ = w.show_window(h, 5);
                }
                if flags & 0x80 != 0 {
                    let _ = w.show_window(h, 0);
                }
                v(ok as i64)
            }
            "IsIconic" | "IsZoomed" => v(0),
            "GetSysColorBrush" => v(i64::from(w.sys_color_brush(a[0] as i32))),
            "CreateCompatibleDC" => v(i64::from(w.create_compatible_dc())),
            "DeleteDC" => v(w.delete_dc(a[0] as u32) as i64),
            "CreateCompatibleBitmap" => v(w.create_bitmap(a[1] as i32, a[2] as i32).map_or(0, i64::from)),
            "BitBlt" => {
                // (dst, dx, dy, w, h[5], src[6], sx[7], sy[8], rop[9]): only SRCCOPY (0x00CC0020) is a plain copy.
                let s: Vec<u64> = (5..=9).map(|n| self.arg(rsp, n)).collect();
                if s[4] as u32 != 0x00CC_0020 {
                    return v(0);
                }
                self.sync_dib(w, w.dc_bitmap(s[1] as u32));
                v(w.bit_blt(a[0] as u32, a[1] as i32, a[2] as i32, a[3] as i32, s[0] as i32, s[1] as u32, s[2] as i32, s[3] as i32) as i64)
            }
            "SetPixel" => {
                let c = a[3] as u32;
                v(if w.set_pixel(a[0] as u32, a[1] as i32, a[2] as i32, c) { i64::from(c) } else { -1 })
            }
            "GetPixel" => v(w.get_pixel(a[0] as u32, a[1] as i32, a[2] as i32).map_or(0xffff_ffff, i64::from)),
            "CreateFontW" => v(i64::from(w.create_font(a[0] as i32))),
            "CreateFontIndirectW" => {
                let h = self.mem.read_u32(a[0]).unwrap_or(16) as i32;
                v(i64::from(w.create_font(h)))
            }
            "GetTextExtentPoint32W" => {
                let Some(t) = wide(&mut self.mem, a[1]) else { return v(0) };
                let t: String = t.chars().take(a[2] as i32 as usize).collect();
                let (cx, cy) = w.text_extent(a[0] as u32, &t);
                let mut o = [0u8; 8];
                o[0..4].copy_from_slice(&cx.to_le_bytes());
                o[4..8].copy_from_slice(&cy.to_le_bytes());
                v(self.mem.write(a[3], &o).is_ok() as i64)
            }
            "GetTextMetricsW" => {
                let (cw, ch) = w.text_extent(a[0] as u32, "x");
                let mut o = [0u8; 60];
                let put = |o: &mut [u8; 60], at: usize, x: i32| o[at..at + 4].copy_from_slice(&x.to_le_bytes());
                put(&mut o, 0, ch);
                put(&mut o, 4, ch * 4 / 5);
                put(&mut o, 8, ch - ch * 4 / 5);
                put(&mut o, 20, cw);
                put(&mut o, 24, cw * 2);
                put(&mut o, 28, 400);
                v(self.mem.write(a[1], &o).is_ok() as i64)
            }
            // Stock cursor/icon ids are small integers: any non-zero handle works.
            _ => v(1),
        }
    }
}
