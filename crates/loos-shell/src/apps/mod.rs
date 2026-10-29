//! Built-in apps: Notepad, LoCmd terminal, Calculator, LoOS Browser. Logic is safe Rust
//! ([`editor`], [`term`], [`calc`], [`browser`]); the `loos_*_frame` functions draw through the C
//! widget toolkit (`src/shell/ui.h`) until it is ported (prompts/shell-rust/PHASE-5).
#![allow(clippy::missing_safety_doc, missing_docs)]

pub mod browser;
pub mod calc;
pub mod editor;
pub mod term;

use crate::draw::{Rect, Theme};
use crate::ffi::{self, Fb};
use crate::font::ShellFont;
use crate::font_data;
use alloc::boxed::Box;
use alloc::ffi::CString;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{c_char, c_void};

/// Opaque C `ui_ctx`.
#[repr(C)]
pub struct UiCtx {
    _p: [u8; 0],
}

#[repr(C)]
struct FsEntry {
    name: [u8; 64],
    is_dir: bool,
    size: u64,
    mtime: i64,
    attrs: u32,
}

extern "C" {
    fn ui_fb(u: *mut UiCtx) -> *mut Fb;
    fn ui_theme_of(u: *const UiCtx) -> *const Theme;
    fn ui_pointer(u: *const UiCtx, x: *mut i32, y: *mut i32, pressed: *mut bool, wheel: *mut i32);
    fn ui_next_input(u: *mut UiCtx, key: *mut u32, mods: *mut u32, is_char: *mut bool) -> bool;
    fn ui_blur(u: *mut UiCtx);
    fn ui_button(u: *mut UiCtx, r: Rect, label: *const c_char) -> bool;
    fn ui_button_accent(u: *mut UiCtx, r: Rect, label: *const c_char) -> bool;
    fn ui_textfield(u: *mut UiCtx, r: Rect, buf: *mut c_char, cap: usize, caret: *mut i32, placeholder: *const c_char) -> bool;
    fn ui_textfield_id(placeholder: *const c_char, r: Rect) -> u32;
    fn ui_focus(u: *mut UiCtx, id: u32);
    fn ui_focused(u: *const UiCtx, id: u32) -> bool;
    fn ui_take_key(u: *mut UiCtx, key: u32, need_mods: u32) -> bool;
    fn fb_text(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: *const c_char, c: u32) -> i32;
    fn fb_text_ex(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: *const c_char, c: u32, f: ShellFont) -> i32;
    fn shell_text_width(s: *const c_char) -> i32;
    fn shell_text_width_ex(s: *const c_char, f: ShellFont) -> i32;
    fn shell_text_fit(s: *const c_char, max_px: i32) -> i32;
    fn shell_text_index_at(s: *const c_char, x: i32) -> i32;
    fn be_fs_list(b: *const c_void, path: *const c_char, out: *mut FsEntry, max: i32) -> i32;
    fn be_fs_read(b: *const c_void, path: *const c_char, out: *mut c_char, cap: usize) -> i64;
    fn be_fs_write(b: *const c_void, path: *const c_char, data: *const c_char, len: usize) -> bool;
    fn be_fs_mkdir(b: *const c_void, path: *const c_char) -> bool;
    fn be_fs_remove(b: *const c_void, path: *const c_char) -> bool;
    fn be_ps_list(b: *const c_void, out: *mut c_char, cap: usize) -> i32;
}

pub(crate) fn cs(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// Thin drawing wrapper over one frame's `ui_ctx`.
pub(crate) struct Ui {
    pub(crate) u: *mut UiCtx,
    pub(crate) fb: *mut Fb,
    pub(crate) t: Theme,
    pub(crate) clip: Rect,
}

impl Ui {
    pub(crate) unsafe fn new(u: *mut UiCtx, w: i32, h: i32) -> Self {
        // SAFETY: called from a frame callback with a live ui_ctx.
        let (fb, t) = unsafe { (ui_fb(u), *ui_theme_of(u)) };
        Ui { u, fb, t, clip: Rect::new(0, 0, w, h) }
    }
    pub(crate) fn fill(&self, r: Rect, c: u32) {
        // SAFETY: fb belongs to the live ui_ctx.
        unsafe { ffi::fb_fill(self.fb, self.clip, r, c) }
    }
    pub(crate) fn frame(&self, r: Rect, c: u32) {
        // SAFETY: fb belongs to the live ui_ctx.
        unsafe { ffi::fb_frame(self.fb, self.clip, r, c) }
    }
    pub(crate) fn round(&self, r: Rect, rad: i32, c: u32) {
        // SAFETY: as above.
        unsafe { ffi::fb_fill_round(self.fb, self.clip, r, rad, c) }
    }
    pub(crate) fn text(&self, x: i32, y: i32, s: &str, c: u32) -> i32 {
        let s = cs(s);
        // SAFETY: as above; s is NUL-terminated.
        unsafe { fb_text(self.fb, self.clip, x, y, s.as_ptr(), c) }
    }
    /// Draw text at a logical size (`bold` selects the semibold face).
    pub(crate) fn text_sized(&self, x: i32, y: i32, s: &str, c: u32, size: i32, bold: bool) -> i32 {
        let s = cs(s);
        let f = face(size, bold);
        // SAFETY: fb belongs to the live ui_ctx; s is NUL-terminated.
        unsafe { fb_text_ex(self.fb, self.clip, x, y, s.as_ptr(), c, f) }
    }
    pub(crate) fn width(s: &str) -> i32 {
        // SAFETY: NUL-terminated.
        unsafe { shell_text_width(cs(s).as_ptr()) }
    }
    /// Width of text at a logical size and weight.
    pub(crate) fn width_sized(s: &str, size: i32, bold: bool) -> i32 {
        let s = cs(s);
        let f = face(size, bold);
        // SAFETY: NUL-terminated.
        unsafe { shell_text_width_ex(s.as_ptr(), f) }
    }
    /// Number of bytes of `s` that fit in `max_px`.
    pub(crate) fn fit(s: &str, max_px: i32) -> usize {
        let s = cs(s);
        // SAFETY: NUL-terminated.
        unsafe { shell_text_fit(s.as_ptr(), max_px.max(0)) as usize }
    }
    pub(crate) fn index_at(s: &str, x: i32) -> usize {
        // SAFETY: NUL-terminated.
        unsafe { shell_text_index_at(cs(s).as_ptr(), x) as usize }
    }
    pub(crate) fn button(&self, r: Rect, label: &str, accent: bool) -> bool {
        let l = cs(label);
        // SAFETY: live ui_ctx, NUL-terminated label.
        unsafe { if accent { ui_button_accent(self.u, r, l.as_ptr()) } else { ui_button(self.u, r, l.as_ptr()) } }
    }
    pub(crate) fn pointer(&self) -> (i32, i32, bool, i32) {
        let (mut x, mut y, mut p, mut w) = (0, 0, false, 0);
        // SAFETY: live ui_ctx.
        unsafe { ui_pointer(self.u, &mut x, &mut y, &mut p, &mut w) };
        (x, y, p, w)
    }
    /// Next queued input: (key or char, mods, is_char).
    pub(crate) fn input(&self) -> Option<(u32, u32, bool)> {
        let (mut k, mut m, mut c) = (0, 0, false);
        // SAFETY: live ui_ctx.
        unsafe { ui_next_input(self.u, &mut k, &mut m, &mut c) }.then_some((k, m, c))
    }
    pub(crate) fn blur(&self) {
        // SAFETY: live ui_ctx.
        unsafe { ui_blur(self.u) }
    }
    pub(crate) fn focus(&self, id: u32) {
        // SAFETY: live ui_ctx.
        unsafe { ui_focus(self.u, id) }
    }
    pub(crate) fn focused(&self, id: u32) -> bool {
        // SAFETY: live ui_ctx.
        unsafe { ui_focused(self.u, id) }
    }
    /// Pop a keyboard shortcut (lowercase key + required modifiers).
    pub(crate) fn take_key(&self, key: u32, need_mods: u32) -> bool {
        // SAFETY: live ui_ctx.
        unsafe { ui_take_key(self.u, key, need_mods) }
    }
    pub(crate) fn textfield_id(placeholder: &str, r: Rect) -> u32 {
        let p = cs(placeholder);
        // SAFETY: NUL-terminated placeholder.
        unsafe { ui_textfield_id(p.as_ptr(), r) }
    }
    /// Draw an editable single-line field; `buf` must stay NUL-terminated.
    pub(crate) fn textfield(&self, r: Rect, buf: &mut [u8], caret: &mut i32, placeholder: &str) -> bool {
        let p = cs(placeholder);
        // SAFETY: buf has stated capacity and is NUL-terminated by the caller; live ui_ctx.
        unsafe { ui_textfield(self.u, r, buf.as_mut_ptr().cast(), buf.len(), caret, p.as_ptr()) }
    }
}

fn face(size: i32, bold: bool) -> ShellFont {
    let face = if bold { core::ptr::addr_of!(font_data::loos_font_semibold) } else { core::ptr::addr_of!(font_data::loos_font_regular) };
    ShellFont { face, size }
}

/// `shell_backend` seen through the `be_*` C wrappers.
pub(crate) struct BeHost(pub(crate) *const c_void);

impl term::Host for BeHost {
    fn list(&self, path: &str) -> Option<Vec<(String, bool, u64)>> {
        let mut buf: Vec<FsEntry> = (0..256).map(|_| FsEntry { name: [0; 64], is_dir: false, size: 0, mtime: 0, attrs: 0 }).collect();
        // SAFETY: buf holds 256 entries; backend pointer is the desktop's.
        let n = unsafe { be_fs_list(self.0, cs(path).as_ptr(), buf.as_mut_ptr(), 256) };
        (n >= 0).then(|| {
            buf.iter().take(n as usize).map(|e| {
                let len = e.name.iter().position(|&b| b == 0).unwrap_or(64);
                (String::from_utf8_lossy(&e.name[..len]).into_owned(), e.is_dir, e.size)
            }).collect()
        })
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let mut buf = alloc::vec![0u8; 65536];
        // SAFETY: buf has its stated capacity.
        let n = unsafe { be_fs_read(self.0, cs(path).as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        (n >= 0).then(|| {
            buf.truncate(n as usize);
            buf
        })
    }
    fn write(&self, path: &str, data: &[u8]) -> bool {
        // SAFETY: data slice is valid for its length.
        unsafe { be_fs_write(self.0, cs(path).as_ptr(), data.as_ptr().cast(), data.len()) }
    }
    fn mkdir(&self, path: &str) -> bool {
        // SAFETY: NUL-terminated path.
        unsafe { be_fs_mkdir(self.0, cs(path).as_ptr()) }
    }
    fn remove(&self, path: &str) -> bool {
        // SAFETY: NUL-terminated path.
        unsafe { be_fs_remove(self.0, cs(path).as_ptr()) }
    }
    fn processes(&self) -> String {
        let mut buf = alloc::vec![0u8; 8192];
        // SAFETY: buf has its stated capacity.
        unsafe { be_ps_list(self.0, buf.as_mut_ptr().cast(), buf.len()) };
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..len]).into_owned()
    }
}

pub(crate) const MOD_CTRL: u32 = 2;
const LINE: i32 = 20;

// ---------------------------------------------------------------- Notepad

/// Notepad state.
pub struct Notepad {
    be: BeHost,
    ed: editor::Editor,
    path: [u8; 160],
    path_caret: i32,
    focus_text: bool,
    status: String,
}

#[no_mangle]
pub extern "C" fn loos_notepad_new(be: *const c_void, path: *const c_char) -> *mut Notepad {
    let mut n = Box::new(Notepad { be: BeHost(be), ed: editor::Editor::default(), path: [0; 160], path_caret: 0, focus_text: true, status: String::new() });
    if !path.is_null() {
        // SAFETY: caller passes a NUL-terminated path.
        let p = unsafe { core::ffi::CStr::from_ptr(path) }.to_bytes();
        let k = p.len().min(159);
        n.path[..k].copy_from_slice(&p[..k]);
        n.path_caret = k as i32;
        n.open();
    }
    Box::into_raw(n)
}

impl Notepad {
    fn path_str(&self) -> String {
        let len = self.path.iter().position(|&b| b == 0).unwrap_or(0);
        let p = String::from_utf8_lossy(&self.path[..len]).into_owned();
        if p.len() >= 2 && p.as_bytes()[1] == b':' { loos_vfs::winpath::to_loos(&p).unwrap_or(p) } else { p }
    }
    fn open(&mut self) {
        let p = self.path_str();
        match term::Host::read(&self.be, &p) {
            Some(d) => { self.ed.load(&d); self.status = alloc::format!("Opened {}", p); }
            None => self.status = alloc::format!("Cannot open {}", p),
        }
    }
    fn save(&mut self) {
        let p = self.path_str();
        if term::Host::write(&self.be, &p, self.ed.text().as_bytes()) {
            self.ed.dirty = false;
            self.status = alloc::format!("Saved {}", p);
        } else {
            self.status = alloc::format!("Cannot save {} (folder missing, read-only or too large)", p);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn loos_notepad_frame(app: *mut c_void, u: *mut UiCtx, w: i32, h: i32) {
    // SAFETY: app is a Notepad from loos_notepad_new; u is live for this frame.
    let (n, ui) = unsafe { (&mut *(app as *mut Notepad), Ui::new(u, w, h)) };
    let t = ui.t;
    ui.fill(ui.clip, t.surface);
    /* toolbar */
    let field = Rect::new(8, 8, w - 8 - 3 * 76 - 8, 28);
    let ph = cs("File path, e.g. /home/user/notes.txt");
    let (mx, my, pressed, wheel) = ui.pointer();
    if pressed && field.y + field.h > my { n.focus_text = true }
    // SAFETY: path buffer is 160 bytes, NUL-terminated.
    if unsafe { ui_textfield(u, field, n.path.as_mut_ptr().cast(), n.path.len(), &mut n.path_caret, ph.as_ptr()) } { n.open() }
    let bx = field.x + field.w + 8;
    if ui.button(Rect::new(bx, 8, 70, 28), "Open", false) { n.open() }
    if ui.button(Rect::new(bx + 76, 8, 70, 28), "Save", true) { n.save() }
    if ui.button(Rect::new(bx + 152, 8, 70, 28), "New", false) { n.ed = editor::Editor::default(); n.status = "New document".into() }
    /* text area */
    let area = Rect::new(0, 44, w, h - 44 - 26);
    ui.fill(area, t.surface_hi);
    let page = (area.h / LINE).max(1) as usize;
    if pressed && area.y <= my && my < area.y + area.h {
        n.focus_text = false;
        ui.blur();
        n.ed.row = ((n.ed.top as i32 + (my - area.y - 6) / LINE).max(0) as usize).min(n.ed.lines.len() - 1);
        n.ed.col = Ui::index_at(&n.ed.lines[n.ed.row], mx - 12);
    }
    if wheel != 0 {
        n.ed.top = (n.ed.top as i32 - wheel * 3).clamp(0, n.ed.lines.len() as i32 - 1) as usize;
    }
    while let Some((k, mods, is_char)) = ui.input() {
        if n.focus_text { continue }
        if mods & MOD_CTRL != 0 && (k == 's' as u32 || k == 'S' as u32) {
            n.save();
        } else if is_char {
            if let Some(c) = char::from_u32(k).filter(|c| !c.is_control()) { n.ed.insert(c) }
        } else {
            n.ed.key(k, page);
        }
        n.ed.scroll_to_caret(page);
    }
    for (i, line) in n.ed.lines.iter().enumerate().skip(n.ed.top).take(page) {
        let y = area.y + 6 + (i - n.ed.top) as i32 * LINE;
        ui.text(12, y, line, t.text);
        if i == n.ed.row && !n.focus_text {
            let cx = 12 + Ui::width(&line[..n.ed.col]);
            ui.fill(Rect::new(cx, y, 2, 16), t.accent);
        }
    }
    /* status bar */
    let sb = Rect::new(0, h - 26, w, 26);
    ui.fill(sb, t.surface);
    let st = alloc::format!("Ln {}, Col {}   {}   UTF-8   {}", n.ed.row + 1, n.ed.col + 1, if n.ed.dirty { "Modified" } else { "Saved" }, n.status);
    ui.text(12, sb.y + 5, &st, t.text_dim);
}

// ---------------------------------------------------------------- LoCmd

/// Terminal window state.
pub struct Terminal {
    be: BeHost,
    t: term::Term,
    scroll: i32,
}

#[no_mangle]
pub extern "C" fn loos_term_new(be: *const c_void) -> *mut Terminal {
    Box::into_raw(Box::new(Terminal { be: BeHost(be), t: term::Term::new("/home/user"), scroll: 0 }))
}

/// True once the user typed EXIT (the host closes the window and resets this).
#[no_mangle]
pub unsafe extern "C" fn loos_term_take_exit(app: *mut c_void) -> bool {
    // SAFETY: app is a Terminal.
    let t = unsafe { &mut *(app as *mut Terminal) };
    core::mem::take(&mut t.t.exit)
}

#[no_mangle]
pub unsafe extern "C" fn loos_term_frame(app: *mut c_void, u: *mut UiCtx, w: i32, h: i32) {
    // SAFETY: app is a Terminal from loos_term_new; u is live for this frame.
    let (s, ui) = unsafe { (&mut *(app as *mut Terminal), Ui::new(u, w, h)) };
    const BG: u32 = 0xFF0C0C0C;
    const FG: u32 = 0xFFCCCCCC;
    ui.fill(ui.clip, BG);
    let (_, _, _, wheel) = ui.pointer();
    s.scroll = (s.scroll + wheel * 3).max(0);
    while let Some((k, _mods, is_char)) = ui.input() {
        s.scroll = 0;
        if is_char {
            if let Some(c) = char::from_u32(k).filter(|c| !c.is_control()) { s.t.input.push(c) }
            continue;
        }
        match k {
            editor::key::ENTER => s.t.enter(&s.be),
            editor::key::BACKSPACE => { s.t.input.pop(); }
            editor::key::UP => s.t.recall(-1),
            editor::key::DOWN => s.t.recall(1),
            _ => {}
        }
    }
    let rows = ((h - 12) / LINE).max(1) as usize;
    let total = s.t.out.len() + 1;
    s.scroll = s.scroll.min(total.saturating_sub(rows) as i32);
    let first = total.saturating_sub(rows + s.scroll as usize);
    let mut y = 6;
    for line in s.t.out.iter().skip(first).take(rows) {
        ui.text(8, y, line, FG);
        y += LINE;
    }
    if first + rows >= total {
        let p = s.t.prompt() + &s.t.input;
        let x = 8 + ui.text(8, y, &p, 0xFFFFFFFF);
        ui.fill(Rect::new(x + 1, y + 13, 9, 2), FG);                       /* underline caret */
    }
}

// ---------------------------------------------------------------- Calculator

/// Calculator state.
pub struct Calculator {
    expr: String,
    result: String,
}

#[no_mangle]
pub extern "C" fn loos_calc_new() -> *mut Calculator {
    Box::into_raw(Box::new(Calculator { expr: String::new(), result: "0".into() }))
}

impl Calculator {
    fn press(&mut self, k: &str) {
        match k {
            "C" => { self.expr.clear(); self.result = "0".into(); }
            "\u{232b}" => { self.expr.pop(); }
            "=" => {
                self.result = match calc::eval(&self.expr) {
                    Ok(v) => { let r = calc::format(v); self.expr = r.clone(); r }
                    Err(calc::CalcError::DivZero) => "Cannot divide by zero".into(),
                    Err(_) => "Invalid input".into(),
                };
            }
            "\u{b1}" => self.expr = alloc::format!("-({})", self.expr),
            k => self.expr.push_str(k),
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn loos_calc_frame(app: *mut c_void, u: *mut UiCtx, w: i32, h: i32) {
    // SAFETY: app is a Calculator from loos_calc_new; u is live for this frame.
    let (c, ui) = unsafe { (&mut *(app as *mut Calculator), Ui::new(u, w, h)) };
    let t = ui.t;
    ui.fill(ui.clip, t.surface);
    while let Some((k, _, is_char)) = ui.input() {
        match (is_char, k) {
            (true, k) => match char::from_u32(k) {
                Some(ch @ ('0'..='9' | '+' | '-' | '*' | '/' | '%' | '(' | ')' | '.')) => c.expr.push(ch),
                Some('=') => c.press("="),
                _ => {}
            },
            (false, editor::key::ENTER) => c.press("="),
            (false, editor::key::BACKSPACE) => c.press("\u{232b}"),
            (false, 27) => c.press("C"),
            _ => {}
        }
    }
    let disp = Rect::new(12, 12, w - 24, 76);
    ui.round(disp, 8, t.surface_hi);
    let e = if c.expr.is_empty() { "0" } else { &c.expr };
    ui.text(disp.x + disp.w - 12 - Ui::width(e), disp.y + 12, e, t.text_dim);
    ui.text(disp.x + disp.w - 12 - Ui::width(&c.result), disp.y + 44, &c.result, t.text);
    const KEYS: [&str; 20] = ["C", "(", ")", "/", "7", "8", "9", "*", "4", "5", "6", "-", "1", "2", "3", "+", "\u{b1}", "0", ".", "="];
    let (gx, gy) = (12, disp.y + disp.h + 12);
    let (bw, bh) = ((w - 24 - 3 * 6) / 4, ((h - gy - 12 - 4 * 6) / 5).max(24));
    for (i, k) in KEYS.iter().enumerate() {
        let r = Rect::new(gx + (i as i32 % 4) * (bw + 6), gy + (i as i32 / 4) * (bh + 6), bw, bh);
        if ui.button(r, k, *k == "=") { c.press(k) }
    }
}

