//! Looks and behaviour: whole-desktop presets, the window-manager / taskbar / display keys behind Settings > Behavior,
//! and the night-light filter. Everything is a plain `ui.*` / `wm.*` config key, so a Look is just a named bundle of
//! keys and anything a Look sets can be changed afterwards one by one.
#![allow(missing_docs)]

use crate::backend::{cfg_bool, cfg_int, cfg_set_bool, cfg_set_int, ShellBackend};
use crate::compositor::Compositor;
use crate::draw::Rect;
use crate::ffi::Fb;
use crate::taskbar::ShellTaskbar;
use crate::wm::{self, ShellWm, FOCUS_CLICK, FOCUS_FOLLOWS_MOUSE};

/// One config entry of a Look. Values are integers/booleans/strings in the config store's text form.
pub type Entry = (&'static str, &'static str);

/// A named bundle of keys plus the three colours the Settings card previews it with.
pub struct Look {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub keys: &'static [Entry],
    /// Preview: wallpaper, window surface, accent.
    pub preview: [u32; 3],
    /// Preview: taskbar at the top, icons centered.
    pub bar_top: bool,
    pub bar_centered: bool,
}

pub const LOOKS: [Look; 8] = [
    Look {
        id: "loos",
        name: "LoOS",
        blurb: "Dark, soft corners, centered taskbar.",
        keys: &[
            ("ui.base", "dark"), ("ui.accent_hex", "#3B82F6"), ("ui.tint", "20"), ("ui.radius", "-1"), ("ui.title_h", "0"),
            ("ui.border", "0"), ("ui.shadow", "100"), ("ui.glow", "100"), ("ui.opacity", "92"), ("ui.icons", "0"),
            ("ui.wallpaper", "0"), ("ui.taskbar.center", "1"), ("ui.taskbar.labels", "0"), ("ui.taskbar.top", "0"),
            ("ui.taskbar.size", "1"), ("wm.tiling", "0"), ("wm.gap", "6"), ("wm.focus", "0"), ("ui.reduce_motion", "0"),
        ],
        preview: [0xFF0E1522, 0xFF1B2433, 0xFF3B82F6],
        bar_top: false,
        bar_centered: true,
    },
    Look {
        id: "classic",
        name: "Classic panel",
        blurb: "Left-aligned bar with window titles, tight corners.",
        keys: &[
            ("ui.base", "dark"), ("ui.accent_hex", "#10B981"), ("ui.tint", "0"), ("ui.radius", "2"), ("ui.title_h", "26"),
            ("ui.border", "1"), ("ui.shadow", "60"), ("ui.glow", "0"), ("ui.opacity", "100"), ("ui.icons", "2"),
            ("ui.wallpaper", "1"), ("ui.taskbar.center", "0"), ("ui.taskbar.labels", "1"), ("ui.taskbar.top", "0"),
            ("ui.taskbar.size", "1"), ("wm.tiling", "0"), ("wm.gap", "6"), ("wm.focus", "0"), ("ui.reduce_motion", "0"),
        ],
        preview: [0xFF1B2A2A, 0xFF26323A, 0xFF10B981],
        bar_top: false,
        bar_centered: false,
    },
    Look {
        id: "dock",
        name: "Floating dock",
        blurb: "Big round icons, deep shadows, aurora wallpaper.",
        keys: &[
            ("ui.base", "nord"), ("ui.accent_hex", "#88C0D0"), ("ui.tint", "40"), ("ui.radius", "16"), ("ui.title_h", "32"),
            ("ui.border", "0"), ("ui.shadow", "130"), ("ui.glow", "120"), ("ui.opacity", "86"), ("ui.icons", "1"),
            ("ui.wallpaper", "2"), ("ui.taskbar.center", "1"), ("ui.taskbar.labels", "0"), ("ui.taskbar.top", "0"),
            ("ui.taskbar.size", "2"), ("wm.tiling", "0"), ("wm.gap", "10"), ("wm.focus", "0"), ("ui.reduce_motion", "0"),
        ],
        preview: [0xFF2E3440, 0xFF3B4252, 0xFF88C0D0],
        bar_top: false,
        bar_centered: true,
    },
    Look {
        id: "tiler",
        name: "Tiler",
        blurb: "Top bar, no decoration, windows tile themselves.",
        keys: &[
            ("ui.base", "gruvbox"), ("ui.accent_hex", "#FABD2F"), ("ui.tint", "0"), ("ui.radius", "0"), ("ui.title_h", "22"),
            ("ui.border", "2"), ("ui.shadow", "0"), ("ui.glow", "0"), ("ui.opacity", "100"), ("ui.icons", "2"),
            ("ui.wallpaper", "3"), ("ui.taskbar.center", "0"), ("ui.taskbar.labels", "1"), ("ui.taskbar.top", "1"),
            ("ui.taskbar.size", "0"), ("wm.tiling", "1"), ("wm.gap", "8"), ("wm.focus", "1"), ("ui.reduce_motion", "1"),
        ],
        preview: [0xFF282828, 0xFF3C3836, 0xFFFABD2F],
        bar_top: true,
        bar_centered: false,
    },
    Look {
        id: "paper",
        name: "Paper",
        blurb: "Light, calm, almost no effects.",
        keys: &[
            ("ui.base", "paper"), ("ui.accent_hex", "#2563EB"), ("ui.tint", "30"), ("ui.radius", "6"), ("ui.title_h", "0"),
            ("ui.border", "1"), ("ui.shadow", "70"), ("ui.glow", "0"), ("ui.opacity", "96"), ("ui.icons", "0"),
            ("ui.wallpaper", "1"), ("ui.taskbar.center", "1"), ("ui.taskbar.labels", "0"), ("ui.taskbar.top", "0"),
            ("ui.taskbar.size", "1"), ("wm.tiling", "0"), ("wm.gap", "6"), ("wm.focus", "0"), ("ui.reduce_motion", "0"),
        ],
        preview: [0xFFE8E2D4, 0xFFF7F3EA, 0xFF2563EB],
        bar_top: false,
        bar_centered: true,
    },
    Look {
        id: "midnight",
        name: "Midnight",
        blurb: "True black for OLED, violet accent, gentle glow.",
        keys: &[
            ("ui.base", "amoled"), ("ui.accent_hex", "#A78BFA"), ("ui.tint", "10"), ("ui.radius", "12"), ("ui.title_h", "30"),
            ("ui.border", "1"), ("ui.shadow", "100"), ("ui.glow", "60"), ("ui.opacity", "90"), ("ui.icons", "1"),
            ("ui.wallpaper", "0"), ("ui.taskbar.center", "1"), ("ui.taskbar.labels", "0"), ("ui.taskbar.top", "0"),
            ("ui.taskbar.size", "1"), ("wm.tiling", "0"), ("wm.gap", "6"), ("wm.focus", "0"), ("ui.reduce_motion", "0"),
        ],
        preview: [0xFF000000, 0xFF0B0B10, 0xFFA78BFA],
        bar_top: false,
        bar_centered: true,
    },
    Look {
        id: "lean",
        name: "Lean",
        blurb: "Flat and cheap to draw: no shadows, glow or motion.",
        keys: &[
            ("ui.base", "dark"), ("ui.accent_hex", "#3B82F6"), ("ui.tint", "0"), ("ui.radius", "0"), ("ui.title_h", "24"),
            ("ui.border", "1"), ("ui.shadow", "0"), ("ui.glow", "0"), ("ui.opacity", "100"), ("ui.icons", "2"),
            ("ui.wallpaper", "3"), ("ui.taskbar.center", "0"), ("ui.taskbar.labels", "0"), ("ui.taskbar.top", "0"),
            ("ui.taskbar.size", "0"), ("wm.tiling", "0"), ("wm.gap", "0"), ("wm.focus", "0"), ("ui.reduce_motion", "1"),
        ],
        preview: [0xFF121824, 0xFF1B2330, 0xFF3B82F6],
        bar_top: false,
        bar_centered: false,
    },
    Look {
        id: "large",
        name: "Large print",
        blurb: "High contrast, 150% scale, big title bars, no motion.",
        keys: &[
            ("ui.base", "hc"), ("ui.accent_hex", "#FFD400"), ("ui.tint", "0"), ("ui.radius", "4"), ("ui.title_h", "40"),
            ("ui.border", "3"), ("ui.shadow", "0"), ("ui.glow", "0"), ("ui.opacity", "100"), ("ui.icons", "2"),
            ("ui.wallpaper", "3"), ("ui.taskbar.center", "0"), ("ui.taskbar.labels", "1"), ("ui.taskbar.top", "0"),
            ("ui.taskbar.size", "2"), ("wm.tiling", "0"), ("wm.gap", "8"), ("wm.focus", "0"), ("ui.reduce_motion", "1"),
            ("ui.scale", "150"),
        ],
        preview: [0xFF000000, 0xFF000000, 0xFFFFD400],
        bar_top: false,
        bar_centered: false,
    },
];

fn z(s: &str, out: &mut [u8]) -> usize {
    let n = s.len().min(out.len() - 1);
    out[..n].copy_from_slice(&s.as_bytes()[..n]);
    out[n] = 0;
    n
}

/// Writes every key of `look` to the backend and removes per-token colour overrides (a Look is a clean slate).
/// The caller re-reads the theme (`settings::apply`) and the behaviour keys (`apply_behavior`) afterwards.
pub fn store_look(be: &ShellBackend, look: &Look) {
    for name in crate::theme::TOKEN_NAMES.iter() {
        let k = crate::theme::key_for(b"ui.c.", name);
        be.cfg_set(&k, b"\0");
    }
    let (mut k, mut v) = ([0u8; 48], [0u8; 24]);
    for (key, val) in look.keys {
        z(key, &mut k);
        z(val, &mut v);
        be.cfg_set(&k, &v);
    }
    z(look.id, &mut v);
    be.cfg_set(b"ui.look\0", &v);
}

/// Index of the Look recorded by the last [`store_look`], if its id is still known.
pub fn current_look(be: &ShellBackend) -> Option<usize> {
    let mut v = [0u8; 24];
    if !be.cfg_get(b"ui.look\0", &mut v) {
        return None;
    }
    let n = v.iter().position(|&b| b == 0).unwrap_or(v.len());
    LOOKS.iter().position(|l| l.id.as_bytes() == &v[..n])
}

pub fn look_index(id: &str) -> Option<usize> {
    LOOKS.iter().position(|l| l.id == id)
}

/// Pushes the `wm.*`, taskbar and display keys onto the live window manager, taskbar and compositor.
/// Returns the night-light strength (0 = off) for the final blit.
pub fn apply_behavior(be: &ShellBackend, wm: &mut ShellWm, c: &mut Compositor, tb: &mut ShellTaskbar) -> i32 {
    wm.gap = cfg_int(be, b"wm.gap\0", 6).clamp(0, 24);
    wm.master_pct = cfg_int(be, b"wm.master\0", 55).clamp(30, 70);
    wm.focus_mode = if cfg_int(be, b"wm.focus\0", 0) == 1 { FOCUS_FOLLOWS_MOUSE } else { FOCUS_CLICK };
    let want = cfg_bool(be, b"wm.tiling\0", false);
    if want || wm.tiling {
        wm::set_tiling(wm, want); // also re-tiles with the new gap / master width
    }
    crate::taskbar::configure(
        tb,
        cfg_bool(be, b"ui.taskbar.center\0", true),
        cfg_bool(be, b"ui.taskbar.labels\0", false),
        cfg_bool(be, b"ui.taskbar.top\0", false),
        cfg_int(be, b"ui.taskbar.size\0", 1),
    );
    c.damage_all();
    cfg_int(be, b"ui.night\0", 0).clamp(0, 100)
}

/// Stores the three tiling keys; kept here so Settings and the quick actions agree on names and ranges.
pub fn store_tiling(be: &ShellBackend, on: bool, gap: i32, master: i32) {
    cfg_set_bool(be, b"wm.tiling\0", on);
    cfg_set_int(be, b"wm.gap\0", gap.clamp(0, 24));
    cfg_set_int(be, b"wm.master\0", master.clamp(30, 70));
}

/// Night light: warms the dirty rectangle of the final image by scaling green and blue down. `strength` is 0..=100.
/// Done once on the way out so it covers every window and the wallpaper without touching any app.
pub fn night_tint(fb: &mut Fb, r: Rect, strength: i32) {
    if strength <= 0 || fb.px.is_null() || r.is_empty() {
        return;
    }
    let s = strength.min(100) as u32;
    let (kg, kb) = (256 - s * 64 / 100, 256 - s * 140 / 100);
    let r = r.intersect(Rect::new(0, 0, fb.w, fb.h));
    if r.is_empty() {
        return;
    }
    for y in r.y..r.y + r.h {
        // SAFETY: the rectangle is clipped to the framebuffer, whose rows are `stride` pixels apart.
        let row = unsafe { core::slice::from_raw_parts_mut(fb.px.add((y * fb.stride + r.x) as usize), r.w as usize) };
        for p in row {
            let v = *p;
            let (g, b) = (((v >> 8) & 0xFF) * kg >> 8, (v & 0xFF) * kb >> 8);
            *p = (v & 0xFFFF_0000) | g << 8 | b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_mem::MemBackend;
    use alloc::boxed::Box;

    #[test]
    fn every_look_has_valid_unique_ids_and_known_values() {
        for (i, l) in LOOKS.iter().enumerate() {
            assert!(LOOKS.iter().skip(i + 1).all(|o| o.id != l.id), "duplicate id {}", l.id);
            assert!(l.keys.iter().any(|(k, _)| *k == "ui.base"));
            for (k, v) in l.keys {
                assert!(k.starts_with("ui.") || k.starts_with("wm."), "{k}");
                assert!(k.len() < 40 && v.len() < 24);
            }
        }
    }

    #[test]
    fn storing_a_look_clears_overrides_and_is_remembered() {
        let mut m = Box::new(MemBackend::new());
        let be = m.vtable();
        be.cfg_set(b"ui.c.surface\0", b"#102030\0");
        store_look(&be, &LOOKS[3]);
        assert_eq!(current_look(&be), Some(3));
        let mut v = [0u8; 16];
        assert!(be.cfg_get(b"ui.c.surface\0", &mut v));
        assert_eq!(v[0], 0, "override emptied");
        assert!(be.cfg_get(b"ui.base\0", &mut v));
        assert_eq!(&v[..8], b"gruvbox\0");
        assert!(cfg_bool(&be, b"wm.tiling\0", false));
    }

    #[test]
    fn night_tint_only_touches_green_and_blue_inside_the_rect() {
        let mut px = [0xFFFF_FFFFu32; 16];
        let mut fb = Fb { px: px.as_mut_ptr(), w: 4, h: 4, stride: 4, scale: 100 };
        night_tint(&mut fb, Rect::new(1, 1, 2, 2), 100);
        assert_eq!(px[0], 0xFFFF_FFFF);
        let p = px[5];
        assert_eq!(p >> 16, 0xFFFF, "alpha and red untouched");
        assert!((p >> 8) & 0xFF < 0xFF && (p & 0xFF) < (p >> 8) & 0xFF, "blue cut more than green");
        let before = px;
        night_tint(&mut fb, Rect::new(0, 0, 4, 4), 0);
        assert_eq!(before, px, "strength 0 is a no-op");
    }
}
