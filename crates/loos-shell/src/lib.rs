//! LoOS desktop shell, being ported from C (`src/shell`) module by module.
//! Phase 1: geometry, colors, themes, the software rasterizer primitives and damage tracking.
//! Phase 2: the outline rasteriser, font engine and text layout/rendering.
//! [`ffi`] exports them under the exact C names and layouts of `src/shell/shell.h`.
#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod apphost;
pub mod apps;
pub mod backend;
pub mod backend_mem;
pub mod compositor;
pub mod desktop;
pub mod draw;
pub mod ffi;
pub mod font;
pub mod filemgr;
/// Generated Inter outline tables and the `loos_font_*` C statics.
pub mod font_data;
pub mod installer;
pub mod inventory;
pub mod launcher;
pub mod look;
pub mod looks_ui;
pub mod notify;
pub mod perms;
pub mod raster;
pub mod session;
pub mod settings;
pub mod taskbar;
pub mod taskmon;
pub mod text;
pub mod theme;
pub mod ui;
pub mod util;
pub mod web;
pub mod wm;

pub use draw::{Canvas, Damage, Rect, Theme};

/// Serializes tests that exercise the single-threaded shell statics (raster scratch, glyph cache).
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
