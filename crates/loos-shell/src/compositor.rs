//! Software compositor and window model (C `compositor.c`): z-order, focus, hit testing,
//! move/resize drag, damage, occlusion culling, wallpaper, chrome, cursor and overlays.
#![allow(missing_docs)]

use crate::draw::{blend, mix, Damage, Rect, Theme};
use crate::ffi::{self, Fb};
use crate::font_data;
use crate::raster;
use core::ffi::{c_char, c_void};

pub const SHELL_MAX_WINDOWS: usize = 128;
pub const SHELL_TITLE_MAX: usize = 96;
pub const SHELL_EVQ: usize = 64;
pub const SHELL_DESKTOPS: i32 = 8;
pub const SHELL_DAMAGE_MAX: usize = 16;

pub const WF_VISIBLE: u32 = 1;
pub const WF_MINIMIZED: u32 = 2;
pub const WF_MAXIMIZED: u32 = 4;
pub const WF_NORESIZE: u32 = 8;
pub const WF_NOCHROME: u32 = 16;
pub const WF_TOPMOST: u32 = 32;
pub const WF_NOFOCUS: u32 = 64;
pub const WF_STICKY: u32 = 128;
pub const WF_SNAPPED: u32 = 256;
pub const WF_NOTASKBAR: u32 = 512;

pub type Zone = i32;
pub mod zone {
    use super::Zone;
    pub const NONE: Zone = 0;
    pub const CLIENT: Zone = 1;
    pub const TITLE: Zone = 2;
    pub const CLOSE: Zone = 3;
    pub const MAXIMIZE: Zone = 4;
    pub const MINIMIZE: Zone = 5;
    pub const EDGE_L: Zone = 6;
    pub const EDGE_R: Zone = 7;
    pub const EDGE_T: Zone = 8;
    pub const EDGE_B: Zone = 9;
    pub const CORNER_TL: Zone = 10;
    pub const CORNER_TR: Zone = 11;
    pub const CORNER_BL: Zone = 12;
    pub const CORNER_BR: Zone = 13;
}

pub mod ev {
    pub const NONE: i32 = 0;
    pub const MOUSE_MOVE: i32 = 1;
    pub const MOUSE_DOWN: i32 = 2;
    pub const MOUSE_UP: i32 = 3;
    pub const MOUSE_WHEEL: i32 = 4;
    pub const KEY_DOWN: i32 = 5;
    pub const KEY_UP: i32 = 6;
    pub const CHAR: i32 = 7;
    pub const FOCUS: i32 = 8;
    pub const BLUR: i32 = 9;
    pub const CLOSE_REQUEST: i32 = 10;
    pub const RESIZE: i32 = 11;
    pub const MOVE: i32 = 12;
    pub const ENTER: i32 = 13;
    pub const LEAVE: i32 = 14;
    pub const DPI_CHANGED: i32 = 15;
}

pub const MOD_SHIFT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_ALT: u32 = 4;
pub const MOD_SUPER: u32 = 8;

pub const BTN_LEFT: i32 = 1;
pub const BTN_RIGHT: i32 = 2;
pub const BTN_MIDDLE: i32 = 4;

pub const KEY_ESC: u32 = 27;
pub const KEY_TAB: u32 = 9;
pub const KEY_ENTER: u32 = 13;
pub const KEY_BACKSPACE: u32 = 8;
pub const KEY_DELETE: u32 = 127;
pub const KEY_UP: u32 = 0x100;
pub const KEY_DOWN: u32 = 0x101;
pub const KEY_LEFT: u32 = 0x102;
pub const KEY_RIGHT: u32 = 0x103;
pub const KEY_HOME: u32 = 0x104;
pub const KEY_END: u32 = 0x105;
pub const KEY_PGUP: u32 = 0x106;
pub const KEY_PGDN: u32 = 0x107;
pub const KEY_F1: u32 = 0x120;
pub const KEY_F4: u32 = 0x123;
pub const KEY_F12: u32 = 0x12b;
pub const KEY_SUPER: u32 = 0x140;
pub const KEY_SPACE: u32 = 32;

/// C `shell_event`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ShellEvent {
    pub ev_type: i32,
    pub x: i32,
    pub y: i32,
    pub button: i32,
    pub key: u32,
    pub mods: u32,
}

const fn event(ev_type: i32, x: i32, y: i32, button: i32, key: u32, mods: u32) -> ShellEvent {
    ShellEvent { ev_type, x, y, button, key, mods }
}

/// C `shell_window`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Window {
    pub id: i32,
    pub rect: Rect,
    pub restore: Rect,
    pub title: [u8; SHELL_TITLE_MAX],
    pub flags: u32,
    pub owner: u32,
    pub desktop: i32,
    pub min_w: i32,
    pub min_h: i32,
    pub client: *mut u32,
    pub client_w: i32,
    pub client_h: i32,
    pub client_stride: i32,
    pub q: [ShellEvent; SHELL_EVQ],
    pub q_head: i32,
    pub q_count: i32,
    pub dpi: u32,
}

impl Window {
    pub const EMPTY: Self = Self {
        id: 0,
        rect: Rect::new(0, 0, 0, 0),
        restore: Rect::new(0, 0, 0, 0),
        title: [0; SHELL_TITLE_MAX],
        flags: 0,
        owner: 0,
        desktop: 0,
        min_w: 0,
        min_h: 0,
        client: core::ptr::null_mut(),
        client_w: 0,
        client_h: 0,
        client_stride: 0,
        q: [ShellEvent { ev_type: 0, x: 0, y: 0, button: 0, key: 0, mods: 0 }; SHELL_EVQ],
        q_head: 0,
        q_count: 0,
        dpi: 0,
    };
}

/// C `shell_metrics`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Metrics {
    pub title_h: i32,
    pub border: i32,
    pub button_w: i32,
    pub edge_grab: i32,
    pub corner_r: i32,
}

/// C `shell_style`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Style {
    pub radius: i32,
    pub title_h: i32,
    pub border: i32,
    pub shadow_pct: i32,
    pub glow_pct: i32,
    pub opacity_pct: i32,
    pub icon_shape: i32,
    pub wallpaper: i32,
}

pub type ChangeFn = unsafe extern "C" fn(*mut Compositor, i32, *mut c_void);
pub type OverlayFn = unsafe extern "C" fn(*mut Compositor, *mut Fb, Rect, *mut c_void);

/// C `shell_comp.overlay[]` element.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Overlay {
    pub func: Option<OverlayFn>,
    pub user: *mut c_void,
}

impl Overlay {
    pub const EMPTY: Self = Self { func: None, user: core::ptr::null_mut() };
}

/// C `shell_comp`.
#[repr(C)]
#[allow(missing_docs)]
pub struct Compositor {
    pub screen_w: i32,
    pub screen_h: i32,
    pub win: [Window; SHELL_MAX_WINDOWS],
    pub order: [i32; SHELL_MAX_WINDOWS],
    pub nwin: i32,
    pub next_id: i32,
    pub focus: i32,
    pub cur_desktop: i32,
    pub theme: Theme,
    pub m: Metrics,
    pub style: Style,
    pub work_area: Rect,
    pub bottom_reserve: i32,
    pub top_reserve: i32,
    pub taskbar: *mut c_void,
    pub wall_cache: *mut u32,
    pub wall_key: u32,
    pub damage: Damage,
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub buttons: u32,
    pub hover: i32,
    pub grab_win: i32,
    pub grab_zone: Zone,
    pub grab_dx: i32,
    pub grab_dy: i32,
    pub grab_rect: Rect,
    pub capture: i32,
    pub on_change: Option<ChangeFn>,
    pub on_change_user: *mut c_void,
    pub overlay: [Overlay; 4],
    pub scale_pct: u32,
    pub reduce_motion: bool,
    pub high_contrast: bool,
    pub kbd_mods: u32,
    pub press_win: i32,
    pub press_zone: Zone,
    pub no_cull: bool,
    pub stat_culled: u32,
    pub last_click_win: i32,
    pub last_click_ms: u32,
    pub now_ms: u32,
}

fn contains(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

fn metrics_for(m: &mut Metrics, st: &Style, pct: u32) {
    let th = if st.title_h > 0 { st.title_h } else { 28 };
    let bd = if st.border > 0 { st.border } else { 1 };
    let rad = if st.radius >= 0 { st.radius } else { 8 };
    m.title_h = (th as u32 * pct / 100) as i32;
    m.border = if st.border > 0 {
        (bd as u32 * if pct >= 150 { 2 } else { 1 }) as i32
    } else if pct >= 150 {
        2
    } else {
        1
    };
    m.button_w = (46 * pct / 100) as i32;
    m.edge_grab = (6 * pct / 100) as i32;
    m.corner_r = (rad as u32 * pct / 100) as i32;
}

impl Compositor {
    fn scale(&self) -> i32 {
        if self.scale_pct > 0 { self.scale_pct as i32 } else { 100 }
    }

    pub fn init(&mut self, w: i32, h: i32) {
        self.screen_w = w;
        self.screen_h = h;
        self.win.fill(Window::EMPTY);
        self.order.fill(0);
        self.nwin = 0;
        self.next_id = 1;
        self.focus = 0;
        self.cur_desktop = 0;
        self.theme = Theme::dark(0xFF3B82F6);
        self.style = Style { radius: -1, title_h: 0, border: 0, shadow_pct: 100, glow_pct: 100, opacity_pct: 92, icon_shape: 0, wallpaper: 0 };
        metrics_for(&mut self.m, &self.style, 100);
        self.work_area = Rect::new(0, 0, w, h);
        self.bottom_reserve = 0;
        self.top_reserve = 0;
        self.taskbar = core::ptr::null_mut();
        self.wall_cache = core::ptr::null_mut();
        self.wall_key = 0;
        self.damage = Damage { r: [Rect::new(0, 0, 0, 0); SHELL_DAMAGE_MAX], n: 0 };
        self.mouse_x = w / 2;
        self.mouse_y = h / 2;
        self.buttons = 0;
        self.hover = 0;
        self.grab_win = 0;
        self.grab_zone = zone::NONE;
        self.grab_dx = 0;
        self.grab_dy = 0;
        self.grab_rect = Rect::new(0, 0, 0, 0);
        self.capture = 0;
        self.on_change = None;
        self.on_change_user = core::ptr::null_mut();
        self.overlay = [Overlay::EMPTY; 4];
        self.scale_pct = 100;
        self.reduce_motion = false;
        self.high_contrast = false;
        self.kbd_mods = 0;
        self.press_win = 0;
        self.press_zone = zone::NONE;
        self.no_cull = false;
        self.stat_culled = 0;
        self.last_click_win = 0;
        self.last_click_ms = 0;
        self.now_ms = 0;
        self.damage_all();
    }

    pub fn add_overlay(&mut self, func: OverlayFn, user: *mut c_void) -> bool {
        for slot in self.overlay.iter_mut() {
            if slot.func.is_none() {
                slot.func = Some(func);
                slot.user = user;
                return true;
            }
        }
        false
    }

    pub fn set_scale(&mut self, pct: u32) {
        let pct = pct.clamp(100, 400);
        self.scale_pct = pct;
        metrics_for(&mut self.m, &self.style, pct);
        if self.bottom_reserve != 0 || self.top_reserve != 0 {
            let top = self.top_reserve * pct as i32 / 100;
            self.work_area = Rect::new(0, top, self.screen_w, self.screen_h - top - self.bottom_reserve * pct as i32 / 100);
        }
        for i in 0..SHELL_MAX_WINDOWS {
            if self.win[i].id == 0 {
                continue;
            }
            self.win[i].dpi = 96 * pct / 100;
            let id = self.win[i].id;
            let dpi = self.win[i].dpi;
            self.post_event(id, event(ev::DPI_CHANGED, dpi as i32, 0, 0, 0, 0));
        }
        self.damage_all();
    }

    pub fn set_style(&mut self, st: &Style) {
        let mut n = *st;
        n.radius = n.radius.clamp(-1, 24);
        n.title_h = if n.title_h <= 0 { 0 } else { n.title_h.clamp(20, 48) };
        n.border = n.border.clamp(0, 4);
        n.shadow_pct = n.shadow_pct.clamp(0, 150);
        n.glow_pct = n.glow_pct.clamp(0, 150);
        n.opacity_pct = if n.opacity_pct <= 0 { 92 } else { n.opacity_pct.clamp(40, 100) };
        n.icon_shape = if !(0..=2).contains(&n.icon_shape) { 0 } else { n.icon_shape };
        n.wallpaper = if !(0..=3).contains(&n.wallpaper) { 0 } else { n.wallpaper };
        self.style = n;
        metrics_for(&mut self.m, &self.style, self.scale_pct);
        self.damage_all();
    }

    pub fn lw(&self) -> i32 {
        self.screen_w * 100 / self.scale()
    }
    pub fn lh(&self) -> i32 {
        self.screen_h * 100 / self.scale()
    }
    pub fn logical(&self, px: i32) -> i32 {
        px * 100 / self.scale()
    }
    pub fn clip_logical(&self, r: Rect) -> Rect {
        let sc = self.scale();
        let x0 = r.x * 100 / sc;
        let y0 = r.y * 100 / sc;
        let x1 = ((r.x + r.w) * 100 + sc - 1) / sc;
        let y1 = ((r.y + r.h) * 100 + sc - 1) / sc;
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
    pub fn damage_logical(&mut self, r: Rect) {
        let sc = self.scale();
        self.damage.add(Rect::new(
            r.x * sc / 100 - 1,
            r.y * sc / 100 - 1,
            (r.w * sc + 99) / 100 + 3,
            (r.h * sc + 99) / 100 + 3,
        ));
    }
    pub fn damage_all(&mut self) {
        self.damage.add(Rect::new(0, 0, self.screen_w, self.screen_h));
    }

    pub fn set_work_area(&mut self, wa: Rect) {
        self.work_area = wa;
        for i in 0..SHELL_MAX_WINDOWS {
            let w = &mut self.win[i];
            if w.id != 0 && (w.flags & WF_MAXIMIZED) != 0 {
                w.rect = wa;
            }
        }
        self.damage_all();
    }

    pub fn get(&self, id: i32) -> Option<&Window> {
        if id <= 0 {
            return None;
        }
        self.win.iter().find(|w| w.id == id)
    }
    pub fn get_mut(&mut self, id: i32) -> Option<&mut Window> {
        if id <= 0 {
            return None;
        }
        self.win.iter_mut().find(|w| w.id == id)
    }
    pub fn notify(&mut self, win: i32) {
        if let Some(f) = self.on_change {
            // SAFETY: the host installed this callback and keeps it valid.
            unsafe { f(self, win, self.on_change_user) };
        }
    }

    pub fn push_event(w: &mut Window, e: ShellEvent) {
        if w.q_count as usize == SHELL_EVQ {
            if e.ev_type == ev::MOUSE_MOVE {
                return;
            }
            w.q_head = (w.q_head + 1) % SHELL_EVQ as i32;
            w.q_count -= 1;
        }
        w.q[((w.q_head + w.q_count) % SHELL_EVQ as i32) as usize] = e;
        w.q_count += 1;
    }

    pub fn pop_event(&mut self, id: i32, out: &mut ShellEvent) -> bool {
        let Some(w) = self.get_mut(id) else { return false };
        if w.q_count == 0 {
            return false;
        }
        *out = w.q[w.q_head as usize];
        w.q_head = (w.q_head + 1) % SHELL_EVQ as i32;
        w.q_count -= 1;
        true
    }

    pub fn is_shown(&self, w: &Window) -> bool {
        w.id != 0
            && (w.flags & WF_VISIBLE) != 0
            && (w.flags & WF_MINIMIZED) == 0
            && ((w.flags & WF_STICKY) != 0 || w.desktop == self.cur_desktop)
    }

    fn outer_with_shadow(&self, r: Rect) -> Rect {
        let m = (22 * 3 / 2 + 7 + 2) * self.scale() / 100;
        Rect::new(r.x - m, r.y - m, r.w + 2 * m, r.h + 2 * m + 7 * self.scale() / 100)
    }

    fn damage_window(&mut self, w: &Window) {
        self.damage.add(self.outer_with_shadow(w.rect));
    }

    pub fn client_rect(&self, w: &Window) -> Rect {
        if (w.flags & WF_NOCHROME) != 0 {
            return w.rect;
        }
        let b = self.m.border;
        let mut r = Rect::new(w.rect.x + b, w.rect.y + b + self.m.title_h, w.rect.w - 2 * b, w.rect.h - 2 * b - self.m.title_h);
        if r.w < 0 {
            r.w = 0;
        }
        if r.h < 0 {
            r.h = 0;
        }
        r
    }

    fn order_remove(&mut self, id: i32) {
        for i in 0..self.nwin as usize {
            if self.order[i] == id {
                let n = self.nwin as usize - i - 1;
                self.order.copy_within(i + 1..i + 1 + n, i);
                self.nwin -= 1;
                return;
            }
        }
    }

    fn order_insert_top(&mut self, id: i32) {
        let topmost = self.get(id).map_or(false, |w| (w.flags & WF_TOPMOST) != 0);
        let mut pos = self.nwin as usize;
        if !topmost {
            while pos > 0 {
                let above_top = self.get(self.order[pos - 1]).map_or(false, |w| (w.flags & WF_TOPMOST) != 0);
                if !above_top {
                    break;
                }
                pos -= 1;
            }
        }
        let n = self.nwin as usize - pos;
        self.order.copy_within(pos..pos + n, pos + 1);
        self.order[pos] = id;
        self.nwin += 1;
    }

    pub fn create(&mut self, title: &[u8], rect: Rect, flags: u32, owner: u32) -> i32 {
        let Some(slot) = self.win.iter().position(|w| w.id == 0) else { return 0 };
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let mut w = Window::EMPTY;
        w.id = id;
        w.flags = flags | WF_VISIBLE;
        w.owner = owner;
        w.desktop = self.cur_desktop;
        w.dpi = 96 * self.scale_pct / 100;
        w.min_w = 160;
        w.min_h = self.m.title_h + 60;
        let n = title.len().min(SHELL_TITLE_MAX - 1);
        w.title[..n].copy_from_slice(&title[..n]);
        w.rect = rect;
        if w.rect.w < w.min_w {
            w.rect.w = w.min_w;
        }
        if w.rect.h < w.min_h {
            w.rect.h = w.min_h;
        }
        self.win[slot] = w;
        self.order_insert_top(id);
        let w = self.win[slot];
        self.damage_window(&w);
        if (w.flags & WF_NOFOCUS) == 0 {
            self.focus(id);
        }
        self.notify(id);
        id
    }

    fn focus_next_shown(&mut self) {
        for i in (0..self.nwin as usize).rev() {
            let id = self.order[i];
            if let Some(w) = self.get(id) {
                if self.is_shown(w) && (w.flags & WF_NOFOCUS) == 0 {
                    self.focus(id);
                    return;
                }
            }
        }
        self.focus(0);
    }

    pub fn destroy(&mut self, id: i32) {
        let Some(w) = self.get(id) else { return };
        let wr = *w;
        self.damage_window(&wr);
        self.order_remove(id);
        if self.grab_win == id {
            self.grab_win = 0;
        }
        if self.capture == id {
            self.capture = 0;
        }
        if self.hover == id {
            self.hover = 0;
        }
        let was_focus = self.focus == id;
        if let Some(w) = self.get_mut(id) {
            *w = Window::EMPTY;
        }
        if was_focus {
            self.focus = 0;
            self.focus_next_shown();
        }
        self.notify(id);
    }

    pub fn set_title(&mut self, id: i32, t: &[u8]) {
        let title_h = self.m.title_h;
        let border = self.m.border;
        let Some(w0) = self.get(id).copied() else { return };
        if let Some(w) = self.get_mut(id) {
            w.title.fill(0);
            let n = t.len().min(SHELL_TITLE_MAX - 1);
            w.title[..n].copy_from_slice(&t[..n]);
        }
        self.damage.add(Rect::new(w0.rect.x, w0.rect.y, w0.rect.w, title_h + border));
        self.notify(id);
    }

    pub fn set_rect(&mut self, id: i32, mut r: Rect) {
        let Some(old) = self.get(id).copied() else { return };
        if r.w < old.min_w {
            r.w = old.min_w;
        }
        if r.h < old.min_h {
            r.h = old.min_h;
        }
        let resized = r.w != old.rect.w || r.h != old.rect.h;
        let moved = r.x != old.rect.x || r.y != old.rect.y;
        self.damage_window(&old);
        if let Some(w) = self.get_mut(id) {
            w.rect = r;
        }
        let new = self.get(id).copied().unwrap_or(old);
        self.damage_window(&new);
        let cr = self.client_rect(&new);
        if resized {
            if let Some(w) = self.get_mut(id) {
                Self::push_event(w, event(ev::RESIZE, cr.w, cr.h, 0, 0, 0));
            }
        }
        if moved {
            if let Some(w) = self.get_mut(id) {
                Self::push_event(w, event(ev::MOVE, r.x, r.y, 0, 0, 0));
            }
        }
    }

    pub fn set_client(&mut self, id: i32, px: *mut u32, cw: i32, ch: i32, stride: i32) {
        if let Some(w) = self.get_mut(id) {
            w.client = px;
            w.client_w = cw;
            w.client_h = ch;
            w.client_stride = stride;
        }
        if let Some(w) = self.get(id).copied() {
            let cr = self.client_rect(&w);
            self.damage.add(cr);
        }
    }

    pub fn invalidate(&mut self, id: i32, rr: Rect) {
        let Some(w) = self.get(id) else { return };
        if !self.is_shown(w) {
            return;
        }
        let cr = self.client_rect(w);
        let abs = Rect::new(cr.x + rr.x, cr.y + rr.y, rr.w, rr.h);
        self.damage.add(abs.intersect(cr));
    }

    pub fn raise(&mut self, id: i32) {
        if self.get(id).is_none() || (self.nwin > 0 && self.order[self.nwin as usize - 1] == id) {
            return;
        }
        let Some(w) = self.get(id) else { return };
        let wr = *w;
        self.order_remove(id);
        self.order_insert_top(id);
        self.damage_window(&wr);
    }

    pub fn lower(&mut self, id: i32) {
        let Some(w) = self.get(id) else { return };
        let wr = *w;
        self.order_remove(id);
        let n = self.nwin as usize;
        self.order.copy_within(0..n, 1);
        self.order[0] = id;
        self.nwin += 1;
        self.damage_window(&wr);
        if self.focus == id {
            self.focus_next_shown();
        }
    }

    pub fn focus(&mut self, id: i32) {
        let nw = if id != 0 { self.get(id).copied() } else { None };
        if id != 0 && (nw.is_none() || (nw.unwrap().flags & WF_NOFOCUS) != 0) {
            return;
        }
        if self.focus == id {
            return;
        }
        let old = if self.focus != 0 { self.get(self.focus).copied() } else { None };
        if let Some(o) = old {
            let id_old = o.id;
            if let Some(w) = self.get_mut(id_old) {
                Self::push_event(w, event(ev::BLUR, 0, 0, 0, 0, 0));
            }
            self.damage_window(&o);
        }
        self.focus = id;
        if let Some(n) = nw {
            let nid = n.id;
            if let Some(w) = self.get_mut(nid) {
                Self::push_event(w, event(ev::FOCUS, 0, 0, 0, 0, 0));
            }
            self.damage_window(&n);
        }
        self.notify(id);
    }

    pub fn minimize(&mut self, id: i32) {
        let Some(w) = self.get(id).copied() else { return };
        if (w.flags & WF_MINIMIZED) != 0 {
            return;
        }
        self.damage_window(&w);
        if let Some(w) = self.get_mut(id) {
            w.flags |= WF_MINIMIZED;
        }
        if self.focus == id {
            self.focus = 0;
            if let Some(w) = self.get_mut(id) {
                Self::push_event(w, event(ev::BLUR, 0, 0, 0, 0, 0));
            }
            self.focus_next_shown();
        }
        self.notify(id);
    }

    pub fn restore(&mut self, id: i32) {
        let Some(w) = self.get(id).copied() else { return };
        if (w.flags & WF_MINIMIZED) != 0 {
            if let Some(w) = self.get_mut(id) {
                w.flags &= !WF_MINIMIZED;
            }
            self.raise(id);
            if let Some(w) = self.get(id).copied() {
                self.damage_window(&w);
            }
        }
        self.focus(id);
        self.notify(id);
    }

    pub fn toggle_maximize(&mut self, id: i32) {
        let Some(w) = self.get(id).copied() else { return };
        if (w.flags & WF_NORESIZE) != 0 {
            return;
        }
        if (w.flags & (WF_MAXIMIZED | WF_SNAPPED)) != 0 {
            if let Some(w) = self.get_mut(id) {
                w.flags &= !(WF_MAXIMIZED | WF_SNAPPED);
            }
            let restore = self.get(id).unwrap().restore;
            self.set_rect(id, restore);
        } else {
            let wa = self.work_area;
            if let Some(w) = self.get_mut(id) {
                w.restore = w.rect;
                w.flags |= WF_MAXIMIZED;
            }
            self.set_rect(id, wa);
        }
        self.notify(id);
    }

    fn zone_at(&self, w: &Window, x: i32, y: i32) -> Zone {
        if !contains(w.rect, x, y) {
            return zone::NONE;
        }
        if (w.flags & WF_NOCHROME) != 0 {
            return zone::CLIENT;
        }
        let m = &self.m;
        let can_size = (w.flags & (WF_NORESIZE | WF_MAXIMIZED)) == 0;
        let lx = x - w.rect.x;
        let ly = y - w.rect.y;
        let rx = w.rect.w - 1 - lx;
        let by = w.rect.h - 1 - ly;
        if can_size {
            let g = m.edge_grab;
            let cg = g * 2;
            let l = lx < g;
            let r = rx < g;
            let t = ly < g;
            let b = by < g;
            if (l && ly < cg) || (t && lx < cg) {
                return zone::CORNER_TL;
            }
            if (r && ly < cg) || (t && rx < cg) {
                return zone::CORNER_TR;
            }
            if (l && by < cg) || (b && lx < cg) {
                return zone::CORNER_BL;
            }
            if (r && by < cg) || (b && rx < cg) {
                return zone::CORNER_BR;
            }
            if l {
                return zone::EDGE_L;
            }
            if r {
                return zone::EDGE_R;
            }
            if t {
                return zone::EDGE_T;
            }
            if b {
                return zone::EDGE_B;
            }
        }
        if ly < m.border + m.title_h {
            let from_right = w.rect.w - m.border - lx;
            if (0..m.button_w).contains(&from_right) {
                return zone::CLOSE;
            }
            if from_right >= m.button_w && from_right < 2 * m.button_w && (w.flags & WF_NORESIZE) == 0 {
                return zone::MAXIMIZE;
            }
            if from_right >= 2 * m.button_w && from_right < 3 * m.button_w {
                return zone::MINIMIZE;
            }
            return zone::TITLE;
        }
        zone::CLIENT
    }

    pub fn hit_test(&self, x: i32, y: i32) -> (Zone, i32) {
        for i in (0..self.nwin as usize).rev() {
            let Some(w) = self.get(self.order[i]) else { continue };
            if !self.is_shown(w) {
                continue;
            }
            let z = self.zone_at(w, x, y);
            if z != zone::NONE {
                return (z, w.id);
            }
        }
        (zone::NONE, 0)
    }

    fn cursor_damage(&mut self) {
        let sc = self.scale();
        self.damage.add(Rect::new(self.mouse_x - 2, self.mouse_y - 2, 14 * sc / 100 + 4, 22 * sc / 100 + 4));
    }

    fn resize_rect(z: Zone, orig: Rect, dx: i32, dy: i32, w: &Window) -> Rect {
        let mut r = orig;
        let l = z == zone::EDGE_L || z == zone::CORNER_TL || z == zone::CORNER_BL;
        let rr = z == zone::EDGE_R || z == zone::CORNER_TR || z == zone::CORNER_BR;
        let t = z == zone::EDGE_T || z == zone::CORNER_TL || z == zone::CORNER_TR;
        let b = z == zone::EDGE_B || z == zone::CORNER_BL || z == zone::CORNER_BR;
        if l {
            r.x = orig.x + dx;
            r.w = orig.w - dx;
            if r.w < w.min_w {
                r.x = orig.x + orig.w - w.min_w;
                r.w = w.min_w;
            }
        }
        if rr {
            r.w = orig.w + dx;
            if r.w < w.min_w {
                r.w = w.min_w;
            }
        }
        if t {
            r.y = orig.y + dy;
            r.h = orig.h - dy;
            if r.h < w.min_h {
                r.y = orig.y + orig.h - w.min_h;
                r.h = w.min_h;
            }
        }
        if b {
            r.h = orig.h + dy;
            if r.h < w.min_h {
                r.h = w.min_h;
            }
        }
        r
    }

    pub fn mouse_move(&mut self, mut x: i32, mut y: i32) {
        self.cursor_damage();
        if x < 0 {
            x = 0;
        }
        if y < 0 {
            y = 0;
        }
        if x >= self.screen_w {
            x = self.screen_w - 1;
        }
        if y >= self.screen_h {
            y = self.screen_h - 1;
        }
        self.mouse_x = x;
        self.mouse_y = y;
        self.cursor_damage();

        if self.grab_win != 0 {
            let Some(w) = self.get(self.grab_win).copied() else {
                self.grab_win = 0;
                return;
            };
            let dx = x - self.grab_dx;
            let dy = y - self.grab_dy;
            let mut r = self.grab_rect;
            if self.grab_zone == zone::TITLE {
                if (w.flags & (WF_MAXIMIZED | WF_SNAPPED)) != 0 {
                    let rs = w.restore;
                    if let Some(w) = self.get_mut(w.id) {
                        w.flags &= !(WF_MAXIMIZED | WF_SNAPPED);
                    }
                    let rel = if self.grab_rect.w != 0 { (self.grab_dx - self.grab_rect.x) * 1000 / self.grab_rect.w } else { 500 };
                    r.w = rs.w;
                    r.h = rs.h;
                    r.x = x - rs.w * rel / 1000;
                    r.y = y - self.m.title_h / 2;
                    self.grab_rect = r;
                    self.grab_dx = x;
                    self.grab_dy = y;
                } else {
                    r.x += dx;
                    r.y += dy;
                }
                if r.y < self.work_area.y {
                    r.y = self.work_area.y;
                }
                if r.y > self.work_area.y + self.work_area.h - self.m.title_h {
                    r.y = self.work_area.y + self.work_area.h - self.m.title_h;
                }
                if r.x + r.w < 64 {
                    r.x = 64 - r.w;
                }
                if r.x > self.screen_w - 64 {
                    r.x = self.screen_w - 64;
                }
            } else {
                r = Self::resize_rect(self.grab_zone, self.grab_rect, dx, dy, &w);
            }
            self.set_rect(w.id, r);
            return;
        }
        if self.capture != 0 {
            let mods = self.kbd_mods;
            if let Some(w) = self.get(self.capture).copied() {
                let cr = self.client_rect(&w);
                if let Some(wm) = self.get_mut(w.id) {
                    Self::push_event(wm, event(ev::MOUSE_MOVE, x - cr.x, y - cr.y, 0, 0, mods));
                }
            }
            return;
        }
        let (z, hw) = self.hit_test(x, y);
        if hw != self.hover {
            let o = if self.hover != 0 { self.get(self.hover).copied() } else { None };
            let n = if hw != 0 { self.get(hw).copied() } else { None };
            let title_h = self.m.title_h;
            let border = self.m.border;
            if let Some(o) = o {
                if let Some(w) = self.get_mut(o.id) {
                    Self::push_event(w, event(ev::LEAVE, 0, 0, 0, 0, 0));
                }
                self.damage.add(Rect::new(o.rect.x, o.rect.y, o.rect.w, title_h + border));
            }
            if let Some(n) = n {
                if let Some(w) = self.get_mut(n.id) {
                    Self::push_event(w, event(ev::ENTER, 0, 0, 0, 0, 0));
                }
            }
            self.hover = hw;
        }
        if let Some(h) = self.get(hw).copied() {
            let title_h = self.m.title_h;
            let border = self.m.border;
            self.damage.add(Rect::new(h.rect.x, h.rect.y, h.rect.w, title_h + border));
            if z == zone::CLIENT {
                let cr = self.client_rect(&h);
                let mods = self.kbd_mods;
                if let Some(w) = self.get_mut(h.id) {
                    Self::push_event(w, event(ev::MOUSE_MOVE, x - cr.x, y - cr.y, 0, 0, mods));
                }
            }
        }
    }

    pub fn mouse_button(&mut self, button: i32, down: bool) {
        if down {
            self.buttons |= button as u32;
        } else {
            self.buttons &= !(button as u32);
        }
        if !down {
            if self.grab_win != 0 {
                let gw = self.grab_win;
                self.grab_win = 0;
                self.notify(gw);
            }
            if self.capture != 0 {
                let mods = self.kbd_mods;
                let (mx, my) = (self.mouse_x, self.mouse_y);
                if let Some(w) = self.get(self.capture).copied() {
                    let cr = self.client_rect(&w);
                    if let Some(wm) = self.get_mut(w.id) {
                        Self::push_event(wm, event(ev::MOUSE_UP, mx - cr.x, my - cr.y, button, 0, mods));
                    }
                }
                self.capture = 0;
            }
            if self.press_win != 0 {
                let (z, hw) = self.hit_test(self.mouse_x, self.mouse_y);
                let pw = self.press_win;
                let pz = self.press_zone;
                self.press_win = 0;
                if hw == pw && z == pz {
                    if pz == zone::CLOSE {
                        if let Some(w) = self.get(pw) {
                            let id = w.id;
                            if let Some(w) = self.get_mut(id) {
                                Self::push_event(w, event(ev::CLOSE_REQUEST, 0, 0, 0, 0, 0));
                            }
                        }
                    } else if pz == zone::MAXIMIZE {
                        self.toggle_maximize(pw);
                    } else if pz == zone::MINIMIZE {
                        self.minimize(pw);
                    }
                }
            }
            return;
        }
        let (z, hw) = self.hit_test(self.mouse_x, self.mouse_y);
        let Some(w) = self.get(hw).copied() else {
            self.focus(0);
            return;
        };
        self.raise(hw);
        self.focus(hw);
        match z {
            zone::CLIENT => {
                self.capture = hw;
                let cr = self.client_rect(&w);
                let (mx, my, mods) = (self.mouse_x, self.mouse_y, self.kbd_mods);
                if let Some(wm) = self.get_mut(hw) {
                    Self::push_event(wm, event(ev::MOUSE_DOWN, mx - cr.x, my - cr.y, button, 0, mods));
                }
            }
            zone::TITLE => {
                if button != BTN_LEFT {
                    return;
                }
                if self.last_click_win == hw && self.now_ms.wrapping_sub(self.last_click_ms) <= 400 && (w.flags & WF_NORESIZE) == 0 {
                    self.last_click_win = 0;
                    self.toggle_maximize(hw);
                    return;
                }
                self.last_click_win = hw;
                self.last_click_ms = self.now_ms;
                self.grab_win = hw;
                self.grab_zone = zone::TITLE;
                self.grab_dx = self.mouse_x;
                self.grab_dy = self.mouse_y;
                self.grab_rect = w.rect;
            }
            zone::CLOSE | zone::MAXIMIZE | zone::MINIMIZE => {
                self.press_win = hw;
                self.press_zone = z;
            }
            _ => {
                if z != zone::NONE && button == BTN_LEFT {
                    self.grab_win = hw;
                    self.grab_zone = z;
                    self.grab_dx = self.mouse_x;
                    self.grab_dy = self.mouse_y;
                    self.grab_rect = w.rect;
                }
            }
        }
    }

    pub fn post_event(&mut self, id: i32, e: ShellEvent) {
        if let Some(w) = self.get_mut(id) {
            Self::push_event(w, e);
        }
    }

    pub fn request_close(&mut self, id: i32) {
        if let Some(w) = self.get_mut(id) {
            Self::push_event(w, event(ev::CLOSE_REQUEST, 0, 0, 0, 0, 0));
        }
    }

    pub fn mouse_wheel(&mut self, delta: i32) {
        let (z, hw) = self.hit_test(self.mouse_x, self.mouse_y);
        if let Some(w) = self.get(hw).copied() {
            if z == zone::CLIENT {
                let cr = self.client_rect(&w);
                let (mx, my, mods) = (self.mouse_x, self.mouse_y, self.kbd_mods);
                if let Some(wm) = self.get_mut(hw) {
                    Self::push_event(wm, event(ev::MOUSE_WHEEL, mx - cr.x, my - cr.y, delta, 0, mods));
                }
            }
        }
    }

    pub fn key(&mut self, key: u32, down: bool, mods: u32) {
        self.kbd_mods = mods;
        if let Some(w) = self.get_mut(self.focus) {
            Self::push_event(w, event(if down { ev::KEY_DOWN } else { ev::KEY_UP }, 0, 0, 0, key, mods));
        }
    }

    pub fn char_event(&mut self, cp: u32) {
        if let Some(w) = self.get_mut(self.focus) {
            Self::push_event(w, event(ev::CHAR, 0, 0, 0, cp, 0));
        }
    }

    pub fn tile_radius(&self, size: i32) -> i32 {
        if self.style.icon_shape == 1 {
            size / 2
        } else if self.style.icon_shape == 2 {
            if size >= 20 { 2 } else { 1 }
        } else {
            size / 4
        }
    }

    pub fn panel_color(&self) -> u32 {
        let op = if self.style.opacity_pct > 0 { self.style.opacity_pct } else { 92 };
        (self.theme.surface & 0x00FF_FFFF) | ((op * 255 / 100) as u32) << 24
    }

    pub fn set_wallpaper_cache(&mut self, px: *mut u32) {
        self.wall_cache = px;
        self.wall_key = 0;
    }

    pub fn render(&mut self, fb: *mut Fb) {
        let d = self.damage;
        self.damage.n = 0;
        for i in 0..d.n as usize {
            paint(self, fb, d.r[i]);
        }
    }

    pub fn render_full(&mut self, fb: *mut Fb) {
        self.damage.n = 0;
        let all = Rect::new(0, 0, self.screen_w, self.screen_h);
        paint(self, fb, all);
    }
}

// ---------------------------------------------------------------- drawing

const CURSOR_PTS: [i16; 14] = [0, 0, 0, -264, 61, -205, 102, -298, 142, -280, 101, -189, 179, -189];
const CURSOR_ON: [u8; 7] = [1; 7];
const CURSOR_ENDS: [u16; 1] = [7];
const CUR_MAX: usize = 80;

struct Cursor {
    fill: [u8; CUR_MAX * CUR_MAX],
    edge: [u8; CUR_MAX * CUR_MAX],
    tmp: [u8; CUR_MAX * CUR_MAX],
    scale: i32,
    w: i32,
    h: i32,
    ox: i32,
    oy: i32,
}

static mut CURSOR: Cursor = Cursor {
    fill: [0; CUR_MAX * CUR_MAX],
    edge: [0; CUR_MAX * CUR_MAX],
    tmp: [0; CUR_MAX * CUR_MAX],
    scale: 0,
    w: 0,
    h: 0,
    ox: 0,
    oy: 0,
};

fn cursor_state() -> &'static mut Cursor {
    // SAFETY: the shell is single-threaded and cursor preparation does not re-enter.
    unsafe { &mut *core::ptr::addr_of_mut!(CURSOR) }
}

fn cursor_prepare(scale: i32) {
    let st = cursor_state();
    if st.scale == scale {
        return;
    }
    let m = (scale * 2 + 99) / 100 + 1;
    let mut w = (14 * scale + 99) / 100 + 2 * m;
    let mut h = (22 * scale + 99) / 100 + 2 * m;
    if w > CUR_MAX as i32 {
        w = CUR_MAX as i32;
    }
    if h > CUR_MAX as i32 {
        h = CUR_MAX as i32;
    }
    st.w = w;
    st.h = h;
    st.ox = m;
    st.oy = m;
    let k = ((scale as i64) << 24) / (100 * 16);
    let (wu, hu) = (w as usize, h as usize);
    raster::fill(&CURSOR_PTS, &CURSOR_ON, &CURSOR_ENDS, k as i32, m * 256, m * 256, w, h, &mut st.fill[..wu * hu]);
    let t = if (scale + 50) / 100 < 1 { 1 } else { (scale + 50) / 100 };
    st.edge[..wu * hu].copy_from_slice(&st.fill[..wu * hu]);
    for _ in 0..t {
        st.tmp[..wu * hu].copy_from_slice(&st.edge[..wu * hu]);
        for y in 0..hu {
            for x in 0..wu {
                let mut best = st.tmp[y * wu + x];
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let xx = x as i32 + dx;
                        let yy = y as i32 + dy;
                        if xx < 0 || yy < 0 || xx >= w || yy >= h {
                            continue;
                        }
                        let v = st.tmp[yy as usize * wu + xx as usize];
                        if v > best {
                            best = v;
                        }
                    }
                }
                st.edge[y * wu + x] = best;
            }
        }
    }
    st.scale = scale;
}

/// SAFETY: `fb` must point to a live shell_fb with stride*h pixels.
unsafe fn fb_px<'a>(fb: *mut Fb) -> &'a mut [u32] {
    // SAFETY: caller contract for shell_fb.
    unsafe { core::slice::from_raw_parts_mut((*fb).px, ((*fb).stride * (*fb).h) as usize) }
}

fn draw_cursor(c: &Compositor, fb: *mut Fb, clip: Rect) {
    let scale = c.scale();
    cursor_prepare(scale);
    let st = cursor_state();
    let screen = Rect::new(0, 0, c.screen_w, c.screen_h);
    let boxr = Rect::new(c.mouse_x - st.ox, c.mouse_y - st.oy, st.w, st.h).intersect(screen).intersect(clip);
    if boxr.is_empty() || unsafe { (*fb).px.is_null() } {
        return;
    }
    let stride = unsafe { (*fb).stride };
    let px = unsafe { fb_px(fb) };
    for py in boxr.y..boxr.y + boxr.h {
        for px_x in boxr.x..boxr.x + boxr.w {
            let i = (py - (c.mouse_y - st.oy)) * st.w + (px_x - (c.mouse_x - st.ox));
            let idx = i as usize;
            let d = &mut px[(py as usize) * stride as usize + px_x as usize];
            if st.edge[idx] != 0 {
                *d = blend(*d, (st.edge[idx] as u32) << 24);
            }
            if st.fill[idx] != 0 {
                *d = blend(*d, ((st.fill[idx] as u32) << 24) | 0x00FF_FFFF);
            }
        }
    }
}

const BAYER4: [i32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

fn paint_wallpaper_live(c: &Compositor, fb: *mut Fb, clip: Rect) {
    let t = &c.theme;
    let v = clip.intersect(Rect::new(0, 0, c.screen_w, c.screen_h));
    if v.is_empty() {
        return;
    }
    if c.high_contrast || c.style.wallpaper == 3 {
        unsafe { ffi::fb_fill(fb, v, v, t.desktop) };
        return;
    }
    let d = t.desktop;
    let lum = ((d >> 16) & 255) + ((d >> 8) & 255) + (d & 255);
    let light = lum > 384;
    let top = if light { mix(d, 0xFFFF_FFFF, 150) } else { mix(d, t.accent, 30) };
    let bot = if light { mix(d, t.accent, 26) } else { mix(d, 0xFF00_0000, 70) };
    let glow_a = t.accent;
    let glow_b = if light { 0xFFFF_FFFF } else { 0xFF7C_3AED };
    let (w, h) = (c.screen_w, c.screen_h);
    let gx1 = w * 78 / 100;
    let gy1 = h * 18 / 100;
    let gr1 = w * 55 / 100;
    let gx2 = w * 14 / 100;
    let gy2 = h * 92 / 100;
    let gr2 = w * 45 / 100;
    let mut g1 = (if light { 70 } else { 100 }) * c.style.glow_pct / 100;
    let mut g2 = (if light { 40 } else { 60 }) * c.style.glow_pct / 100;
    if c.style.wallpaper == 1 {
        g1 = 0;
        g2 = 0;
    }
    if c.style.wallpaper == 2 {
        let stride = unsafe { (*fb).stride };
        let px = unsafe { fb_px(fb) };
        for y in v.y..v.y + v.h {
            for x in v.x..v.x + v.w {
                let mut dd = (x * 3 + y * 5) % (w + h);
                if dd < 0 {
                    dd += w + h;
                }
                let k = dd * 512 / (w + h);
                let k = if k > 256 { 512 - k } else { k };
                let band = mix(glow_a, glow_b, k);
                let base = mix(top, bot, if h > 1 { y * 256 / (h - 1) } else { 0 });
                let a = ((k * (256 - k)) >> 8) * c.style.glow_pct / 150;
                let a = if a > 160 { 160 } else { a };
                px[(y as usize) * stride as usize + x as usize] = mix(base, band, a) | 0xFF00_0000;
            }
        }
        return;
    }
    if g1 == 0 && g2 == 0 {
        for y in v.y..v.y + v.h {
            let col = mix(top, bot, if h > 1 { y * 256 / (h - 1) } else { 0 });
            unsafe { ffi::fb_fill(fb, v, Rect::new(v.x, y, v.w, 1), col) };
        }
        return;
    }
    let inv1 = (256i64 << 24) / (gr1 as i64 * gr1 as i64);
    let inv2 = (256i64 << 24) / (gr2 as i64 * gr2 as i64);
    let ar = ((glow_a >> 16) & 255) as i32;
    let ag = ((glow_a >> 8) & 255) as i32;
    let ab = (glow_a & 255) as i32;
    let br2 = ((glow_b >> 16) & 255) as i32;
    let bg2 = ((glow_b >> 8) & 255) as i32;
    let bb2 = (glow_b & 255) as i32;
    let stride = unsafe { (*fb).stride };
    let px = unsafe { fb_px(fb) };
    for y in v.y..v.y + v.h {
        let base = mix(top, bot, if h > 1 { y * 256 / (h - 1) } else { 0 });
        let br = ((base >> 16) & 255) as i32;
        let bg = ((base >> 8) & 255) as i32;
        let bb = (base & 255) as i32;
        let dy1 = (y - gy1) as i64;
        let dy2 = (y - gy2) as i64;
        let d1y = dy1 * dy1;
        let d2y = dy2 * dy2;
        let row = (y as usize) * stride as usize;
        for x in v.x..v.x + v.w {
            let dx1 = (x - gx1) as i64;
            let dx2 = (x - gx2) as i64;
            let q1 = ((dx1 * dx1 + d1y) * inv1) >> 24;
            let q2 = ((dx2 * dx2 + d2y) * inv2) >> 24;
            let a1 = if q1 >= 256 { 0 } else { ((256 - q1) * (256 - q1) * g1 as i64 >> 16) as i32 };
            let a2 = if q2 >= 256 { 0 } else { ((256 - q2) * (256 - q2) * g2 as i64 >> 16) as i32 };
            let mut r = br * 256 + (ar - br) * a1 + (br2 - br) * a2;
            let mut g = bg * 256 + (ag - bg) * a1 + (bg2 - bg) * a2;
            let mut b = bb * 256 + (ab - bb) * a1 + (bb2 - bb) * a2;
            let th = BAYER4[((y & 3) * 4 + (x & 3)) as usize] * 16 + 8;
            r = (r + th) >> 8;
            g = (g + th) >> 8;
            b = (b + th) >> 8;
            let clamp = |v: i32| if v < 0 { 0 } else if v > 255 { 255 } else { v } as u32;
            px[row + x as usize] = 0xFF00_0000 | clamp(r) << 16 | clamp(g) << 8 | clamp(b);
        }
    }
}

fn paint_wallpaper(c: &mut Compositor, fb: *mut Fb, clip: Rect) {
    if c.wall_cache.is_null() || unsafe { (*fb).w } != c.screen_w || unsafe { (*fb).h } != c.screen_h {
        paint_wallpaper_live(c, fb, clip);
        return;
    }
    let t = &c.theme;
    let mut key = 2_166_136_261u32;
    let parts = [t.desktop, t.accent, c.style.glow_pct as u32, c.screen_w as u32, c.screen_h as u32, c.high_contrast as u32, c.style.wallpaper as u32];
    for p in parts {
        key = (key ^ p).wrapping_mul(16_777_619);
    }
    if key == 0 {
        key = 1;
    }
    if key != c.wall_key {
        let mut cf = Fb { px: c.wall_cache, w: c.screen_w, h: c.screen_h, stride: c.screen_w, scale: unsafe { (*fb).scale } };
        paint_wallpaper_live(c, &mut cf as *mut Fb, Rect::new(0, 0, c.screen_w, c.screen_h));
        c.wall_key = key;
    }
    let v = clip.intersect(Rect::new(0, 0, c.screen_w, c.screen_h));
    if v.is_empty() {
        return;
    }
    let stride = unsafe { (*fb).stride } as usize;
    let dst = unsafe { fb_px(fb) };
    // SAFETY: the cache is screen_w*screen_h pixels owned by the host.
    let src = unsafe { core::slice::from_raw_parts(c.wall_cache, (c.screen_w * c.screen_h) as usize) };
    for y in v.y..v.y + v.h {
        let so = (y as usize) * c.screen_w as usize + v.x as usize;
        let dof = (y as usize) * stride + v.x as usize;
        dst[dof..dof + v.w as usize].copy_from_slice(&src[so..so + v.w as usize]);
    }
}

fn draw_button_glyph(fb: *mut Fb, clip: Rect, z: Zone, b: Rect, col: u32, scale: i32) {
    let cx = b.x + b.w / 2;
    let cy = b.y + b.h / 2;
    let mut u = (scale + 50) / 100;
    if u < 1 {
        u = 1;
    }
    match z {
        zone::MINIMIZE => unsafe { ffi::fb_fill(fb, clip, Rect::new(cx - 5 * u, cy + 2 * u, 10 * u, u), col) },
        zone::MAXIMIZE => {
            let r = Rect::new(cx - 5 * u, cy - 5 * u, 10 * u, 10 * u);
            unsafe {
                ffi::fb_frame(fb, clip, r, col);
                ffi::fb_fill(fb, clip, Rect::new(r.x, r.y + u, r.w, u), col);
            }
        }
        _ => {
            let f = crate::font::ShellFont { face: core::ptr::addr_of!(font_data::loos_font_regular), size: 20 * scale / 100 };
            let times = b"\xC3\x97\0";
            let tw = unsafe { ffi::shell_text_width_ex(times.as_ptr() as *const c_char, f) };
            unsafe { ffi::fb_text_ex(fb, clip, cx - tw / 2, cy - f.size * 55 / 100, times.as_ptr() as *const c_char, col, f) };
        }
    }
}

fn blit_client(fb: *mut Fb, clip: Rect, cr: Rect, w: &Window, outer: Rect, rad: i32) {
    let src = Rect::new(0, 0, w.client_w, w.client_h);
    unsafe {
        if rad > 0 {
            ffi::fb_blit_rounded(fb, clip, cr.x, cr.y, w.client, w.client_w, w.client_h, w.client_stride, src, outer, rad);
        } else {
            ffi::fb_blit(fb, clip, cr.x, cr.y, w.client, w.client_w, w.client_h, w.client_stride, src);
        }
    }
}

fn window_title(w: &Window) -> &[u8] {
    let n = w.title.iter().position(|&b| b == 0).unwrap_or(SHELL_TITLE_MAX);
    &w.title[..n]
}

fn draw_window(c: &Compositor, fb: *mut Fb, clip: Rect, w: &Window, hover_zone: Zone) {
    let t = &c.theme;
    let sc = c.scale();
    let focused = c.focus == w.id;
    if (w.flags & WF_NOCHROME) != 0 {
        let cr = w.rect;
        unsafe {
            if !w.client.is_null() {
                blit_client(fb, clip, cr, w, cr, 0);
            } else {
                ffi::fb_fill(fb, clip, cr, t.surface);
            }
        }
        return;
    }
    let max = (w.flags & WF_MAXIMIZED) != 0;
    let rad = if max { 0 } else { c.m.corner_r };
    if !max {
        let sp = c.style.shadow_pct;
        if sp > 0 {
            let a = (if focused { 0x70 } else { 0x48 }) * sp / 100;
            let a = if a > 255 { 255 } else { a } as u32;
            unsafe { ffi::fb_shadow(fb, clip, w.rect, rad, 22 * sp / 100 * sc / 100, 7 * sc / 100, a << 24) };
        }
    }
    let b = c.m.border;
    let tb = Rect::new(w.rect.x + b, w.rect.y + b, w.rect.w - 2 * b, c.m.title_h);
    let tcol = if focused { mix(t.surface, t.accent, 14) } else { mix(t.surface, t.title_inactive, 128) };
    unsafe {
        ffi::fb_fill_round(fb, clip, w.rect, rad, t.surface);
        let cr = c.client_rect(w);
        ffi::fb_fill_round(fb, clip, Rect::new(tb.x, tb.y, tb.w, tb.h + rad), if rad > b { rad - b } else { 0 }, tcol);
        ffi::fb_fill(fb, clip, cr, t.surface);
        ffi::fb_fill(fb, clip, Rect::new(tb.x, tb.y + tb.h - 1, tb.w, 1), mix(tcol, t.border, 120));
    }
    let avail = tb.w - 3 * c.m.button_w - 44 * sc / 100;
    let tf = crate::font::ShellFont { face: core::ptr::addr_of!(font_data::loos_font_semibold), size: 13 * sc / 100 };
    let mut buf = [0u8; SHELL_TITLE_MAX];
    let title = window_title(w);
    let font = font_data::semibold();
    if crate::font::text_width(&font, tf.size, title) > avail {
        let room = avail - crate::font::text_width(&font, tf.size, b"\xE2\x80\xA6");
        let mut p = 0usize;
        let mut acc = 0i32;
        loop {
            let mut q = p;
            let cp = crate::font::next_cp(title, &mut q);
            if cp == 0 {
                break;
            }
            acc += crate::font::advance_q8(&font, tf.size, cp) >> 8;
            if acc > room {
                break;
            }
            p = q;
        }
        let mut n = p;
        if n > SHELL_TITLE_MAX - 4 {
            n = SHELL_TITLE_MAX - 4;
        }
        buf[..n].copy_from_slice(&title[..n]);
        buf[n..n + 3].copy_from_slice(b"\xE2\x80\xA6");
        buf[n + 3] = 0;
    } else {
        let n = title.len().min(SHELL_TITLE_MAX - 1);
        buf[..n].copy_from_slice(&title[..n]);
    }
    let isz = 16 * sc / 100;
    let tr = Rect::new(tb.x + 12 * sc / 100, tb.y + (tb.h - isz) / 2, isz, isz);
    unsafe {
        crate::taskbar::draw_app_tile(fb, clip, tr, window_title(w), c.tile_radius(isz));
        ffi::fb_text_ex(
            fb,
            clip,
            tb.x + 12 * sc / 100 + isz + 8 * sc / 100,
            tb.y + (tb.h - tf.size) / 2 - sc / 100,
            buf.as_ptr() as *const c_char,
            if focused { t.text } else { t.text_dim },
            tf,
        );
    }
    let zs = [zone::MINIMIZE, zone::MAXIMIZE, zone::CLOSE];
    for &z in &zs {
        if z == zone::MAXIMIZE && (w.flags & WF_NORESIZE) != 0 {
            continue;
        }
        let slot = if z == zone::CLOSE { 0 } else if z == zone::MAXIMIZE { 1 } else { 2 };
        let br = Rect::new(w.rect.x + w.rect.w - b - (slot + 1) * c.m.button_w, tb.y, c.m.button_w, tb.h - 1);
        if hover_zone == z {
            unsafe {
                if z == zone::CLOSE && rad > 0 {
                    ffi::fb_fill_round(fb, clip, Rect::new(br.x, br.y, br.w, br.h + rad), if rad > b { rad - b } else { 0 }, 0xFFC4_2B1C);
                    ffi::fb_fill(fb, clip, Rect::new(br.x, br.y + br.h - 1, br.w, rad + 1), t.surface);
                } else {
                    ffi::fb_fill(fb, clip, br, if z == zone::CLOSE { 0xFFC4_2B1C } else { (t.text & 0x00FF_FFFF) | 0x1A00_0000 });
                }
            }
        }
        let col = if hover_zone == z && z == zone::CLOSE { 0xFFFF_FFFF } else { t.text };
        draw_button_glyph(fb, clip, z, br, col, sc);
    }
    if !w.client.is_null() {
        blit_client(fb, clip, c.client_rect(w), w, w.rect, rad);
    }
    unsafe { ffi::fb_stroke_round(fb, clip, w.rect, rad, b, if focused { t.accent } else { t.border }) };
}

fn window_hides(c: &Compositor, w: &Window, clip: Rect) -> bool {
    if (w.flags & WF_NOCHROME) != 0 {
        return false;
    }
    let rad = if (w.flags & WF_MAXIMIZED) != 0 { 0 } else { c.m.corner_r };
    let inside = Rect::new(w.rect.x + rad, w.rect.y + rad, w.rect.w - 2 * rad, w.rect.h - 2 * rad);
    !inside.is_empty()
        && !clip.is_empty()
        && clip.x >= inside.x
        && clip.y >= inside.y
        && clip.x + clip.w <= inside.x + inside.w
        && clip.y + clip.h <= inside.y + inside.h
}

fn paint(c: &mut Compositor, fb: *mut Fb, clip: Rect) {
    let mut start = 0;
    if !c.no_cull {
        for i in (0..c.nwin as usize).rev() {
            if let Some(w) = c.get(c.order[i]) {
                if c.is_shown(w) && window_hides(c, w, clip) {
                    start = i;
                    break;
                }
            }
        }
    }
    if start > 0 {
        c.stat_culled += 1;
    } else {
        paint_wallpaper(c, fb, clip);
    }
    let (hz, hz_win) = c.hit_test(c.mouse_x, c.mouse_y);
    for i in start..c.nwin as usize {
        let Some(w) = c.get(c.order[i]) else { continue };
        if !c.is_shown(w) {
            continue;
        }
        if !c.outer_with_shadow(w.rect).overlaps(clip) {
            continue;
        }
        let hover = if w.id == hz_win { hz } else { zone::NONE };
        draw_window(c, fb, clip, w, hover);
    }
    for i in 0..4 {
        if let Some(f) = c.overlay[i].func {
            // SAFETY: overlay callbacks are host-installed and valid for the compositor's lifetime.
            unsafe { f(c as *mut Compositor, fb, clip, c.overlay[i].user) };
        }
    }
    draw_cursor(c, fb, clip);
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn comp_init(c: *mut Compositor, w: i32, h: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.init(w, h);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_add_overlay(c: *mut Compositor, func: Option<OverlayFn>, user: *mut c_void) -> bool {
    match unsafe { c.as_mut() } {
        Some(c) => func.is_some() && c.add_overlay(func.unwrap(), user),
        None => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_set_scale(c: *mut Compositor, pct: u32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.set_scale(pct);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_set_style(c: *mut Compositor, st: *const Style) {
    if let (Some(c), Some(st)) = (unsafe { c.as_mut() }, unsafe { st.as_ref() }) {
        c.set_style(st);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_create(c: *mut Compositor, title: *const c_char, rect: Rect, flags: u32, owner: u32) -> i32 {
    match unsafe { c.as_mut() } {
        Some(c) => c.create(unsafe { ffi::c_bytes(title) }, rect, flags, owner),
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_destroy(c: *mut Compositor, id: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.destroy(id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_set_work_area(c: *mut Compositor, wa: Rect) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.set_work_area(wa);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_get(c: *mut Compositor, id: i32) -> *mut Window {
    match unsafe { c.as_mut() } {
        Some(c) => c.get_mut(id).map_or(core::ptr::null_mut(), |w| w as *mut Window),
        None => core::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_set_title(c: *mut Compositor, id: i32, title: *const c_char) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.set_title(id, unsafe { ffi::c_bytes(title) });
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_set_rect(c: *mut Compositor, id: i32, r: Rect) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.set_rect(id, r);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_set_client(c: *mut Compositor, id: i32, px: *mut u32, w: i32, h: i32, stride: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.set_client(id, px, w, h, stride);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_invalidate(c: *mut Compositor, id: i32, r: Rect) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.invalidate(id, r);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_raise(c: *mut Compositor, id: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.raise(id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_lower(c: *mut Compositor, id: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.lower(id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_focus(c: *mut Compositor, id: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.focus(id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_minimize(c: *mut Compositor, id: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.minimize(id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_restore(c: *mut Compositor, id: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.restore(id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_toggle_maximize(c: *mut Compositor, id: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.toggle_maximize(id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_client_rect(c: *const Compositor, w: *const Window) -> Rect {
    match (unsafe { c.as_ref() }, unsafe { w.as_ref() }) {
        (Some(c), Some(w)) => c.client_rect(w),
        _ => Rect::new(0, 0, 0, 0),
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_hit_test(c: *const Compositor, x: i32, y: i32, win_id: *mut i32) -> Zone {
    match unsafe { c.as_ref() } {
        Some(c) => {
            let (z, id) = c.hit_test(x, y);
            if !win_id.is_null() {
                unsafe { *win_id = id };
            }
            z
        }
        None => {
            if !win_id.is_null() {
                unsafe { *win_id = 0 };
            }
            zone::NONE
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_is_shown(c: *const Compositor, w: *const Window) -> bool {
    match (unsafe { c.as_ref() }, unsafe { w.as_ref() }) {
        (Some(c), Some(w)) => c.is_shown(w),
        _ => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_mouse_move(c: *mut Compositor, x: i32, y: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.mouse_move(x, y);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_mouse_button(c: *mut Compositor, button: i32, down: bool) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.mouse_button(button, down);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_mouse_wheel(c: *mut Compositor, delta: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.mouse_wheel(delta);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_key(c: *mut Compositor, key: u32, down: bool, mods: u32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.key(key, down, mods);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_char(c: *mut Compositor, cp: u32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.char_event(cp);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_pop_event(c: *mut Compositor, id: i32, out: *mut ShellEvent) -> bool {
    let Some(out) = (unsafe { out.as_mut() }) else { return false };
    match unsafe { c.as_mut() } {
        Some(c) => c.pop_event(id, out),
        None => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_render(c: *mut Compositor, fb: *mut Fb) {
    if let Some(c) = unsafe { c.as_mut() } {
        if !fb.is_null() {
            c.render(fb);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_render_full(c: *mut Compositor, fb: *mut Fb) {
    if let Some(c) = unsafe { c.as_mut() } {
        if !fb.is_null() {
            c.render_full(fb);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_tile_radius(c: *const Compositor, size: i32) -> i32 {
    unsafe { c.as_ref() }.map_or(0, |c| c.tile_radius(size))
}

#[no_mangle]
pub unsafe extern "C" fn comp_panel_color(c: *const Compositor) -> u32 {
    unsafe { c.as_ref() }.map_or(0, Compositor::panel_color)
}

#[no_mangle]
pub unsafe extern "C" fn comp_set_wallpaper_cache(c: *mut Compositor, px: *mut u32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.set_wallpaper_cache(px);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_damage_all(c: *mut Compositor) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.damage_all();
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_lw(c: *const Compositor) -> i32 {
    unsafe { c.as_ref() }.map_or(0, Compositor::lw)
}

#[no_mangle]
pub unsafe extern "C" fn comp_lh(c: *const Compositor) -> i32 {
    unsafe { c.as_ref() }.map_or(0, Compositor::lh)
}

#[no_mangle]
pub unsafe extern "C" fn comp_logical(c: *const Compositor, px: i32) -> i32 {
    unsafe { c.as_ref() }.map_or(0, |c| c.logical(px))
}

#[no_mangle]
pub unsafe extern "C" fn comp_clip_logical(c: *const Compositor, r: Rect) -> Rect {
    unsafe { c.as_ref() }.map_or(Rect::new(0, 0, 0, 0), |c| c.clip_logical(r))
}

#[no_mangle]
pub unsafe extern "C" fn comp_damage_logical(c: *mut Compositor, r: Rect) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.damage_logical(r);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_view(c: *const Compositor, fb: *const Fb) -> Fb {
    match (unsafe { c.as_ref() }, unsafe { fb.as_ref() }) {
        (Some(c), Some(fb)) => Fb { px: fb.px, w: fb.w, h: fb.h, stride: fb.stride, scale: c.scale() },
        _ => Fb { px: core::ptr::null_mut(), w: 0, h: 0, stride: 0, scale: 100 },
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_notify(c: *mut Compositor, win: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.notify(win);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_post_event(c: *mut Compositor, id: i32, e: ShellEvent) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.post_event(id, e);
    }
}

#[no_mangle]
pub unsafe extern "C" fn comp_request_close(c: *mut Compositor, id: i32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.request_close(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    fn comp() -> &'static mut Compositor {
        // Tests only: a leak per call is fine, and avoids a 220 KiB stack temporary.
        Box::leak(Box::new(unsafe { core::mem::zeroed::<Compositor>() }))
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ShellEvent>(), 24);
        assert_eq!(size_of::<Window>(), 1720);
        assert_eq!(size_of::<Metrics>(), 20);
        assert_eq!(size_of::<Style>(), 32);
        assert_eq!(size_of::<Compositor>(), 221272);
        assert_eq!(offset_of!(Window, title), 36);
        assert_eq!(offset_of!(Window, flags), 132);
        assert_eq!(offset_of!(Window, client), 152);
        assert_eq!(offset_of!(Window, q), 172);
        assert_eq!(offset_of!(Window, dpi), 1716);
        assert_eq!(offset_of!(Compositor, win), 8);
        assert_eq!(offset_of!(Compositor, order), 220168);
        assert_eq!(offset_of!(Compositor, nwin), 220680);
        assert_eq!(offset_of!(Compositor, theme), 220696);
        assert_eq!(offset_of!(Compositor, m), 220736);
        assert_eq!(offset_of!(Compositor, style), 220756);
        assert_eq!(offset_of!(Compositor, work_area), 220788);
        assert_eq!(offset_of!(Compositor, taskbar), 220816);
        assert_eq!(offset_of!(Compositor, wall_cache), 220824);
        assert_eq!(offset_of!(Compositor, damage), 220836);
        assert_eq!(offset_of!(Compositor, mouse_x), 221096);
        assert_eq!(offset_of!(Compositor, hover), 221108);
        assert_eq!(offset_of!(Compositor, grab_win), 221112);
        assert_eq!(offset_of!(Compositor, capture), 221144);
        assert_eq!(offset_of!(Compositor, on_change), 221152);
        assert_eq!(offset_of!(Compositor, overlay), 221168);
        assert_eq!(offset_of!(Compositor, scale_pct), 221232);
        assert_eq!(offset_of!(Compositor, reduce_motion), 221236);
        assert_eq!(offset_of!(Compositor, kbd_mods), 221240);
        assert_eq!(offset_of!(Compositor, no_cull), 221252);
        assert_eq!(offset_of!(Compositor, now_ms), 221268);
    }

    #[test]
    fn caption_buttons_are_46px_close_rightmost() {
        let c = comp();
        c.init(800, 600);
        let id = c.create(b"Test", Rect::new(100, 100, 400, 300), 0, 1);
        let w = c.get(id).unwrap();
        let right = w.rect.x + w.rect.w - 1;
        let title_y = w.rect.y + 12;
        assert_eq!(c.hit_test(right - 10, title_y), (zone::CLOSE, id));
        assert_eq!(c.hit_test(right - 46 - 10, title_y), (zone::MAXIMIZE, id));
        assert_eq!(c.hit_test(right - 92 - 10, title_y), (zone::MINIMIZE, id));
        assert_eq!(c.hit_test(right - 138 - 10, title_y), (zone::TITLE, id));
        // Close is rightmost: at 200% each caption slot doubles.
        c.set_scale(200);
        let w = c.get(id).unwrap().rect;
        let right = w.x + w.w - 1;
        assert_eq!(c.hit_test(right - 30, w.y + 30).0, zone::CLOSE);
        assert_eq!(c.hit_test(right - 92 - 10, w.y + 30).0, zone::MAXIMIZE);
    }

    #[test]
    fn move_damages_old_and_new_rects() {
        let c = comp();
        c.init(800, 600);
        let id = c.create(b"Move", Rect::new(100, 100, 200, 150), 0, 1);
        c.damage.n = 0;
        c.set_rect(id, Rect::new(300, 200, 200, 150));
        let b = c.damage.bounds();
        assert!(b.x <= 58 && b.y <= 58, "old shadow rect is damaged: {b:?}");
        assert!(b.x + b.w >= 542 && b.y + b.h >= 392, "new shadow rect is damaged: {b:?}");
        let mut e = ShellEvent::default();
        let mut moved = false;
        while c.pop_event(id, &mut e) {
            if e.ev_type == ev::MOVE {
                moved = true;
                assert_eq!((e.x, e.y), (300, 200));
            }
        }
        assert!(moved, "move event queued");
    }

    #[test]
    fn z_order_focus_and_minimize() {
        let c = comp();
        c.init(800, 600);
        let a = c.create(b"A", Rect::new(0, 0, 200, 200), 0, 1);
        let b = c.create(b"B", Rect::new(50, 50, 200, 200), 0, 1);
        assert_eq!(c.focus, b);
        assert_eq!(c.order[c.nwin as usize - 1], b);
        c.raise(a);
        assert_eq!(c.order[c.nwin as usize - 1], a);
        c.minimize(a);
        assert_eq!(c.focus, b, "focus falls back to the next shown window");
        c.restore(a);
        assert_eq!(c.focus, a);
    }
}
