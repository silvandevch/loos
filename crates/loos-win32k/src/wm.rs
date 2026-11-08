//! Window classes, window lifecycle, message queues, painting protocol.

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::*;

/// NC frame: border thickness and caption height of captioned windows.
pub const BORDER: i32 = 2;
/// Caption bar height.
pub const CAPTION_H: i32 = 24;
const MAX_DIM: i32 = 16384;
const MAX_PIXELS: i64 = 64 << 20;

/// Result of `get_message`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GetMsg {
    /// A message to dispatch.
    Message(Msg),
    /// WM_QUIT with exit code (GetMessage returns 0).
    Quit(i32),
    /// Nothing pending: the thread would block until input/timer/post.
    Idle,
}

/// `PAINTSTRUCT`.
#[derive(Clone, Copy, Debug)]
pub struct PaintStruct {
    /// DC clipped to the update rect.
    pub hdc: Hdc,
    /// Background still needs erasing.
    pub erase: bool,
    /// Update rect (client coords).
    pub rc: Rect,
}

/// Menu-bar height.
pub const MENU_H: i32 = 22;

fn insets(style: u32, menu: bool) -> (i32, i32, i32, i32) {
    if style & WS_CHILD == 0 && style & WS_CAPTION == WS_CAPTION {
        (BORDER, BORDER + CAPTION_H + if menu { MENU_H } else { 0 }, BORDER, BORDER)
    } else if style & WS_BORDER != 0 {
        (1, 1, 1, 1)
    } else {
        (0, 0, 0, 0)
    }
}

/// `AdjustWindowRect`: the outer rectangle whose client area is `client`, for a window `style` (with or without a menu bar).
pub fn adjust_window_rect(style: u32, menu: bool, client: Rect) -> Rect {
    let (l, t, r, b) = insets(style, menu);
    Rect::new(client.left - l, client.top - t, client.right + r, client.bottom + b)
}

impl Win32k {
    // ---------- classes ----------

    /// `RegisterClassEx`. Returns the atom.
    pub fn register_class(&mut self, def: WndClass) -> Result<u16, Win32kError> {
        if self.classes.iter().any(|c| c.def.name.eq_ignore_ascii_case(&def.name)) {
            return self.fail(Win32kError::ClassExists);
        }
        let atom = 0xC000 + self.classes.len() as u16;
        self.classes.push(Class { def, atom });
        Ok(atom)
    }

    /// `UnregisterClass` (fails with `InvalidHandle` while windows use it).
    pub fn unregister_class(&mut self, name: &str) -> Result<(), Win32kError> {
        let Some(i) = self.classes.iter().position(|c| c.def.name.eq_ignore_ascii_case(name)) else {
            return self.fail(Win32kError::NoClass);
        };
        if self.wins.values().any(|w| w.class == i) {
            return self.fail(Win32kError::InvalidHandle);
        }
        // Keep indices stable: neutralise instead of removing.
        self.classes[i].def.name = format!("\u{0}dead{i}");
        Ok(())
    }

    // ---------- geometry helpers ----------

    pub(crate) fn win(&self, h: Hwnd) -> Result<&Window, Win32kError> {
        self.wins.get(&h).ok_or(Win32kError::InvalidWindow)
    }

    /// `IsWindow`.
    pub fn is_window(&self, h: Hwnd) -> bool {
        self.wins.contains_key(&h)
    }

    /// Client origin in screen coordinates.
    pub(crate) fn client_origin(&self, h: Hwnd) -> Point {
        let Some(w) = self.wins.get(&h) else { return Point::default() };
        let (l, t, _, _) = insets(w.style, !w.menu.is_empty());
        if w.parent == 0 {
            Point { x: w.x + l, y: w.y + t }
        } else {
            let p = self.client_origin(w.parent);
            Point { x: p.x + w.x + l, y: p.y + w.y + t }
        }
    }

    /// `GetWindowRect` (screen coords).
    pub fn get_window_rect(&self, h: Hwnd) -> Result<Rect, Win32kError> {
        let w = self.win(h)?;
        let o = if w.parent == 0 { Point::default() } else { self.client_origin(w.parent) };
        Ok(Rect::new(o.x + w.x, o.y + w.y, o.x + w.x + w.w, o.y + w.y + w.h))
    }

    /// `GetClientRect` (origin 0,0).
    pub fn get_client_rect(&self, h: Hwnd) -> Result<Rect, Win32kError> {
        let w = self.win(h)?;
        Ok(Rect::new(0, 0, w.cw, w.ch))
    }

    /// `ClientToScreen`.
    pub fn client_to_screen(&self, h: Hwnd, p: Point) -> Result<Point, Win32kError> {
        self.win(h)?;
        let o = self.client_origin(h);
        Ok(Point { x: p.x + o.x, y: p.y + o.y })
    }

    /// `ScreenToClient`.
    pub fn screen_to_client(&self, h: Hwnd, p: Point) -> Result<Point, Win32kError> {
        self.win(h)?;
        let o = self.client_origin(h);
        Ok(Point { x: p.x - o.x, y: p.y - o.y })
    }

    pub(crate) fn relayout(&mut self, h: Hwnd) -> Result<(), Win32kError> {
        let w = self.wins.get_mut(&h).ok_or(Win32kError::InvalidWindow)?;
        let (l, t, r, b) = insets(w.style, !w.menu.is_empty());
        let cw = (w.w - l - r).max(0);
        let ch = (w.h - t - b).max(0);
        if cw > MAX_DIM || ch > MAX_DIM || cw as i64 * ch as i64 > MAX_PIXELS {
            return Err(Win32kError::TooLarge);
        }
        if cw != w.cw || ch != w.ch || w.bmp.len() != (cw * ch) as usize {
            w.cw = cw;
            w.ch = ch;
            w.bmp = vec![0xFFFF_FFFF; (cw * ch) as usize];
        }
        w.invalid = Some(Rect::new(0, 0, cw, ch));
        w.erase = true;
        Ok(())
    }

    // ---------- creation / destruction ----------

    /// `CreateWindowEx`. Sends WM_NCCREATE, WM_CREATE, WM_SIZE, WM_MOVE; shows if WS_VISIBLE.
    #[allow(clippy::too_many_arguments)]
    pub fn create_window_ex(
        &mut self,
        exstyle: u32,
        class: &str,
        title: &str,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Hwnd,
        id: isize,
        create_param: isize,
    ) -> Result<Hwnd, Win32kError> {
        let Some(ci) = self.classes.iter().position(|c| c.def.name.eq_ignore_ascii_case(class)) else {
            return self.fail(Win32kError::NoClass);
        };
        if parent != 0 && !self.is_window(parent) {
            return self.fail(Win32kError::InvalidWindow);
        }
        if style & WS_CHILD != 0 && parent == 0 {
            return self.fail(Win32kError::InvalidWindow);
        }
        let (mut x, mut y, mut w, mut h) = (x, y, w, h);
        if x == CW_USEDEFAULT || y == CW_USEDEFAULT {
            x = 40 + self.cascade;
            y = 40 + self.cascade;
            self.cascade = (self.cascade + 28) % 200;
        }
        if w == CW_USEDEFAULT || h == CW_USEDEFAULT {
            w = (self.screen.0 * 3 / 5).max(160);
            h = (self.screen.1 * 3 / 5).max(120);
        }
        let hwnd = self.next_hwnd;
        self.next_hwnd += 2;
        let cs = CreateStruct {
            create_param,
            parent,
            id,
            cy: h,
            cx: w,
            y,
            x,
            style,
            exstyle,
            name: title.to_string(),
        };
        let def = self.classes[ci].def.clone();
        let win = Window {
            class: ci,
            parent,
            owner: 0,
            children: Vec::new(),
            title: title.to_string(),
            style: style & !WS_VISIBLE,
            exstyle,
            x,
            y,
            w: w.max(0),
            h: h.max(0),
            minimized: false,
            maximized: false,
            restore: None,
            tid: self.cur_tid,
            proc_: def.proc_,
            userdata: 0,
            id,
            extra: vec![0; def.wnd_extra],
            invalid: None,
            erase: false,
            bmp: Vec::new(),
            cw: 0,
            ch: 0,
            create: cs,
            menu: Vec::new(),
        };
        self.wins.insert(hwnd, win);
        if let Err(e) = self.relayout(hwnd) {
            self.wins.remove(&hwnd);
            return self.fail(e);
        }
        if let Some(w) = self.wins.get_mut(&hwnd) {
            w.invalid = None;
        }
        if parent == 0 {
            self.tops.push(hwnd);
        } else if let Some(p) = self.wins.get_mut(&parent) {
            p.children.push(hwnd);
        }
        if self.send_message(hwnd, WM_NCCREATE, 0, hwnd as isize) == 0
            || self.send_message(hwnd, WM_CREATE, 0, hwnd as isize) == -1
            || !self.is_window(hwnd)
        {
            if self.is_window(hwnd) {
                self.destroy_window(hwnd).ok();
            }
            return self.fail(Win32kError::CreateFailed);
        }
        let (cw, ch, wx, wy) = {
            let w = &self.wins[&hwnd];
            (w.cw, w.ch, w.x, w.y)
        };
        self.send_message(hwnd, WM_SIZE, 0, make_lparam(cw, ch));
        self.send_message(hwnd, WM_MOVE, 0, make_lparam(wx, wy));
        if style & WS_VISIBLE != 0 {
            self.show_window(hwnd, SW_SHOW).ok();
        }
        Ok(hwnd)
    }

    /// The CREATESTRUCT of a window (valid while handling WM_NCCREATE/WM_CREATE; kept for its lifetime).
    pub fn create_struct(&self, h: Hwnd) -> Result<&CreateStruct, Win32kError> {
        Ok(&self.win(h)?.create)
    }

    /// `DestroyWindow`: children first; WM_DESTROY then WM_NCDESTROY.
    pub fn destroy_window(&mut self, h: Hwnd) -> Result<(), Win32kError> {
        if !self.is_window(h) {
            return self.fail(Win32kError::InvalidWindow);
        }
        let kids: Vec<Hwnd> = self.wins[&h].children.clone();
        if self.focus == h || self.is_ancestor(h, self.focus) {
            let old = self.focus;
            self.focus = 0;
            self.send_message(old, WM_KILLFOCUS, 0, 0);
        }
        if self.active == h {
            self.active = 0;
        }
        if self.capture == h {
            self.capture = 0;
        }
        self.wins.get_mut(&h).map(|w| w.style &= !WS_VISIBLE);
        self.send_message(h, WM_DESTROY, 0, 0);
        for k in kids {
            if self.is_window(k) {
                self.destroy_window(k).ok();
            }
        }
        self.send_message(h, WM_NCDESTROY, 0, 0);
        let w = self.wins.remove(&h);
        if let Some(w) = w {
            if w.parent == 0 {
                self.tops.retain(|&t| t != h);
            } else if let Some(p) = self.wins.get_mut(&w.parent) {
                p.children.retain(|&c| c != h);
            }
        }
        self.timers.retain(|t| t.hwnd != h);
        for q in self.queues.values_mut() {
            q.posted.retain(|m| m.hwnd != h);
        }
        self.dcs.retain(|_, d| !matches!(d.kind, DcKind::Window(w) if w == h));
        if self.active == 0 {
            if let Some(&top) = self.tops.iter().rev().find(|&&t| self.wins[&t].style & WS_VISIBLE != 0) {
                self.set_active_window(top);
            }
        }
        Ok(())
    }

    pub(crate) fn is_ancestor(&self, anc: Hwnd, mut h: Hwnd) -> bool {
        while let Some(w) = self.wins.get(&h) {
            if w.parent == anc {
                return true;
            }
            h = w.parent;
        }
        false
    }

    // ---------- visibility / geometry changes ----------

    /// `IsWindowVisible` (self and all ancestors visible, not minimised).
    pub fn is_window_visible(&self, mut h: Hwnd) -> bool {
        while let Some(w) = self.wins.get(&h) {
            if w.style & WS_VISIBLE == 0 || w.minimized {
                return false;
            }
            h = w.parent;
        }
        h == 0
    }

    /// `ShowWindow`. Returns whether it was previously visible.
    pub fn show_window(&mut self, h: Hwnd, cmd: i32) -> Result<bool, Win32kError> {
        if !self.is_window(h) {
            return self.fail(Win32kError::InvalidWindow);
        }
        let was = self.wins[&h].style & WS_VISIBLE != 0;
        let (screen_w, screen_h) = self.screen;
        match cmd {
            SW_HIDE => {
                self.wins.get_mut(&h).unwrap().style &= !WS_VISIBLE;
                if was {
                    self.send_message(h, WM_SHOWWINDOW, 0, 0);
                }
                if self.active == h {
                    self.active = 0;
                    if let Some(&t) = self.tops.iter().rev().find(|&&t| t != h && self.is_window_visible(t)) {
                        self.set_active_window(t);
                    }
                }
                if self.focus == h || self.is_ancestor(h, self.focus) {
                    self.focus = 0;
                }
            }
            SW_SHOWMINIMIZED | SW_MINIMIZE => {
                let w = self.wins.get_mut(&h).unwrap();
                w.style |= WS_VISIBLE;
                w.minimized = true;
                self.send_message(h, WM_SIZE, 1, 0);
            }
            SW_SHOWMAXIMIZED => {
                let w = self.wins.get_mut(&h).unwrap();
                w.style |= WS_VISIBLE;
                w.minimized = false;
                if !w.maximized {
                    w.restore = Some((w.x, w.y, w.w, w.h));
                    w.maximized = true;
                    w.x = 0;
                    w.y = 0;
                    w.w = screen_w;
                    w.h = screen_h;
                }
                self.relayout(h).ok();
                let (cw, ch) = (self.wins[&h].cw, self.wins[&h].ch);
                self.send_message(h, WM_SIZE, 2, make_lparam(cw, ch));
                self.send_message(h, WM_SHOWWINDOW, 1, 0);
            }
            SW_RESTORE => {
                let w = self.wins.get_mut(&h).unwrap();
                w.style |= WS_VISIBLE;
                w.minimized = false;
                if let (true, Some((x, y, ww, hh))) = (w.maximized, w.restore.take()) {
                    w.maximized = false;
                    w.x = x;
                    w.y = y;
                    w.w = ww;
                    w.h = hh;
                    self.relayout(h).ok();
                }
                let (cw, ch) = (self.wins[&h].cw, self.wins[&h].ch);
                self.send_message(h, WM_SIZE, 0, make_lparam(cw, ch));
                self.send_message(h, WM_SHOWWINDOW, 1, 0);
            }
            _ => {
                let w = self.wins.get_mut(&h).unwrap();
                w.style |= WS_VISIBLE;
                w.minimized = false;
                if !was {
                    self.send_message(h, WM_SHOWWINDOW, 1, 0);
                }
            }
        }
        if cmd != SW_HIDE && cmd != SW_SHOWMINIMIZED && cmd != SW_MINIMIZE {
            let (parent, top) = (self.wins[&h].parent, self.wins[&h].parent == 0);
            if top {
                self.raise(h);
                if cmd != SW_SHOWNORMAL || self.active != h {
                    self.set_active_window(h);
                }
            } else {
                let _ = parent;
            }
            self.invalidate_all(h);
        }
        Ok(was)
    }

    fn invalidate_all(&mut self, h: Hwnd) {
        if let Some(w) = self.wins.get_mut(&h) {
            w.invalid = Some(Rect::new(0, 0, w.cw, w.ch));
            w.erase = true;
            let kids = w.children.clone();
            for k in kids {
                self.invalidate_all(k);
            }
        }
    }

    pub(crate) fn raise(&mut self, h: Hwnd) {
        if self.wins.get(&h).map(|w| w.parent) == Some(0) {
            self.tops.retain(|&t| t != h);
            self.tops.push(h);
        }
    }

    /// `MoveWindow` (coordinates relative to the parent client area / screen for top-levels).
    pub fn move_window(&mut self, h: Hwnd, x: i32, y: i32, w: i32, hh: i32) -> Result<(), Win32kError> {
        if !self.is_window(h) {
            return self.fail(Win32kError::InvalidWindow);
        }
        let (ow, oh, ox, oy) = {
            let win = &self.wins[&h];
            (win.w, win.h, win.x, win.y)
        };
        {
            let win = self.wins.get_mut(&h).unwrap();
            win.x = x;
            win.y = y;
            win.w = w.max(0);
            win.h = hh.max(0);
        }
        if (w, hh) != (ow, oh) {
            if let Err(e) = self.relayout(h) {
                let win = self.wins.get_mut(&h).unwrap();
                win.x = ox;
                win.y = oy;
                win.w = ow;
                win.h = oh;
                return self.fail(e);
            }
            let (cw, ch) = (self.wins[&h].cw, self.wins[&h].ch);
            self.send_message(h, WM_SIZE, 0, make_lparam(cw, ch));
        }
        if (x, y) != (ox, oy) {
            self.send_message(h, WM_MOVE, 0, make_lparam(x, y));
        }
        Ok(())
    }

    /// `SetWindowText`.
    pub fn set_window_text(&mut self, h: Hwnd, text: &str) -> Result<(), Win32kError> {
        self.win(h)?;
        self.send_message(h, WM_SETTEXT, 0, 0);
        self.wins.get_mut(&h).unwrap().title = text.to_string();
        Ok(())
    }

    /// `GetWindowText`.
    pub fn get_window_text(&self, h: Hwnd) -> Result<String, Win32kError> {
        Ok(self.win(h)?.title.clone())
    }

    /// `GetWindowLongPtr(GWLP_USERDATA)`.
    pub fn get_user_data(&self, h: Hwnd) -> Result<isize, Win32kError> {
        Ok(self.win(h)?.userdata)
    }
    /// `SetWindowLongPtr(GWLP_USERDATA)`; returns the previous value.
    pub fn set_user_data(&mut self, h: Hwnd, v: isize) -> Result<isize, Win32kError> {
        self.win(h)?;
        Ok(core::mem::replace(&mut self.wins.get_mut(&h).unwrap().userdata, v))
    }
    /// `SetWindowLongPtr(GWLP_WNDPROC)` (subclassing); returns the previous procedure.
    pub fn set_window_proc(&mut self, h: Hwnd, p: Proc) -> Result<Proc, Win32kError> {
        self.win(h)?;
        Ok(core::mem::replace(&mut self.wins.get_mut(&h).unwrap().proc_, p))
    }
    /// `GetWindowLongPtr(index >= 0)` extra bytes.
    pub fn get_extra(&self, h: Hwnd, i: usize) -> Result<isize, Win32kError> {
        self.win(h)?.extra.get(i).copied().ok_or(Win32kError::InvalidHandle)
    }
    /// `SetWindowLongPtr(index >= 0)` extra bytes.
    pub fn set_extra(&mut self, h: Hwnd, i: usize, v: isize) -> Result<isize, Win32kError> {
        self.win(h)?;
        match self.wins.get_mut(&h).unwrap().extra.get_mut(i) {
            Some(s) => Ok(core::mem::replace(s, v)),
            None => self.fail(Win32kError::InvalidHandle),
        }
    }
    /// `GetWindowLong(GWL_STYLE)`.
    pub fn get_style(&self, h: Hwnd) -> Result<u32, Win32kError> {
        Ok(self.win(h)?.style)
    }
    /// `GetDlgCtrlID` / `GWLP_ID`.
    pub fn get_id(&self, h: Hwnd) -> Result<isize, Win32kError> {
        Ok(self.win(h)?.id)
    }
    /// `GetParent`.
    pub fn get_parent(&self, h: Hwnd) -> Result<Hwnd, Win32kError> {
        Ok(self.win(h)?.parent)
    }
    /// Child windows, bottom to top.
    pub fn children(&self, h: Hwnd) -> Vec<Hwnd> {
        self.wins.get(&h).map(|w| w.children.clone()).unwrap_or_default()
    }
    /// Top-level windows, bottom to top.
    pub fn top_levels(&self) -> Vec<Hwnd> {
        self.tops.clone()
    }
    /// `FindWindow(class, title)`.
    pub fn find_window(&self, class: Option<&str>, title: Option<&str>) -> Hwnd {
        for &t in self.tops.iter().rev() {
            let w = &self.wins[&t];
            let cn = &self.classes[w.class].def.name;
            if class.is_none_or(|c| cn.eq_ignore_ascii_case(c)) && title.is_none_or(|s| w.title == s) {
                return t;
            }
        }
        0
    }
    /// `EnableWindow`; returns previous disabled state.
    pub fn enable_window(&mut self, h: Hwnd, enable: bool) -> Result<bool, Win32kError> {
        self.win(h)?;
        let w = self.wins.get_mut(&h).unwrap();
        let was_disabled = w.style & WS_DISABLED != 0;
        if enable {
            w.style &= !WS_DISABLED;
        } else {
            w.style |= WS_DISABLED;
        }
        if was_disabled == enable {
            self.send_message(h, WM_ENABLE, enable as usize, 0);
        }
        Ok(was_disabled)
    }
    /// `IsWindowEnabled`.
    pub fn is_window_enabled(&self, h: Hwnd) -> bool {
        self.wins.get(&h).is_some_and(|w| w.style & WS_DISABLED == 0)
    }

    // ---------- focus / activation / capture ----------

    /// `SetActiveWindow` (top-level only); also raises and focuses it.
    pub fn set_active_window(&mut self, h: Hwnd) -> Hwnd {
        let Some(w) = self.wins.get(&h) else { return 0 };
        let top = if w.parent == 0 { h } else { self.root_of(h) };
        let prev = self.active;
        if prev == top {
            return prev;
        }
        if prev != 0 && self.is_window(prev) {
            self.send_message(prev, WM_ACTIVATE, 0, top as isize);
        }
        self.active = top;
        self.raise(top);
        self.send_message(top, WM_ACTIVATE, 1, prev as isize);
        if self.active == top {
            self.set_focus(top);
        }
        prev
    }

    pub(crate) fn root_of(&self, mut h: Hwnd) -> Hwnd {
        while let Some(w) = self.wins.get(&h) {
            if w.parent == 0 {
                return h;
            }
            h = w.parent;
        }
        0
    }

    /// `SetFocus`; returns previous focus.
    pub fn set_focus(&mut self, h: Hwnd) -> Hwnd {
        if !self.is_window(h) || !self.is_window_enabled(h) {
            return self.focus;
        }
        let prev = self.focus;
        if prev == h {
            return prev;
        }
        if prev != 0 && self.is_window(prev) {
            self.send_message(prev, WM_KILLFOCUS, h as usize, 0);
        }
        self.focus = h;
        self.send_message(h, WM_SETFOCUS, prev as usize, 0);
        prev
    }
    /// `GetFocus`.
    pub fn get_focus(&self) -> Hwnd {
        self.focus
    }
    /// `GetActiveWindow` / `GetForegroundWindow`.
    pub fn get_active_window(&self) -> Hwnd {
        self.active
    }
    /// `SetCapture`; returns previous.
    pub fn set_capture(&mut self, h: Hwnd) -> Hwnd {
        let n = if self.is_window(h) { h } else { 0 };
        core::mem::replace(&mut self.capture, n)
    }
    /// `ReleaseCapture`.
    pub fn release_capture(&mut self) {
        self.capture = 0;
    }
    /// `GetCapture`.
    pub fn get_capture(&self) -> Hwnd {
        self.capture
    }
    /// `BringWindowToTop`.
    pub fn bring_window_to_top(&mut self, h: Hwnd) -> Result<(), Win32kError> {
        self.win(h)?;
        if self.wins[&h].parent == 0 {
            self.raise(h);
        } else {
            let p = self.wins[&h].parent;
            let kids = &mut self.wins.get_mut(&p).unwrap().children;
            kids.retain(|&c| c != h);
            kids.push(h);
        }
        Ok(())
    }

    // ---------- messaging ----------

    /// Calls the window procedure directly (`CallWindowProc`).
    pub fn call_window_proc(&mut self, p: Proc, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
        match p {
            Proc::Native(f) => f(self, h, msg, w, l),
            Proc::User(addr) => match self.upcall.take() {
                Some(mut up) => {
                    let r = up(self, addr, h, msg, w, l);
                    if self.upcall.is_none() {
                        self.upcall = Some(up);
                    }
                    r
                }
                None => 0,
            },
        }
    }

    /// `SendMessage`: runs the target procedure now, on the owning thread's context.
    pub fn send_message(&mut self, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
        let Some(win) = self.wins.get(&h) else { return 0 };
        let (p, tid) = (win.proc_, win.tid);
        let saved = core::mem::replace(&mut self.cur_tid, tid);
        let r = self.call_window_proc(p, h, msg, w, l);
        self.cur_tid = saved;
        r
    }

    /// `PostMessage` to the window's thread queue.
    pub fn post_message(&mut self, h: Hwnd, msg: u32, w: usize, l: isize) -> Result<(), Win32kError> {
        let tid = self.win(h)?.tid;
        let m = Msg { hwnd: h, message: msg, wparam: w, lparam: l, time: self.tick(), pt: self.cursor };
        self.queues.entry(tid).or_default().posted.push_back(m);
        Ok(())
    }

    /// `PostThreadMessage` (hwnd = 0).
    pub fn post_thread_message(&mut self, tid: u32, msg: u32, w: usize, l: isize) {
        let m = Msg { hwnd: 0, message: msg, wparam: w, lparam: l, time: self.tick(), pt: self.cursor };
        self.queues.entry(tid).or_default().posted.push_back(m);
    }

    /// `PostQuitMessage` for the current thread.
    pub fn post_quit_message(&mut self, code: i32) {
        self.queues.entry(self.cur_tid).or_default().quit = Some(code);
    }

    fn matches(m: &Msg, hwnd: Hwnd, min: u32, max: u32) -> bool {
        (hwnd == 0 || m.hwnd == hwnd) && (max == 0 || (m.message >= min && m.message <= max))
    }

    /// `PeekMessage` on the current thread. WM_QUIT is reported as a message.
    pub fn peek_message(&mut self, hwnd: Hwnd, min: u32, max: u32, remove: bool) -> Option<Msg> {
        let tid = self.cur_tid;
        let q = self.queues.entry(tid).or_default();
        if let Some(i) = q.posted.iter().position(|m| Self::matches(m, hwnd, min, max)) {
            return if remove { q.posted.remove(i) } else { Some(q.posted[i]) };
        }
        if let Some(code) = q.quit {
            let m = Msg { message: WM_QUIT, wparam: code as usize, ..Msg::default() };
            if Self::matches(&m, hwnd, min, max) {
                if remove {
                    q.quit = None;
                }
                return Some(m);
            }
        }
        // WM_PAINT: synthesised from invalid regions.
        if Self::matches(&Msg { message: WM_PAINT, ..Msg::default() }, 0, min, max) {
            let order: Vec<Hwnd> = self.tops.iter().copied().flat_map(|t| self.subtree(t)).collect();
            for h in order {
                let w = &self.wins[&h];
                if w.tid == tid
                    && w.invalid.is_some_and(|r| !r.is_empty())
                    && self.is_window_visible(h)
                    && (hwnd == 0 || hwnd == h)
                {
                    return Some(Msg { hwnd: h, message: WM_PAINT, time: self.tick(), pt: self.cursor, ..Msg::default() });
                }
            }
        }
        // WM_TIMER.
        if Self::matches(&Msg { message: WM_TIMER, ..Msg::default() }, 0, min, max) {
            let now = self.now_ms;
            if let Some(t) = self.timers.iter_mut().find(|t| t.tid == tid && t.next <= now && (hwnd == 0 || t.hwnd == hwnd)) {
                t.next = now + t.interval.max(1);
                return Some(Msg { hwnd: t.hwnd, message: WM_TIMER, wparam: t.id, time: now as u32, pt: self.cursor, ..Msg::default() });
            }
        }
        None
    }

    fn subtree(&self, h: Hwnd) -> Vec<Hwnd> {
        let mut v = vec![h];
        if let Some(w) = self.wins.get(&h) {
            for &c in &w.children {
                v.extend(self.subtree(c));
            }
        }
        v
    }

    /// `GetMessage` (non-blocking form: `Idle` means the thread would sleep).
    pub fn get_message(&mut self, hwnd: Hwnd, min: u32, max: u32) -> GetMsg {
        match self.peek_message(hwnd, min, max, true) {
            Some(m) if m.message == WM_QUIT => GetMsg::Quit(m.wparam as i32),
            Some(m) => GetMsg::Message(m),
            None => GetMsg::Idle,
        }
    }

    /// `TranslateMessage`: posts WM_CHAR for character keys. Returns whether a char was produced.
    pub fn translate_message(&mut self, m: &Msg) -> bool {
        if m.message != WM_KEYDOWN && m.message != WM_SYSKEYDOWN {
            return false;
        }
        let shift = self.keys[VK_SHIFT as usize];
        let Some(ch) = vk_to_char(m.wparam as u32, shift) else { return false };
        if self.keys[VK_CONTROL as usize] && (ch as u32) >= 0x20 {
            return false;
        }
        self.post_message(m.hwnd, WM_CHAR, ch as usize, m.lparam).is_ok()
    }

    /// `DispatchMessage`.
    pub fn dispatch_message(&mut self, m: &Msg) -> isize {
        if m.hwnd == 0 {
            return 0;
        }
        self.send_message(m.hwnd, m.message, m.wparam, m.lparam)
    }

    // ---------- timers ----------

    /// `SetTimer`.
    pub fn set_timer(&mut self, h: Hwnd, id: usize, ms: u32) -> usize {
        let id = if id == 0 { 0x1000 + self.timers.len() } else { id };
        self.timers.retain(|t| !(t.hwnd == h && t.id == id));
        let iv = ms.max(10) as u64;
        self.timers.push(Timer { tid: self.cur_tid, hwnd: h, id, interval: iv, next: self.now_ms + iv });
        id
    }
    /// `KillTimer`.
    pub fn kill_timer(&mut self, h: Hwnd, id: usize) -> bool {
        let n = self.timers.len();
        self.timers.retain(|t| !(t.hwnd == h && t.id == id));
        n != self.timers.len()
    }
    /// Advance the system clock.
    pub fn advance_time(&mut self, ms: u64) {
        self.now_ms += ms;
    }

    // ---------- painting protocol ----------

    /// `InvalidateRect` (None = whole client).
    pub fn invalidate_rect(&mut self, h: Hwnd, r: Option<Rect>, erase: bool) -> Result<(), Win32kError> {
        self.win(h)?;
        let w = self.wins.get_mut(&h).unwrap();
        let full = Rect::new(0, 0, w.cw, w.ch);
        let r = r.map_or(full, |r| r.intersect(&full));
        if r.is_empty() {
            return Ok(());
        }
        w.invalid = Some(w.invalid.map_or(r, |o| o.union(&r)));
        w.erase |= erase;
        Ok(())
    }
    /// `ValidateRect` (whole client).
    pub fn validate_rect(&mut self, h: Hwnd) -> Result<(), Win32kError> {
        self.win(h)?;
        let w = self.wins.get_mut(&h).unwrap();
        w.invalid = None;
        w.erase = false;
        Ok(())
    }
    /// `GetUpdateRect`.
    pub fn get_update_rect(&self, h: Hwnd) -> Result<Option<Rect>, Win32kError> {
        Ok(self.win(h)?.invalid)
    }
    /// `UpdateWindow`: sends WM_PAINT now if the window has an update region.
    pub fn update_window(&mut self, h: Hwnd) -> Result<(), Win32kError> {
        self.win(h)?;
        if self.wins[&h].invalid.is_some() && self.is_window_visible(h) {
            self.send_message(h, WM_PAINT, 0, 0);
        }
        Ok(())
    }
    /// `BeginPaint`.
    pub fn begin_paint(&mut self, h: Hwnd) -> Result<PaintStruct, Win32kError> {
        self.win(h)?;
        let (rc, erase) = {
            let w = &self.wins[&h];
            (w.invalid.unwrap_or(Rect::new(0, 0, w.cw, w.ch)), w.erase)
        };
        let hdc = self.get_dc(h)?;
        self.dcs.get_mut(&hdc).unwrap().clip = Some(rc);
        let mut need_erase = false;
        if erase {
            need_erase = self.send_message(h, WM_ERASEBKGND, hdc as usize, 0) == 0;
        }
        let w = self.wins.get_mut(&h).unwrap();
        w.invalid = None;
        w.erase = false;
        Ok(PaintStruct { hdc, erase: need_erase, rc })
    }
    /// `EndPaint`.
    pub fn end_paint(&mut self, h: Hwnd, ps: &PaintStruct) {
        self.release_dc(h, ps.hdc);
    }

    // ---------- default procedure ----------

    /// `DefWindowProc`.
    pub fn def_window_proc(&mut self, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
        match msg {
            WM_NCCREATE => 1,
            WM_NCHITTEST => self.default_hit_test(h, lparam_x(l), lparam_y(l)),
            WM_ERASEBKGND => self.default_erase(h, w as Hdc),
            WM_PAINT => {
                if let Ok(ps) = self.begin_paint(h) {
                    self.end_paint(h, &ps);
                }
                0
            }
            WM_CLOSE => {
                self.destroy_window(h).ok();
                0
            }
            WM_SYSCOMMAND => {
                match w & 0xFFF0 {
                    SC_CLOSE => {
                        self.send_message(h, WM_CLOSE, 0, 0);
                    }
                    SC_MINIMIZE => {
                        self.show_window(h, SW_MINIMIZE).ok();
                    }
                    SC_MAXIMIZE => {
                        self.show_window(h, SW_SHOWMAXIMIZED).ok();
                    }
                    SC_RESTORE => {
                        self.show_window(h, SW_RESTORE).ok();
                    }
                    _ => {}
                }
                0
            }
            WM_SETTEXT | WM_GETTEXTLENGTH => {
                if msg == WM_GETTEXTLENGTH {
                    self.wins.get(&h).map_or(0, |x| x.title.chars().count() as isize)
                } else {
                    1
                }
            }
            WM_QUERYENDSESSION => 1,
            _ => 0,
        }
    }

    fn default_erase(&mut self, h: Hwnd, hdc: Hdc) -> isize {
        let Some(win) = self.wins.get(&h) else { return 0 };
        let (bg, full) = (self.classes[win.class].def.background, Rect::new(0, 0, win.cw, win.ch));
        if bg == 0 {
            return 0;
        }
        let brush = if bg <= 32 { self.sys_color_brush(bg as i32 - 1) } else { bg };
        let rc = self.dcs.get(&hdc).and_then(|d| d.clip).unwrap_or(full);
        self.fill_rect(hdc, rc, brush);
        1
    }

    fn default_hit_test(&mut self, h: Hwnd, sx: i32, sy: i32) -> isize {
        let Ok(r) = self.get_window_rect(h) else { return HTTRANSPARENT };
        let p = Point { x: sx, y: sy };
        if !r.contains(p) {
            return HTTRANSPARENT;
        }
        let w = &self.wins[&h];
        let o = self.client_origin(h);
        if Rect::new(o.x, o.y, o.x + w.cw, o.y + w.ch).contains(p) {
            return HTCLIENT;
        }
        if w.parent == 0 && w.style & WS_CAPTION == WS_CAPTION {
            if !w.menu.is_empty() && self.menu_bar_rect(h).contains(p) {
                return HTMENU;
            }
            if w.style & WS_SYSMENU != 0 && self.close_button(h).contains(p) {
                return HTCLOSE;
            }
            if Rect::new(r.left + BORDER, r.top + BORDER, r.right - BORDER, r.top + BORDER + CAPTION_H).contains(p) {
                return HTCAPTION;
            }
        }
        HTBORDER
    }

    /// Close-button rectangle in screen coordinates.
    pub fn close_button(&self, h: Hwnd) -> Rect {
        let r = self.get_window_rect(h).unwrap_or_default();
        Rect::new(r.right - BORDER - 20, r.top + BORDER + 3, r.right - BORDER - 2, r.top + BORDER + 21)
    }
}

/// US-layout VK → character.
pub fn vk_to_char(vk: u32, shift: bool) -> Option<char> {
    let c = match vk {
        0x41..=0x5A => {
            let c = (vk as u8 - 0x41 + b'a') as char;
            if shift { c.to_ascii_uppercase() } else { c }
        }
        0x30..=0x39 => {
            if shift {
                b")!@#$%^&*(".get((vk - 0x30) as usize).map(|&b| b as char)?
            } else {
                (vk as u8) as char
            }
        }
        VK_SPACE => ' ',
        VK_RETURN => '\r',
        VK_BACK => '\u{8}',
        VK_TAB => '\t',
        VK_ESCAPE => '\u{1b}',
        _ => return None,
    };
    Some(c)
}
