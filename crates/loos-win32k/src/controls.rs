//! Standard window classes: `Button`, `Static`, the dialog class `#32770`, and `MessageBox`.

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::*;

/// BN_CLICKED.
pub const BN_CLICKED: u32 = 0;
/// IDOK.
pub const IDOK: i32 = 1;
/// IDCANCEL.
pub const IDCANCEL: i32 = 2;
/// IDYES.
pub const IDYES: i32 = 6;
/// IDNO.
pub const IDNO: i32 = 7;
/// MB_OK.
pub const MB_OK: u32 = 0;
/// MB_OKCANCEL.
pub const MB_OKCANCEL: u32 = 1;
/// MB_YESNO.
pub const MB_YESNO: u32 = 4;

fn button_proc(k: &mut Win32k, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        WM_PAINT => {
            if let Ok(ps) = k.begin_paint(h) {
                let (title, cw, ch, down) = (k.get_window_text(h).unwrap_or_default(), k.get_client_rect(h).unwrap_or_default(), 0, k.get_extra(h, 0).unwrap_or(0) == 1);
                let _ = ch;
                let face = k.create_solid_brush(sys_color(COLOR_BTNFACE));
                k.fill_rect(ps.hdc, cw, face);
                k.delete_object(face);
                let (hi, lo) = if down { (rgb(64, 64, 64), rgb(255, 255, 255)) } else { (rgb(255, 255, 255), rgb(64, 64, 64)) };
                for (c, a, b) in [(hi, (0, 0), (cw.right - 1, 0)), (hi, (0, 0), (0, cw.bottom - 1)), (lo, (0, cw.bottom - 1), (cw.right - 1, cw.bottom - 1)), (lo, (cw.right - 1, 0), (cw.right - 1, cw.bottom - 1))] {
                    let pen = k.create_pen(1, c);
                    let old = k.select_object(ps.hdc, pen);
                    k.move_to(ps.hdc, a.0, a.1);
                    k.line_to(ps.hdc, b.0, b.1);
                    k.select_object(ps.hdc, old);
                    k.delete_object(pen);
                }
                k.set_bk_mode(ps.hdc, 1);
                let off = down as i32;
                k.draw_text(ps.hdc, &title, Rect::new(off, off, cw.right + off, cw.bottom + off), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                k.end_paint(h, &ps);
            }
            0
        }
        WM_LBUTTONDOWN => {
            if k.is_window_enabled(h) {
                k.set_capture(h);
                k.set_extra(h, 0, 1).ok();
                k.invalidate_rect(h, None, false).ok();
            }
            0
        }
        WM_LBUTTONUP => {
            if k.get_extra(h, 0).unwrap_or(0) == 1 {
                k.release_capture();
                k.set_extra(h, 0, 0).ok();
                k.invalidate_rect(h, None, false).ok();
                let (x, y) = (lparam_x(l), lparam_y(l));
                let cr = k.get_client_rect(h).unwrap_or_default();
                if cr.contains(Point { x, y }) {
                    let parent = k.get_parent(h).unwrap_or(0);
                    let id = k.get_id(h).unwrap_or(0) as usize;
                    k.post_message(parent, WM_COMMAND, id | (BN_CLICKED as usize) << 16, h as isize).ok();
                }
            }
            0
        }
        _ => k.def_window_proc(h, msg, w, l),
    }
}

fn static_proc(k: &mut Win32k, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    if msg == WM_PAINT {
        if let Ok(ps) = k.begin_paint(h) {
            let title = k.get_window_text(h).unwrap_or_default();
            let cr = k.get_client_rect(h).unwrap_or_default();
            k.set_bk_mode(ps.hdc, 1);
            k.draw_text(ps.hdc, &title, cr, DT_LEFT | DT_SINGLELINE);
            k.end_paint(h, &ps);
        }
        return 0;
    }
    k.def_window_proc(h, msg, w, l)
}

/// EN_CHANGE notification code.
pub const EN_CHANGE: u32 = 0x0300;
const LINE_H: i32 = 16;

/// Multi-line `Edit`: the text lives in the window title. Appends typed characters, handles Backspace/Enter.
fn edit_proc(k: &mut Win32k, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if let Ok(ps) = k.begin_paint(h) {
                let (text, cr) = (k.get_window_text(h).unwrap_or_default(), k.get_client_rect(h).unwrap_or_default());
                let white = k.create_solid_brush(rgb(255, 255, 255));
                k.fill_rect(ps.hdc, cr, white);
                k.delete_object(white);
                k.set_bk_mode(ps.hdc, 1);
                let mut lines = 0;
                for (i, line) in text.split('\n').enumerate() {
                    let y = 2 + i as i32 * LINE_H;
                    k.draw_text(ps.hdc, line.trim_end_matches('\r'), Rect::new(3, y, cr.right, y + LINE_H), DT_LEFT | DT_SINGLELINE);
                    lines = i as i32;
                }
                if k.focus == h {
                    let last = text.rsplit('\n').next().unwrap_or("").chars().count() as i32;
                    let pen = k.create_pen(1, rgb(0, 0, 0));
                    let old = k.select_object(ps.hdc, pen);
                    let (x, y) = (3 + last * 7, 2 + lines * LINE_H);
                    k.move_to(ps.hdc, x, y);
                    k.line_to(ps.hdc, x, y + LINE_H - 2);
                    k.select_object(ps.hdc, old);
                    k.delete_object(pen);
                }
                k.end_paint(h, &ps);
            }
            0
        }
        WM_LBUTTONDOWN => {
            k.set_focus(h);
            0
        }
        WM_SETTEXT => {
            k.invalidate_rect(h, None, false).ok();
            1
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            k.invalidate_rect(h, None, false).ok();
            0
        }
        WM_CHAR => {
            let mut text = k.get_window_text(h).unwrap_or_default();
            match w as u32 {
                8 => {
                    text.pop();
                }
                13 => text.push('\n'),
                c if c >= 32 => text.extend(char::from_u32(c)),
                _ => return 0,
            }
            if let Some(win) = k.wins.get_mut(&h) {
                win.title = text;
            }
            k.invalidate_rect(h, None, false).ok();
            let (parent, id) = (k.get_parent(h).unwrap_or(0), k.get_id(h).unwrap_or(0) as usize);
            k.post_message(parent, WM_COMMAND, id | (EN_CHANGE as usize) << 16, h as isize).ok();
            0
        }
        _ => k.def_window_proc(h, msg, w, l),
    }
}

fn dialog_proc(k: &mut Win32k, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    if let Some(&dp) = k.dlg_procs.get(&h) {
        // Template dialog: the program's DLGPROC sees every message first; a non-zero return means "handled".
        let r = k.call_window_proc(Proc::User(dp), h, msg, w, l);
        if msg == 0x82 {
            k.dlg_procs.remove(&h); // WM_NCDESTROY
        }
        if r != 0 || msg == 0x110 {
            return r;
        }
        if msg == WM_CLOSE {
            return k.send_message(h, WM_COMMAND, IDCANCEL as usize, 0);
        }
        return k.def_window_proc(h, msg, w, l);
    }
    match msg {
        WM_COMMAND => {
            k.results.insert(h, (w & 0xffff) as i32);
            k.destroy_window(h).ok();
            0
        }
        WM_CLOSE => {
            k.results.insert(h, IDCANCEL);
            k.destroy_window(h).ok();
            0
        }
        _ => k.def_window_proc(h, msg, w, l),
    }
}

impl Win32k {
    pub(crate) fn init_controls(&mut self) {
        let face = COLOR_BTNFACE as u32 + 1;
        for (name, p, bg, extra) in [("Button", button_proc as NativeProc, face, 1), ("Static", static_proc, face, 0), ("msctls_statusbar32", static_proc, face, 0), ("Edit", edit_proc, 0, 0), ("SysTabControl32", static_proc, face, 0), ("SysListView32", static_proc, face, 0), ("SysTreeView32", static_proc, face, 0), ("ToolbarWindow32", static_proc, face, 0), ("ComboBox", static_proc, face, 0), ("ListBox", static_proc, face, 0), ("ScrollBar", static_proc, face, 0), ("msctls_progress32", static_proc, face, 0), ("msctls_trackbar32", static_proc, face, 0), ("tooltips_class32", static_proc, face, 0), ("RichEdit20W", edit_proc, 0, 0), ("#32770", dialog_proc, face, 0)] {
            self.register_class(WndClass { name: name.into(), style: 0, proc_: Proc::Native(p), background: bg, wnd_extra: extra }).ok();
        }
    }

    /// `MessageBox` (non-blocking form): shows a dialog and returns its handle. Poll
    /// [`Win32k::message_box_result`]; the app's thread keeps pumping messages meanwhile.
    pub fn message_box(&mut self, owner: Hwnd, text: &str, caption: &str, kind: u32) -> Result<Hwnd, Win32kError> {
        let (sw, sh) = self.screen;
        let (w, h) = (340, 150 + 24);
        let dlg = self.create_window_ex(0, "#32770", caption, WS_CAPTION | WS_SYSMENU | WS_VISIBLE, (sw - w) / 2, (sh - h) / 2, w, h, 0, 0, 0)?;
        self.wins.get_mut(&dlg).unwrap().owner = owner;
        self.create_window_ex(0, "Static", text, WS_CHILD | WS_VISIBLE, 16, 20, 300, 40, dlg, 0, 0)?;
        let buttons: &[(i32, &str)] = match kind & 0xF {
            MB_OKCANCEL => &[(IDOK, "OK"), (IDCANCEL, "Cancel")],
            MB_YESNO => &[(IDYES, "Yes"), (IDNO, "No")],
            _ => &[(IDOK, "OK")],
        };
        let total = buttons.len() as i32 * 84 - 8;
        for (i, (id, label)) in buttons.iter().enumerate() {
            self.create_window_ex(0, "Button", label, WS_CHILD | WS_VISIBLE, (w - 4 - total) / 2 + i as i32 * 84, 70, 76, 26, dlg, *id as isize, 0)?;
        }
        if owner != 0 {
            self.enable_window(owner, false).ok();
        }
        Ok(dlg)
    }

    /// Result of a finished message box (IDOK, ...); re-enables the owner. `None` while open.
    pub fn message_box_result(&mut self, dlg: Hwnd, owner: Hwnd) -> Option<i32> {
        let r = self.results.remove(&dlg)?;
        if owner != 0 {
            self.enable_window(owner, true).ok();
        }
        Some(r)
    }
}

/// One control of a dialog template (coordinates already in pixels, relative to the client area).
#[derive(Clone, Debug, Default)]
pub struct DlgItem {
    /// Window class name (`Button`, `Edit`, ...).
    pub class: String,
    /// Initial text.
    pub text: String,
    /// Window style (WS_CHILD is added).
    pub style: u32,
    /// Extended style.
    pub exstyle: u32,
    /// Left.
    pub x: i32,
    /// Top.
    pub y: i32,
    /// Width.
    pub cx: i32,
    /// Height.
    pub cy: i32,
    /// Control id.
    pub id: i32,
}

/// `WM_INITDIALOG`.
pub const WM_INITDIALOG: u32 = 0x0110;

impl Win32k {
    /// Creates a dialog window of class `#32770` from template data: the frame (`client` is the client size in pixels),
    /// its controls, then sends `WM_INITDIALOG(first tab-stop control, lparam)` to `dlgproc` (a user-mode address).
    #[allow(clippy::too_many_arguments)]
    pub fn create_dialog(&mut self, owner: Hwnd, title: &str, style: u32, exstyle: u32, pos: Option<(i32, i32)>, client: (i32, i32), dlgproc: u64, items: &[DlgItem], lparam: isize) -> Result<Hwnd, Win32kError> {
        let outer = crate::adjust_window_rect(style, false, Rect::new(0, 0, client.0, client.1));
        let (ow, oh) = (outer.width(), outer.height());
        let (sw, sh) = self.screen;
        let (x, y) = pos.unwrap_or(((sw - ow) / 2, (sh - oh) / 2));
        let dlg = self.create_window_ex(exstyle, "#32770", title, style & !WS_VISIBLE, x, y, ow, oh, 0, 0, 0)?;
        if let Some(win) = self.wins.get_mut(&dlg) {
            win.owner = owner;
        }
        if dlgproc != 0 {
            self.dlg_procs.insert(dlg, dlgproc);
        }
        let mut first = 0;
        for it in items {
            let hc = self.create_window_ex(it.exstyle, &it.class, &it.text, it.style | WS_CHILD, it.x, it.y, it.cx, it.cy, dlg, it.id as isize, 0)?;
            if first == 0 && it.style & 0x0001_0000 != 0 {
                first = hc; // WS_TABSTOP
            }
        }
        if dlgproc != 0 {
            self.call_window_proc(Proc::User(dlgproc), dlg, WM_INITDIALOG, first as usize, lparam);
        }
        if style & WS_VISIBLE != 0 {
            self.show_window(dlg, 5)?;
        }
        Ok(dlg)
    }

    /// `EndDialog`: records `result` for the modal loop and closes the dialog window.
    pub fn end_dialog(&mut self, dlg: Hwnd, result: i32) -> bool {
        if !self.is_window(dlg) {
            return false;
        }
        self.results.insert(dlg, result);
        self.destroy_window(dlg).is_ok()
    }
}
