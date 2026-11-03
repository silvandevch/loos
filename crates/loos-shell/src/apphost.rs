//! Hosts an immediate-mode app frame function inside a compositor window (was `src/shell/apphost.c`).
//! The client surface is device resolution; the app lays out in logical px, so pointer events are scaled back.
#![allow(missing_docs)]

use crate::compositor::{ev, Compositor, ShellEvent};
use crate::draw::Rect;
use crate::ffi::Fb;
use crate::ui::{self, UiCtx};
use core::ffi::c_void;

pub type FrameFn = unsafe extern "C" fn(*mut c_void, *mut UiCtx, i32, i32);

#[repr(C)]
pub struct AppWindow {
    pub comp: *mut Compositor,
    pub win: i32,
    pub px: *mut u32,
    pub cap_w: i32,
    pub cap_h: i32,
    pub w: i32,
    pub h: i32,
    pub frame: Option<FrameFn>,
    pub app: *mut c_void,
    pub ui: UiCtx,
    pub dirty: bool,
    pub closed: bool,
    pub frames: u32,
    pub on_close: Option<unsafe extern "C" fn(*mut c_void)>,
}

impl AppWindow {
    pub fn is_open(&self) -> bool {
        !self.comp.is_null() && !self.closed
    }

    /// Creates the compositor window and paints the first frame. `aw` must be zero-initialised or previously closed.
    ///
    /// # Safety
    /// `c` must outlive the window and `px` must hold `cap_w * cap_h` pixels.
    pub unsafe fn open(
        &mut self, c: *mut Compositor, title: &[u8], rect: Rect, flags: u32, owner: u32, px: *mut u32, cap_w: i32, cap_h: i32, frame: FrameFn,
        app: *mut c_void,
    ) -> bool {
        // SAFETY: AppWindow is plain data (raw pointers, ints, Option<fn>); all-zero is a valid empty value.
        unsafe { core::ptr::write_bytes(self as *mut Self, 0, 1) };
        self.comp = c;
        self.px = px;
        self.cap_w = cap_w;
        self.cap_h = cap_h;
        self.frame = Some(frame);
        self.app = app;
        let comp = unsafe { &mut *c };
        self.win = comp.create(title, rect, flags, owner);
        if self.win == 0 {
            return false;
        }
        let Some(wp) = comp.get_mut(self.win).map(|w| w as *mut crate::compositor::Window) else { return false };
        // SAFETY: `wp` points into the compositor's window table, which is not modified by client_rect.
        let cr = comp.client_rect(unsafe { &*wp });
        self.w = cr.w.min(cap_w);
        self.h = cr.h.min(cap_h);
        comp.set_client(self.win, px, self.w, self.h, cap_w);
        self.dirty = true;
        self.redraw();
        true
    }

    fn win_scale(&self) -> i32 {
        let c = unsafe { &mut *self.comp };
        match c.get_mut(self.win) {
            Some(w) if w.dpi >= 48 => (w.dpi * 100 / 96) as i32,
            _ => 100,
        }
    }

    pub fn redraw(&mut self) {
        let sc = self.win_scale();
        let mut fb = Fb { px: self.px, w: self.w, h: self.h, stride: self.cap_w, scale: sc };
        let c = unsafe { &mut *self.comp };
        ui::begin(&mut self.ui, &mut fb, &c.theme, 100);
        self.ui.reduce_motion = c.reduce_motion;
        self.ui.high_contrast = c.high_contrast;
        let (lw, lh) = (fb.w * 100 / sc, fb.h * 100 / sc);
        if let Some(f) = self.frame {
            // SAFETY: the frame function belongs to `app`, which outlives the window (desktop contract).
            unsafe { f(self.app, &mut self.ui, lw, lh) };
        }
        ui::end(&mut self.ui);
        self.dirty = false;
        self.frames = self.frames.wrapping_add(1);
        c.invalidate(self.win, Rect::new(0, 0, self.w, self.h));
    }

    pub fn close(&mut self) {
        if self.closed || self.comp.is_null() {
            return;
        }
        self.closed = true;
        unsafe { &mut *self.comp }.destroy(self.win);
    }

    /// Drains the window's events into the UI context and repaints when something changed.
    pub fn pump(&mut self) {
        if self.closed || self.comp.is_null() {
            return;
        }
        let c = unsafe { &mut *self.comp };
        if c.get_mut(self.win).is_none() {
            return;
        }
        let mut e = ShellEvent::default();
        let mut any = false;
        self.ui.now_ms = c.now_ms;
        while c.pop_event(self.win, &mut e) {
            match e.ev_type {
                ev::RESIZE => {
                    self.w = e.x.min(self.cap_w);
                    self.h = e.y.min(self.cap_h);
                    c.set_client(self.win, self.px, self.w, self.h, self.cap_w);
                    self.dirty = true;
                }
                ev::CLOSE_REQUEST => {
                    match self.on_close {
                        // SAFETY: callback installed by the owner with `app` as its argument.
                        Some(f) => unsafe { f(self.app) },
                        None => self.close(),
                    }
                    return;
                }
                ev::DPI_CHANGED => self.dirty = true,
                t => {
                    if matches!(t, ev::MOUSE_MOVE | ev::MOUSE_DOWN | ev::MOUSE_UP | ev::MOUSE_WHEEL) {
                        let sc = self.win_scale();
                        e.x = e.x * 100 / sc; // device px -> logical px
                        e.y = e.y * 100 / sc;
                    }
                    ui::feed(&mut self.ui, &e);
                    any = true;
                }
            }
            if e.ev_type == ev::FOCUS || e.ev_type == ev::BLUR {
                self.dirty = true;
            }
        }
        if any || self.dirty {
            self.redraw();
        }
    }
}
