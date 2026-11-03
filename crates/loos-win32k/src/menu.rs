//! Menu bars with one level of drop-down popups.

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::wm::{BORDER, CAPTION_H, MENU_H};
use crate::*;
use loos_shell::{font_data, text};

/// A menu item; non-empty `sub` makes it a popup.
#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    /// Command id sent in `WM_COMMAND`.
    pub id: u32,
    /// Label.
    pub text: String,
    /// Drop-down items.
    pub sub: Vec<MenuItem>,
}

impl MenuItem {
    /// Leaf command.
    pub fn item(id: u32, text: &str) -> Self {
        Self { id, text: text.into(), sub: Vec::new() }
    }
    /// Popup menu.
    pub fn popup(text: &str, sub: Vec<MenuItem>) -> Self {
        Self { id: 0, text: text.into(), sub }
    }
}

const ROW_H: i32 = 22;

fn tw(s: &str) -> i32 {
    text::width(&font_data::regular(), 14, s.as_bytes())
}

impl Win32k {
    /// `SetMenu`: installs a menu bar (empty removes it) and re-lays out the window.
    pub fn set_menu(&mut self, h: Hwnd, items: Vec<MenuItem>) -> Result<(), Win32kError> {
        self.win(h)?;
        self.wins.get_mut(&h).unwrap().menu = items;
        self.open_menu = None;
        self.relayout(h).map_err(|e| {
            self.last_error = e.code();
            e
        })
    }

    /// Menu-bar rectangle (screen coords).
    pub fn menu_bar_rect(&self, h: Hwnd) -> Rect {
        let r = self.get_window_rect(h).unwrap_or_default();
        let top = r.top + BORDER + CAPTION_H;
        Rect::new(r.left + BORDER, top, r.right - BORDER, top + MENU_H)
    }

    /// Screen rectangles of bar items.
    pub fn menu_bar_items(&self, h: Hwnd) -> Vec<Rect> {
        let Some(w) = self.wins.get(&h) else { return Vec::new() };
        let bar = self.menu_bar_rect(h);
        let mut x = bar.left + 4;
        w.menu
            .iter()
            .map(|m| {
                let r = Rect::new(x, bar.top, x + tw(&m.text) + 16, bar.bottom);
                x = r.right;
                r
            })
            .collect()
    }

    /// Screen rectangles of the open drop-down rows.
    pub fn dropdown_rows(&self) -> Vec<Rect> {
        let Some((h, i)) = self.open_menu else { return Vec::new() };
        let Some(w) = self.wins.get(&h) else { return Vec::new() };
        let Some(item) = w.menu.get(i) else { return Vec::new() };
        let bar = self.menu_bar_items(h)[i];
        let width = item.sub.iter().map(|s| tw(&s.text)).max().unwrap_or(40) + 32;
        (0..item.sub.len() as i32).map(|k| Rect::new(bar.left, bar.bottom + k * ROW_H, bar.left + width, bar.bottom + (k + 1) * ROW_H)).collect()
    }

    /// Mouse press while a menu is involved. Returns true if the click was consumed.
    pub(crate) fn menu_click(&mut self, p: Point, hit_menu: Option<Hwnd>) -> bool {
        if let Some((h, i)) = self.open_menu {
            let rows = self.dropdown_rows();
            if let Some(k) = rows.iter().position(|r| r.contains(p)) {
                let id = self.wins[&h].menu[i].sub[k].id;
                self.open_menu = None;
                self.post_message(h, WM_COMMAND, id as usize, 0).ok();
                return true;
            }
            self.open_menu = None;
            if hit_menu.is_none() {
                return false;
            }
        }
        let Some(h) = hit_menu else { return false };
        let Some(i) = self.menu_bar_items(h).iter().position(|r| r.contains(p)) else { return true };
        let item = &self.wins[&h].menu[i];
        if item.sub.is_empty() {
            let id = item.id;
            self.post_message(h, WM_COMMAND, id as usize, 0).ok();
        } else {
            self.open_menu = Some((h, i));
        }
        true
    }

    pub(crate) fn draw_menus(&self, fb: &mut [u32], tops: &[Hwnd]) {
        use loos_shell::draw::{Canvas, Rect as SRect};
        let (sw, sh) = self.screen;
        let fill = |fb: &mut [u32], r: Rect, c: u32| crate::gdi::fill(fb, sw, r.intersect(&Rect::new(0, 0, sw, sh)), c);
        for &t in tops {
            if self.wins.get(&t).is_none_or(|w| w.menu.is_empty()) || !self.is_window_visible(t) {
                continue;
            }
            fill(fb, self.menu_bar_rect(t), 0xFFD4D0C8);
            let items = self.menu_bar_items(t);
            for (k, r) in items.iter().enumerate() {
                if self.open_menu == Some((t, k)) {
                    fill(fb, *r, 0xFF0A246A);
                }
                let col = if self.open_menu == Some((t, k)) { 0xFFFFFFFF } else { 0xFF000000 };
                let mut c = Canvas { px: &mut fb[..(sw * sh) as usize], w: sw, h: sh, stride: sw, scale: 0 };
                text::draw(&mut c, SRect::new(0, 0, sw, sh), r.left + 8, r.top + 3, self.wins[&t].menu[k].text.as_bytes(), col, &font_data::regular(), 14);
            }
        }
        if let Some((h, i)) = self.open_menu {
            let rows = self.dropdown_rows();
            if let Some(w) = self.wins.get(&h) {
                if let Some(first) = rows.first() {
                    let last = rows.last().unwrap();
                    fill(fb, Rect::new(first.left - 1, first.top - 1, last.right + 1, last.bottom + 1), 0xFF000000);
                }
                for (k, r) in rows.iter().enumerate() {
                    fill(fb, *r, if r.contains(self.cursor) { 0xFF0A246A } else { 0xFFD4D0C8 });
                    let col = if r.contains(self.cursor) { 0xFFFFFFFF } else { 0xFF000000 };
                    let mut c = Canvas { px: &mut fb[..(sw * sh) as usize], w: sw, h: sh, stride: sw, scale: 0 };
                    text::draw(&mut c, SRect::new(0, 0, sw, sh), r.left + 16, r.top + 3, w.menu[i].sub[k].text.as_bytes(), col, &font_data::regular(), 14);
                }
            }
        }
    }
}
