//! Hardware input routing (mouse/keyboard → messages) and desktop composition.

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::gdi::{colorref_to_argb, fill};
use crate::wm::{BORDER, CAPTION_H};
use crate::*;
use loos_shell::draw::{Canvas, Rect as SRect};
use loos_shell::{font_data, text};

impl Win32k {
    /// Topmost visible, enabled window under a screen point (deepest child) with its NC hit-test code.
    pub fn window_from_point(&mut self, p: Point) -> Option<(Hwnd, isize)> {
        let tops: Vec<Hwnd> = self.tops.iter().rev().copied().collect();
        for t in tops {
            if !self.is_window_visible(t) {
                continue;
            }
            let Ok(r) = self.get_window_rect(t) else { continue };
            if !r.contains(p) {
                continue;
            }
            let ht = self.send_message(t, WM_NCHITTEST, 0, make_lparam(p.x, p.y));
            if ht == HTTRANSPARENT {
                continue;
            }
            if ht == HTCLIENT {
                return Some((self.child_at(t, p), HTCLIENT));
            }
            return Some((t, ht));
        }
        None
    }

    fn child_at(&mut self, parent: Hwnd, p: Point) -> Hwnd {
        let kids: Vec<Hwnd> = self.wins[&parent].children.iter().rev().copied().collect();
        for k in kids {
            if !self.is_window_visible(k) {
                continue;
            }
            if self.get_window_rect(k).is_ok_and(|r| r.contains(p)) {
                let ht = self.send_message(k, WM_NCHITTEST, 0, make_lparam(p.x, p.y));
                if ht == HTTRANSPARENT {
                    continue;
                }
                return if ht == HTCLIENT { self.child_at(k, p) } else { k };
            }
        }
        parent
    }

    fn mouse_flags(&self) -> usize {
        let mut f = self.buttons as usize;
        if self.keys[VK_SHIFT as usize] {
            f |= 4;
        }
        if self.keys[VK_CONTROL as usize] {
            f |= 8;
        }
        f
    }

    fn post_mouse(&mut self, h: Hwnd, msg: u32) {
        if !self.is_window(h) || !self.is_window_enabled(h) {
            return;
        }
        let c = self.screen_to_client(h, self.cursor).unwrap_or_default();
        let fl = self.mouse_flags();
        self.post_message(h, msg, fl, make_lparam(c.x, c.y)).ok();
    }

    /// Absolute mouse move (screen coords).
    pub fn inject_mouse_move(&mut self, x: i32, y: i32) {
        self.cursor = Point { x: x.clamp(0, self.screen.0 - 1), y: y.clamp(0, self.screen.1 - 1) };
        if let Some((h, dx, dy)) = self.drag {
            let (nx, ny) = (self.cursor.x - dx, self.cursor.y - dy);
            self.move_window(h, nx, ny, self.wins[&h].w, self.wins[&h].h).ok();
            return;
        }
        let target = if self.capture != 0 {
            self.capture
        } else {
            match self.window_from_point(self.cursor) {
                Some((h, HTCLIENT)) => h,
                _ => return,
            }
        };
        self.post_mouse(target, WM_MOUSEMOVE);
    }

    /// Mouse button transition at the current cursor. `button`: 0 left, 1 right, 2 middle.
    pub fn inject_mouse_button(&mut self, button: u32, down: bool) {
        let (flag, dn, up) = match button {
            0 => (MK_LBUTTON, WM_LBUTTONDOWN, WM_LBUTTONUP),
            1 => (MK_RBUTTON, WM_RBUTTONDOWN, WM_RBUTTONUP),
            _ => (MK_MBUTTON, WM_MBUTTONDOWN, WM_MBUTTONUP),
        };
        if down {
            self.buttons |= flag;
        } else {
            self.buttons &= !flag;
        }
        // End of caption drag / close-button release.
        if !down && button == 0 {
            if self.drag.take().is_some() {
                return;
            }
            let cd = core::mem::take(&mut self.close_down);
            if cd != 0 {
                if self.is_window(cd) && self.close_button(cd).contains(self.cursor) {
                    self.post_message(cd, WM_SYSCOMMAND, SC_CLOSE, 0).ok();
                }
                return;
            }
        }
        let hit = if self.capture != 0 { Some((self.capture, HTCLIENT)) } else { self.window_from_point(self.cursor) };
        if down && button == 0 && (self.open_menu.is_some() || matches!(hit, Some((_, HTMENU)))) {
            let hm = match hit {
                Some((h, HTMENU)) => Some(h),
                _ => None,
            };
            if let Some((h, _)) = hit {
                let root = self.root_of(h);
                if root != 0 && self.active != root {
                    self.set_active_window(root);
                }
            }
            if self.menu_click(self.cursor, hm) {
                return;
            }
        }
        let Some((h, ht)) = hit else { return };
        if down {
            let root = self.root_of(h);
            if root != 0 && self.is_window_enabled(root) && self.active != root {
                self.set_active_window(root);
            }
            if button == 0 {
                if ht == HTCAPTION && self.capture == 0 {
                    let r = self.get_window_rect(h).unwrap_or_default();
                    self.drag = Some((h, self.cursor.x - r.left, self.cursor.y - r.top));
                    return;
                }
                if ht == HTCLOSE {
                    self.close_down = h;
                    return;
                }
            }
            if ht == HTCLIENT && self.is_window_enabled(h) && self.wins[&h].style & WS_CHILD != 0 {
                self.set_focus(h);
            }
        }
        if ht == HTCLIENT || self.capture != 0 {
            self.post_mouse(h, if down { dn } else { up });
        }
    }

    /// Key transition; routed to the focus window (WM_SYSKEY* while Alt is held).
    pub fn inject_key(&mut self, vk: u32, down: bool) {
        if vk > 255 {
            return;
        }
        let repeat = down && self.keys[vk as usize];
        self.keys[vk as usize] = down;
        let target = if self.focus != 0 { self.focus } else { self.active };
        if target == 0 || !self.is_window(target) {
            return;
        }
        let alt = self.keys[VK_MENU as usize];
        let msg = match (down, alt) {
            (true, false) => WM_KEYDOWN,
            (false, false) => WM_KEYUP,
            (true, true) => WM_SYSKEYDOWN,
            (false, true) => WM_SYSKEYUP,
        };
        let mut lp: u32 = 1 | (vk & 0xFF) << 16; // repeat count + scancode≈vk
        if repeat {
            lp |= 1 << 30;
        }
        if !down {
            lp |= 1 << 30 | 1 << 31;
        }
        self.post_message(target, msg, vk as usize, lp as i32 as isize).ok();
    }

    /// Render the whole desktop into `fb` (`screen.0 * screen.1` pixels, 0xAARRGGBB).
    pub fn compose(&mut self, fb: &mut [u32]) {
        let (sw, sh) = self.screen;
        if fb.len() < (sw * sh) as usize {
            return;
        }
        fb[..(sw * sh) as usize].fill(self.desktop_color);
        let tops = self.tops.clone();
        for t in tops {
            if self.is_window_visible(t) {
                self.draw_window(fb, t, t == self.active);
            }
        }
        self.draw_menus(fb, &self.tops.clone());
    }

    fn draw_window(&mut self, fb: &mut [u32], h: Hwnd, active: bool) {
        let (sw, sh) = self.screen;
        let Some(w) = self.wins.get(&h) else { return };
        let r = self.get_window_rect(h).unwrap_or_default();
        let o = self.client_origin(h);
        let screen = Rect::new(0, 0, sw, sh);
        let top = w.parent == 0;
        let captioned = top && w.style & WS_CAPTION == WS_CAPTION;
        let title = w.title.clone();
        let sysmenu = w.style & WS_SYSMENU != 0;
        let (cw, ch) = (w.cw, w.ch);
        let clip = if top { screen } else { self.parent_clip(w.parent).intersect(&screen) };
        if captioned || w.style & WS_BORDER != 0 {
            let frame = if active { 0xFFD4D0C8 } else { 0xFFBFBBB4 };
            let bw = if captioned { BORDER } else { 1 };
            put(fb, sw, clip.intersect(&r), frame);
            if captioned {
                let cap = if active { 0xFF0A246A } else { 0xFF808080 };
                let cr = Rect::new(r.left + BORDER, r.top + BORDER, r.right - BORDER, r.top + BORDER + CAPTION_H);
                put(fb, sw, clip.intersect(&cr), cap);
                let mut c = Canvas { px: &mut fb[..(sw * sh) as usize], w: sw, h: sh, stride: sw, scale: 0 };
                let tclip = clip.intersect(&Rect::new(cr.left, cr.top, cr.right - 24, cr.bottom));
                text::draw(&mut c, SRect::new(tclip.left, tclip.top, tclip.width().max(0), tclip.height().max(0)), cr.left + 6, cr.top + 4, title.as_bytes(), 0xFFFFFFFF, &font_data::semibold(), 15);
                if sysmenu {
                    let b = self.close_button(h);
                    put(fb, sw, clip.intersect(&b), 0xFFD4D0C8);
                    for i in 0..8 {
                        for (x, y) in [(b.left + 5 + i, b.top + 5 + i), (b.left + 12 - i, b.top + 5 + i)] {
                            if clip.contains(Point { x, y }) {
                                fb[(y * sw + x) as usize] = 0xFF000000;
                            }
                        }
                    }
                }
            }
            let inner = Rect::new(r.left + bw, o.y.min(r.bottom), r.right - bw, r.bottom - bw);
            let _ = inner;
        }
        // Client bitmap.
        let cr = Rect::new(o.x, o.y, o.x + cw, o.y + ch).intersect(&clip);
        if let Some(w) = self.wins.get(&h) {
            for y in cr.top..cr.bottom {
                let src = ((y - o.y) * cw + (cr.left - o.x)) as usize;
                let dst = (y * sw + cr.left) as usize;
                let n = cr.width() as usize;
                fb[dst..dst + n].copy_from_slice(&w.bmp[src..src + n]);
            }
        }
        for k in self.children(h) {
            if self.is_window_visible(k) {
                self.draw_window(fb, k, false);
            }
        }
    }

    fn parent_clip(&self, p: Hwnd) -> Rect {
        let o = self.client_origin(p);
        let w = &self.wins[&p];
        let r = Rect::new(o.x, o.y, o.x + w.cw, o.y + w.ch);
        if w.parent == 0 { r } else { r.intersect(&self.parent_clip(w.parent)) }
    }

    /// COLORREF helper for callers building raw pixels.
    pub fn colorref(c: u32) -> u32 {
        colorref_to_argb(c)
    }
}

fn put(fb: &mut [u32], stride: i32, r: Rect, c: u32) {
    fill(fb, stride, r, c);
}
