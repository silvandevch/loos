//! Settings pages "Looks" (one-click whole-desktop presets with live previews) and "Behavior" (window manager,
//! display and a shortcut reference). Both are thin: they only read and write the `ui.*` / `wm.*` config keys.
#![allow(missing_docs)]

use crate::backend::{cfg_bool, cfg_int, cfg_set_bool, cfg_set_int};
use crate::draw::{mix, Rect};
use crate::ffi;
use crate::look::{self, LOOKS};
use crate::settings::{be, changed, pers_slider, ShellSettings};
use crate::ui::{self, UiCtx};

const CARD_H: i32 = 84;
const CARD_GAP: i32 = 10;

fn apply_look(s: &mut ShellSettings, i: usize) {
    look::store_look(be(s), &LOOKS[i]);
    changed(s);
}

/// The little desktop drawn on every Look card: wallpaper, one window with an accent title strip, and the bar.
fn preview(u: &mut UiCtx, r: Rect, l: &look::Look) {
    let [bg, surf, acc] = l.preview;
    unsafe {
        ffi::fb_fill_round(u.fb, u.clip, r, 8, bg);
        let bar = Rect::new(r.x, if l.bar_top { r.y } else { r.y + r.h - 9 }, r.w, 9);
        ffi::fb_fill_round(u.fb, u.clip, bar, 3, mix(surf, 0xFF00_0000, 40));
        let win = Rect::new(r.x + 14, r.y + if l.bar_top { 16 } else { 8 }, r.w - 34, r.h - 30);
        ffi::fb_fill_round(u.fb, u.clip, win, 4, surf);
        ffi::fb_fill_round(u.fb, u.clip, Rect::new(win.x, win.y, win.w, 6), 3, acc);
        for k in 0..3 {
            let cx = if l.bar_centered { r.x + r.w / 2 - 8 + k * 8 } else { r.x + 8 + k * 8 };
            ffi::fb_fill_circle(u.fb, u.clip, cx, bar.y + 4, 2, acc);
        }
    }
}

pub fn page_looks(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    let saved = ui::clip_push(u, c);
    let total = 76 + LOOKS.len() as i32 * (CARD_H + CARD_GAP);
    let max_scroll = (total - c.h).max(0);
    if ui::hovered(u, c) && u.wheel != 0 {
        s.p_scroll -= u.wheel * 24;
    }
    s.p_scroll = s.p_scroll.clamp(0, max_scroll);
    let oy = c.y - s.p_scroll;
    ui::heading(u, c.x, oy, b"Looks");
    ui::label(u, c.x, oy + 30, b"One click restyles the whole desktop. Everything it sets stays editable in Personalize and Behavior.", t.text_dim);
    let cur = look::current_look(be(s)).or(Some(0)); // never chosen = the stock LoOS look
    for (i, l) in LOOKS.iter().enumerate() {
        let r = Rect::new(c.x, oy + 62 + i as i32 * (CARD_H + CARD_GAP), c.w, CARD_H);
        let active = cur == Some(i);
        let hot = ui::hovered(u, r);
        let fill = if active { mix(t.surface_hi, t.accent, 60) } else if hot { mix(t.surface, t.surface_hi, 150) } else { mix(t.surface, t.desktop, 120) };
        unsafe {
            ffi::fb_fill_round(u.fb, u.clip, r, 10, fill);
            ffi::fb_stroke_round(u.fb, u.clip, r, 10, if active { 2 } else { 1 }, if active { t.accent } else { t.border });
        }
        preview(u, Rect::new(r.x + 12, r.y + 12, 112, r.h - 24), l);
        ui::label(u, r.x + 140, r.y + 16, l.name.as_bytes(), t.text);
        ui::label(u, r.x + 140, r.y + 42, l.blurb.as_bytes(), t.text_dim);
        if active {
            ui::label_right(u, r.x + r.w - 14, r.y + 16, b"Active", t.accent);
        }
        if u.pressed && hot && !active && r.y >= c.y - CARD_H && r.y < c.y + c.h {
            apply_look(s, i);
        }
    }
    ui::clip_pop(u, saved);
}

const SHORTCUTS: [(&str, &str); 9] = [
    ("Super", "Start and search"),
    ("Ctrl+Space", "Command palette"),
    ("Alt+Tab", "Switch windows"),
    ("Super+Left / Right", "Snap to a half (again to release)"),
    ("Super+Up / Down", "Maximize / restore or minimize"),
    ("Super+1 .. 8", "Go to desktop (add Shift to send the window)"),
    ("Super+T", "Tile windows on or off"),
    ("Drag to an edge", "Snap preview (corners give quarters)"),
    ("Double-click title", "Maximize or restore"),
];

pub fn page_behavior(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    let saved = ui::clip_push(u, c);
    let total = 470;
    s.p_scroll = if ui::hovered(u, c) && u.wheel != 0 { s.p_scroll - u.wheel * 24 } else { s.p_scroll }.clamp(0, (total - c.h).max(0));
    let oy = c.y - s.p_scroll;
    let half = (c.w - 16) / 2;
    let x2 = c.x + half + 16;
    ui::heading(u, c.x, oy, b"Behavior");

    ui::label(u, c.x, oy + 40, b"Windows", t.text_dim);
    let mut tile = cfg_bool(be(s), b"wm.tiling\0", false);
    if ui::toggle(u, Rect::new(c.x, oy + 62, half, 26), b"Tile windows automatically", &mut tile) {
        cfg_set_bool(be(s), b"wm.tiling\0", tile);
        changed(s);
    }
    let mut follow = cfg_int(be(s), b"wm.focus\0", 0) == 1;
    if ui::toggle(u, Rect::new(x2, oy + 62, half, 26), b"Focus follows the mouse", &mut follow) {
        cfg_set_int(be(s), b"wm.focus\0", follow as i32);
        changed(s);
    }
    let mut gap = cfg_int(be(s), b"wm.gap\0", 6);
    if pers_slider(u, c.x, oy + 100, half - 8, b"Gap between windows", &mut gap, 0, 24) {
        cfg_set_int(be(s), b"wm.gap\0", gap);
        changed(s);
    }
    let mut master = cfg_int(be(s), b"wm.master\0", 55);
    if pers_slider(u, x2, oy + 100, half - 8, b"Main window width %", &mut master, 30, 70) {
        cfg_set_int(be(s), b"wm.master\0", master);
        changed(s);
    }

    ui::label(u, c.x, oy + 160, b"Display", t.text_dim);
    let mut night = cfg_int(be(s), b"ui.night\0", 0);
    if pers_slider(u, c.x, oy + 182, half - 8, b"Night light %", &mut night, 0, 100) {
        cfg_set_int(be(s), b"ui.night\0", night);
        changed(s);
    }
    crate::settings::cfg_toggle(s, u, Rect::new(x2, oy + 204, half, 26), b"Reduce motion", b"ui.reduce_motion\0", false);

    ui::label(u, c.x, oy + 252, b"Keyboard shortcuts", t.text_dim);
    for (i, (k, d)) in SHORTCUTS.iter().enumerate() {
        let y = oy + 274 + i as i32 * 20;
        ui::label(u, c.x + 4, y, k.as_bytes(), t.text);
        ui::label(u, c.x + 190, y, d.as_bytes(), t.text_dim);
    }
    if ui::button(u, Rect::new(c.x, oy + 274 + SHORTCUTS.len() as i32 * 20 + 14, 220, 28), b"Back to the LoOS look") {
        apply_look(s, 0);
    }
    ui::clip_pop(u, saved);
}
