//! Wayland compositor protocol for the Linux layer.
//!
//! Transport-independent: the socket layer feeds each client `sendmsg` to
//! [`Server::receive_with_fds`] and drains encoded events (plus the fds that must travel with them)
//! with [`Server::take_events`] / [`Server::take_out_fds`]. Implements what real toolkits (GTK,
//! SDL, Firefox's widget layer) bind on a desktop compositor: wl_compositor/region/subcompositor,
//! wl_shm with live pools, wl_seat with pointer and keyboard, wl_output, xdg-shell (toplevels,
//! popups, positioners) and a data-device stub. Object ids are recycled through
//! `wl_display.delete_id`, and every event respects the version the client bound.

use crate::mem::SharedBuf;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

/// Protocol parsing or object-state failure (the connection is dropped, like `wl_display.error`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The request header or payload is truncated/malformed.
    Malformed,
    /// The request references an unknown object.
    UnknownObject,
    /// The request is outside the implemented protocol.
    Unsupported,
    /// The request violates object lifecycle rules.
    InvalidState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Iface {
    Display,
    Registry,
    Callback,
    Compositor,
    Region,
    Subcompositor,
    Subsurface,
    Shm,
    ShmPool,
    Buffer,
    Surface,
    Seat,
    Pointer,
    Keyboard,
    Touch,
    Output,
    XdgWmBase,
    XdgPositioner,
    XdgSurface,
    XdgToplevel,
    XdgPopup,
    DataDeviceManager,
    DataSource,
    DataDevice,
    DecorationManager,
    ToplevelDecoration,
}

#[derive(Debug, Default)]
struct Surface {
    pending: Option<u32>,
    attached: Option<u32>,
    frame_cbs: Vec<u32>,
    pending_cbs: Vec<u32>,
    xdg: Option<u32>,
    role_top: Option<u32>,
}

#[derive(Debug)]
struct Obj {
    iface: Iface,
    version: u32,
    surface: Option<u32>,
    pool: Option<Pool>,
    buffer: Option<Buffer>,
    surf: Option<Surface>,
    title: String,
    app_id: String,
    /// `xdg_surface.set_window_geometry` (x, y, w, h): the visible window inside the buffer
    /// (client-side shadows lie outside it).
    geometry: Option<(u32, u32, u32, u32)>,
}

#[derive(Debug, Clone)]
struct Pool {
    buf: SharedBuf,
    size: usize,
}

#[derive(Clone, Debug)]
struct Buffer {
    pool: Pool,
    offset: usize,
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
}

/// A file descriptor carried by `SCM_RIGHTS`: the file's live page cache.
#[derive(Clone, Debug)]
pub struct SharedFd {
    /// Shared pages of the file (also mapped by the client).
    pub buf: SharedBuf,
}

/// A committed wl_shm frame ready for compositor presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// wl_surface object id (the window key).
    pub surface: u32,
    /// Buffer width in pixels.
    pub width: u32,
    /// Buffer height in pixels.
    pub height: u32,
    /// Pixels in LoOS `0xAARRGGBB` format.
    pub pixels: Vec<u32>,
    /// `xdg_toplevel.set_title`.
    pub title: Vec<u8>,
    /// `xdg_toplevel.set_app_id`.
    pub app_id: Vec<u8>,
    /// False for popups/menus (no window decoration, no taskbar entry).
    pub toplevel: bool,
}

/// A global advertised by the registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Global {
    /// Registry numeric name.
    pub name: u32,
    /// Interface name.
    pub interface: &'static str,
    /// Maximum supported protocol version.
    pub version: u32,
}

const GLOBALS: [(&str, u32, Iface); 9] = [
    ("wl_compositor", 5, Iface::Compositor),
    ("wl_subcompositor", 1, Iface::Subcompositor),
    ("wl_shm", 1, Iface::Shm),
    ("wl_seat", 7, Iface::Seat),
    ("wl_output", 4, Iface::Output),
    ("xdg_wm_base", 5, Iface::XdgWmBase),
    ("wl_data_device_manager", 3, Iface::DataDeviceManager),
    ("zxdg_decoration_manager_v1", 1, Iface::DecorationManager),
    ("wl_display", 0, Iface::Display), // sentinel, never advertised
];

/// Path of the XKB keymap sent to clients (generated into the rootfs); `/dev/null` means "no keymap".
pub const KEYMAP_PATH: &str = "/usr/share/loos/keymap.xkb";

/// One Wayland client connection.
pub struct Server {
    objects: BTreeMap<u32, Obj>,
    globals: Vec<Global>,
    events: Vec<u8>,
    out_fds: Vec<String>,
    in_fds: VecDeque<SharedFd>,
    frames: Vec<Frame>,
    closed: Vec<u32>,
    serial: u32,
    /// (width, height) of the output.
    pub screen: (u32, u32),
    /// Keymap file size (bytes) for `wl_keyboard.keymap`; 0 sends `no_keymap`.
    pub keymap_size: u32,
    pointer_focus: Option<u32>,
    key_focus: Option<u32>,
    time_ms: u32,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// A fresh connection (object 1 is `wl_display`).
    pub fn new() -> Self {
        let mut objects = BTreeMap::new();
        objects.insert(1, new_obj(Iface::Display, 1));
        let globals = GLOBALS.iter().take(GLOBALS.len() - 1).enumerate().map(|(i, g)| Global { name: i as u32 + 1, interface: g.0, version: g.1 }).collect();
        Self { objects, globals, events: Vec::new(), out_fds: Vec::new(), in_fds: VecDeque::new(), frames: Vec::new(), closed: Vec::new(), serial: 1, screen: (1024, 768), keymap_size: 0, pointer_focus: None, key_focus: None, time_ms: 0 }
    }

    /// The globals `wl_registry.global` advertises.
    pub fn globals(&self) -> &[Global] {
        &self.globals
    }

    /// Feeds requests without fds.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.receive_with_fds(bytes, &[])
    }

    /// Feeds one `sendmsg` worth of requests plus its `SCM_RIGHTS` fds (consumed in order).
    pub fn receive_with_fds(&mut self, bytes: &[u8], fds: &[SharedFd]) -> Result<(), Error> {
        self.in_fds.extend(fds.iter().cloned());
        let mut off = 0;
        while off < bytes.len() {
            if bytes.len() - off < 8 {
                return Err(Error::Malformed);
            }
            let object = get_u32(&bytes[off..])?;
            let word = get_u32(&bytes[off + 4..])?;
            let size = (word >> 16) as usize;
            if size < 8 || size % 4 != 0 || size > bytes.len() - off {
                return Err(Error::Malformed);
            }
            self.request(object, (word & 0xffff) as u16, &bytes[off + 8..off + size])?;
            off += size;
        }
        Ok(())
    }

    /// Takes encoded events.
    pub fn take_events(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.events)
    }

    /// Paths of files whose fds must accompany the events just taken (in order).
    pub fn take_out_fds(&mut self) -> Vec<String> {
        core::mem::take(&mut self.out_fds)
    }

    /// Takes committed frames for presentation.
    pub fn take_frames(&mut self) -> Vec<Frame> {
        core::mem::take(&mut self.frames)
    }

    /// Surfaces whose toplevel/popup role ended (their windows should go away).
    pub fn take_closed(&mut self) -> Vec<u32> {
        core::mem::take(&mut self.closed)
    }

    /// The compositor showed the last frames: fire pending `wl_surface.frame` callbacks.
    pub fn frames_presented(&mut self, time_ms: u32) {
        self.time_ms = time_ms;
        let ids: Vec<u32> = self.objects.iter().filter(|(_, o)| o.surf.as_ref().is_some_and(|s| !s.frame_cbs.is_empty())).map(|(&k, _)| k).collect();
        for id in ids {
            let cbs = core::mem::take(&mut self.objects.get_mut(&id).unwrap().surf.as_mut().unwrap().frame_cbs);
            for cb in cbs {
                self.done(cb, time_ms);
            }
        }
    }

    /// Ask the toplevel showing `surface` to close (the user clicked the window's close button).
    pub fn close(&mut self, surface: u32) {
        if let Some(top) = self.objects.get(&surface).and_then(|o| o.surf.as_ref()).and_then(|s| s.role_top) {
            self.event(top, 1, &[]);
        }
    }

    /// Pointer moved to surface-local (x, y) over `surface`.
    pub fn pointer_motion(&mut self, surface: u32, x: i32, y: i32, time_ms: u32) {
        let (fx, fy) = ((x * 256) as u32, (y * 256) as u32);
        let pointers = self.with(Iface::Pointer);
        if self.pointer_focus != Some(surface) {
            let serial = self.next_serial();
            for &p in &pointers {
                if let Some(old) = self.pointer_focus.filter(|o| self.objects.contains_key(o)) {
                    self.event(p, 1, &words(&[serial, old]));
                }
                self.event(p, 0, &words(&[serial, surface, fx, fy]));
            }
            self.pointer_focus = Some(surface);
        }
        for &p in &pointers {
            self.event(p, 2, &words(&[time_ms, fx, fy]));
            self.pointer_frame(p);
        }
    }

    /// Mouse button (Linux `BTN_*` code) pressed or released over the focused surface.
    pub fn pointer_button(&mut self, button: u32, pressed: bool, time_ms: u32) {
        if self.pointer_focus.is_none() {
            return;
        }
        let serial = self.next_serial();
        for p in self.with(Iface::Pointer) {
            self.event(p, 3, &words(&[serial, time_ms, button, pressed as u32]));
            self.pointer_frame(p);
        }
        if pressed {
            if let Some(s) = self.pointer_focus {
                self.keyboard_focus(s);
            }
        }
    }

    /// Vertical wheel: `clicks` notches (positive = down).
    pub fn pointer_axis(&mut self, clicks: i32, time_ms: u32) {
        for p in self.with(Iface::Pointer) {
            self.event(p, 4, &words(&[time_ms, 0, (clicks * 10 * 256) as u32]));
            self.pointer_frame(p);
        }
    }

    /// Give keyboard focus to `surface`.
    pub fn keyboard_focus(&mut self, surface: u32) {
        if self.key_focus == Some(surface) {
            return;
        }
        let serial = self.next_serial();
        for k in self.with(Iface::Keyboard) {
            if let Some(old) = self.key_focus.filter(|o| self.objects.contains_key(o)) {
                self.event(k, 2, &words(&[serial, old]));
            }
            let mut b = words(&[serial, surface]);
            put_u32(&mut b, 0); // empty pressed-keys array
            self.event(k, 1, &b);
            self.event(k, 4, &words(&[serial, 0, 0, 0, 0]));
        }
        self.key_focus = Some(surface);
    }

    /// Key event (Linux evdev keycode) for the focused surface, plus the resulting modifier masks.
    pub fn key(&mut self, code: u32, pressed: bool, mods: (u32, u32, u32), time_ms: u32) {
        if self.key_focus.is_none() {
            return;
        }
        let serial = self.next_serial();
        for k in self.with(Iface::Keyboard) {
            self.event(k, 3, &words(&[serial, time_ms, code, pressed as u32]));
            self.event(k, 4, &words(&[serial, mods.0, mods.1, mods.2, 0]));
        }
    }

    fn pointer_frame(&mut self, p: u32) {
        if self.objects.get(&p).is_some_and(|o| o.version >= 5) {
            self.event(p, 5, &[]);
        }
    }

    fn with(&self, iface: Iface) -> Vec<u32> {
        self.objects.iter().filter(|(_, o)| o.iface == iface).map(|(&k, _)| k).collect()
    }

    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1).max(1);
        self.serial
    }

    fn new_id(&mut self, p: &[u8], at: usize, iface: Iface, parent: u32) -> Result<u32, Error> {
        let id = get_u32(p.get(at..).ok_or(Error::Malformed)?)?;
        let version = self.objects.get(&parent).map_or(1, |o| o.version);
        self.insert(id, iface, version)?;
        Ok(id)
    }

    fn destroy(&mut self, id: u32) {
        if let Some(o) = self.objects.remove(&id) {
            match o.iface {
                Iface::XdgToplevel | Iface::XdgPopup => {
                    if let Some(s) = self.surface_of_role(id) {
                        self.closed.push(s);
                    }
                }
                Iface::Surface => {
                    self.closed.push(id);
                    if self.pointer_focus == Some(id) {
                        self.pointer_focus = None;
                    }
                    if self.key_focus == Some(id) {
                        self.key_focus = None;
                    }
                }
                _ => {}
            }
        }
        self.event(1, 1, &words(&[id]));
    }

    /// wl_surface behind an xdg_toplevel/xdg_popup.
    fn surface_of_role(&self, role: u32) -> Option<u32> {
        let xdg = self.objects.iter().find(|(_, o)| o.iface == Iface::XdgSurface && o.surf.as_ref().is_some_and(|s| s.role_top == Some(role))).map(|(&k, _)| k)?;
        self.objects.get(&xdg)?.surface
    }

    fn done(&mut self, cb: u32, data: u32) {
        if self.objects.remove(&cb).is_some() {
            self.event(cb, 0, &words(&[data]));
            self.event(1, 1, &words(&[cb]));
        }
    }

    fn request(&mut self, object: u32, op: u16, p: &[u8]) -> Result<(), Error> {
        let iface = self.objects.get(&object).ok_or(Error::UnknownObject)?.iface;
        use Iface::*;
        match (iface, op) {
            (Display, 0) => {
                let cb = self.new_id(p, 0, Callback, object)?;
                let s = self.serial;
                self.done(cb, s);
            }
            (Display, 1) => {
                let reg = self.new_id(p, 0, Registry, object)?;
                for g in self.globals.clone() {
                    let mut b = words(&[g.name]);
                    put_string(&mut b, g.interface);
                    put_u32(&mut b, g.version);
                    self.event(reg, 0, &b);
                }
            }
            (Registry, 0) => self.bind(p)?,
            (Compositor, 0) => {
                let id = self.new_id(p, 0, Surface, object)?;
                self.objects.get_mut(&id).unwrap().surf = Some(self::Surface::default());
                if self.objects[&id].version >= 6 {
                    self.event(id, 2, &words(&[1]));
                }
            }
            (Compositor, 1) => {
                self.new_id(p, 0, Region, object)?;
            }
            (Region, 0) | (Subsurface, 0) | (Buffer, 0) | (XdgPositioner, 0) | (DataSource, 1) | (ToplevelDecoration, 0) | (Subcompositor, 0) | (DecorationManager, 0) | (XdgWmBase, 0) => self.destroy(object),
            (Pointer, 1) | (Keyboard, 0) | (Touch, 0) | (Seat, 3) | (Output, 0) | (DataDevice, 2) | (Shm, 1) | (DataDeviceManager, 2) => self.destroy(object),
            (Region, 1) | (Region, 2) => {}
            (Subcompositor, 1) => {
                self.new_id(p, 0, Subsurface, object)?;
            }
            (Subsurface, 1..=5) => {}
            (Shm, 0) => {
                let id = get_u32(p)?;
                let size = get_u32(p.get(4..).ok_or(Error::Malformed)?)? as usize;
                let fd = self.in_fds.pop_front().ok_or(Error::InvalidState)?;
                self.insert(id, ShmPool, 1)?;
                fd.buf.borrow_mut().grow_to(size);
                self.objects.get_mut(&id).unwrap().pool = Some(Pool { buf: fd.buf, size });
            }
            (ShmPool, 0) => {
                let pool = self.objects[&object].pool.clone().ok_or(Error::InvalidState)?;
                let v = |i: usize| get_u32(p.get(i * 4..).ok_or(Error::Malformed)?);
                let (offset, width, height, stride, format) = (v(1)? as usize, v(2)?, v(3)?, v(4)?, v(5)?);
                if format > 1 && format != 0x3432_4258 && format != 0x3432_5258 {
                    return Err(Error::Unsupported);
                }
                if width == 0 || height == 0 || stride < width * 4 || offset + stride as usize * height as usize > pool.size {
                    return Err(Error::InvalidState);
                }
                let id = self.new_id(p, 0, Buffer, object)?;
                self.objects.get_mut(&id).unwrap().buffer = Some(self::Buffer { pool, offset, width, height, stride, format });
            }
            (ShmPool, 1) => self.destroy(object),
            (ShmPool, 2) => {
                let size = get_u32(p)? as usize;
                let pool = self.objects.get_mut(&object).unwrap().pool.as_mut().ok_or(Error::InvalidState)?;
                pool.buf.borrow_mut().grow_to(size);
                pool.size = pool.size.max(size);
            }
            (Surface, 0) => self.destroy(object),
            (Surface, 1) => {
                let b = get_u32(p)?;
                self.surf(object).pending = Some(b);
            }
            (Surface, 3) => {
                let cb = self.new_id(p, 0, Callback, object)?;
                self.surf(object).pending_cbs.push(cb);
            }
            (Surface, 2) | (Surface, 4) | (Surface, 5) | (Surface, 7) | (Surface, 8) | (Surface, 9) | (Surface, 10) => {}
            (Surface, 6) => self.commit(object)?,
            (Seat, 0) => {
                self.new_id(p, 0, Pointer, object)?;
            }
            (Seat, 1) => {
                let k = self.new_id(p, 0, Keyboard, object)?;
                if self.keymap_size > 0 {
                    self.event(k, 0, &words(&[1, self.keymap_size]));
                    self.out_fds.push(KEYMAP_PATH.into());
                } else {
                    self.event(k, 0, &words(&[0, 0]));
                    self.out_fds.push("/dev/null".into());
                }
                if self.objects[&k].version >= 4 {
                    self.event(k, 5, &words(&[25, 400]));
                }
            }
            (Seat, 2) => {
                self.new_id(p, 0, Touch, object)?;
            }
            (Pointer, 0) => {}
            (XdgWmBase, 1) => {
                self.new_id(p, 0, XdgPositioner, object)?;
            }
            (XdgWmBase, 2) => {
                let id = self.new_id(p, 0, XdgSurface, object)?;
                let surface = get_u32(p.get(4..).ok_or(Error::Malformed)?)?;
                let o = self.objects.get_mut(&id).unwrap();
                o.surface = Some(surface);
                o.surf = Some(self::Surface::default());
                self.surf(surface).xdg = Some(id);
            }
            (XdgWmBase, 3) => {}
            (XdgPositioner, 1..=9) => {}
            (XdgSurface, 0) => self.destroy(object),
            (XdgSurface, 1) => {
                let top = self.new_id(p, 0, XdgToplevel, object)?;
                self.role(object, top);
                let v = self.objects[&top].version;
                if v >= 5 {
                    let mut b = Vec::new();
                    put_array(&mut b, &words(&[1, 2, 3, 4])); // window menu, maximize, fullscreen, minimize
                    self.event(top, 3, &b);
                }
                if v >= 4 {
                    self.event(top, 2, &words(&[self.screen.0, self.screen.1.saturating_sub(48)]));
                }
                self.configure_toplevel(object, top);
            }
            (XdgSurface, 2) => {
                let popup = self.new_id(p, 0, XdgPopup, object)?;
                self.role(object, popup);
                let mut b = words(&[0, 0, 200, 200]);
                b.truncate(16);
                self.event(popup, 0, &b);
                let s = self.next_serial();
                self.event(object, 0, &words(&[s]));
            }
            (XdgSurface, 3) => {
                let v = |i: usize| get_u32(p.get(i * 4..).ok_or(Error::Malformed)?);
                let g = (v(0)?, v(1)?, v(2)?, v(3)?);
                self.objects.get_mut(&object).unwrap().geometry = (g.2 > 0 && g.3 > 0).then_some(g);
            }
            (XdgSurface, 4) => {}
            (XdgToplevel, 0) | (XdgPopup, 0) => self.destroy(object),
            (XdgToplevel, 2) => {
                let (t, _) = get_string(p)?;
                self.objects.get_mut(&object).unwrap().title = t.into();
            }
            (XdgToplevel, 3) => {
                let (t, _) = get_string(p)?;
                self.objects.get_mut(&object).unwrap().app_id = t.into();
            }
            (XdgToplevel, 1) | (XdgToplevel, 4..=13) => {}
            (XdgToplevel, 14) => {}
            (XdgPopup, 1) => {}
            (XdgPopup, 2) => {
                let token = get_u32(p.get(4..).ok_or(Error::Malformed)?)?;
                self.event(object, 2, &words(&[token]));
            }
            (DataDeviceManager, 0) => {
                self.new_id(p, 0, DataSource, object)?;
            }
            (DataDeviceManager, 1) => {
                self.new_id(p, 0, DataDevice, object)?;
            }
            (DataSource, 0) | (DataSource, 2) | (DataDevice, 0) | (DataDevice, 1) => {}
            (DecorationManager, 1) => {
                let d = self.new_id(p, 0, ToplevelDecoration, object)?;
                self.event(d, 0, &words(&[1])); // client-side: toolkits draw their own titlebar
            }
            (ToplevelDecoration, 1) | (ToplevelDecoration, 2) => {}
            _ => return Err(Error::Unsupported),
        }
        Ok(())
    }

    fn role(&mut self, xdg: u32, role: u32) {
        self.objects.get_mut(&role).unwrap().surface = Some(xdg);
        if let Some(s) = self.objects.get_mut(&xdg).and_then(|o| o.surf.as_mut()) {
            s.role_top = Some(role);
        }
        if let Some(ws) = self.objects.get(&xdg).and_then(|o| o.surface) {
            if let Some(s) = self.objects.get_mut(&ws).and_then(|o| o.surf.as_mut()) {
                s.role_top = Some(role);
            }
        }
    }

    fn configure_toplevel(&mut self, xdg: u32, top: u32) {
        let mut b = words(&[0, 0]); // 0x0: the client picks its size
        put_array(&mut b, &words(&[4])); // activated
        self.event(top, 0, &b);
        let s = self.next_serial();
        self.event(xdg, 0, &words(&[s]));
    }

    fn surf(&mut self, id: u32) -> &mut Surface {
        self.objects.get_mut(&id).and_then(|o| o.surf.as_mut()).expect("surface object")
    }

    fn commit(&mut self, id: u32) -> Result<(), Error> {
        let s = self.surf(id);
        let cbs = core::mem::take(&mut s.pending_cbs);
        s.frame_cbs.extend(cbs);
        let Some(b) = s.pending.take() else { return Ok(()) };
        s.attached = if b == 0 { None } else { Some(b) };
        let (xdg, top) = (s.xdg, s.role_top);
        let Some(bid) = s.attached else {
            self.closed.push(id);
            return Ok(());
        };
        let Some(buffer) = self.objects.get(&bid).and_then(|o| o.buffer.clone()) else { return Err(Error::InvalidState) };
        let mut pixels = decode_pixels(&buffer);
        let (mut width, mut height) = (buffer.width, buffer.height);
        if let Some((gx, gy, gw, gh)) = xdg.and_then(|x| self.objects.get(&x)).and_then(|o| o.geometry) {
            // Show only the window geometry: drop the client's shadow margin.
            let (gw, gh) = (gw.min(width.saturating_sub(gx)), gh.min(height.saturating_sub(gy)));
            if gw > 0 && gh > 0 {
                let mut out = Vec::with_capacity((gw * gh) as usize);
                for y in gy..gy + gh {
                    let row = (y * width + gx) as usize;
                    out.extend_from_slice(&pixels[row..row + gw as usize]);
                }
                pixels = out;
                (width, height) = (gw, gh);
            }
        }
        let (title, app_id, toplevel) = match top.and_then(|t| self.objects.get(&t)) {
            Some(o) => (o.title.as_bytes().to_vec(), o.app_id.as_bytes().to_vec(), o.iface == Iface::XdgToplevel),
            None => (Vec::new(), Vec::new(), xdg.is_none()),
        };
        if xdg.is_some() || top.is_some() {
            self.frames.push(Frame { surface: id, width, height, pixels, title, app_id, toplevel });
        }
        self.event(bid, 0, &[]); // pixels copied: the client may reuse the buffer at once
        Ok(())
    }

    fn bind(&mut self, p: &[u8]) -> Result<(), Error> {
        let name = get_u32(p)?;
        let (interface, used) = get_string(&p[4..])?;
        let version = get_u32(p.get(4 + used..).ok_or(Error::Malformed)?)?;
        let id = get_u32(p.get(8 + used..).ok_or(Error::Malformed)?)?;
        let g = self.globals.iter().find(|g| g.name == name && g.interface == interface).copied().ok_or(Error::Unsupported)?;
        if version == 0 || version > g.version {
            return Err(Error::Unsupported);
        }
        let iface = GLOBALS.iter().find(|x| x.0 == interface).unwrap().2;
        self.insert(id, iface, version)?;
        match iface {
            Iface::Shm => {
                self.event(id, 0, &words(&[0]));
                self.event(id, 0, &words(&[1]));
            }
            Iface::Seat => {
                self.event(id, 0, &words(&[3]));
                if version >= 2 {
                    let mut b = Vec::new();
                    put_string(&mut b, "seat0");
                    self.event(id, 1, &b);
                }
            }
            Iface::Output => {
                let mut g = words(&[0, 0, 340, 270, 0]);
                put_string(&mut g, "LoOS");
                put_string(&mut g, "Display");
                put_u32(&mut g, 0);
                self.event(id, 0, &g);
                self.event(id, 1, &words(&[3, self.screen.0, self.screen.1, 60000]));
                if version >= 2 {
                    self.event(id, 3, &words(&[1]));
                }
                if version >= 4 {
                    let mut b = Vec::new();
                    put_string(&mut b, "LOOS-1");
                    self.event(id, 4, &b);
                }
                if version >= 2 {
                    self.event(id, 2, &[]);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn insert(&mut self, id: u32, iface: Iface, version: u32) -> Result<(), Error> {
        if id == 0 || self.objects.contains_key(&id) {
            return Err(Error::InvalidState);
        }
        self.objects.insert(id, new_obj(iface, version));
        Ok(())
    }

    fn event(&mut self, object: u32, opcode: u16, body: &[u8]) {
        put_u32(&mut self.events, object);
        put_u32(&mut self.events, (((body.len() + 8) as u32) << 16) | opcode as u32);
        self.events.extend_from_slice(body);
    }
}

fn new_obj(iface: Iface, version: u32) -> Obj {
    Obj { iface, version, surface: None, pool: None, buffer: None, surf: None, title: String::new(), app_id: String::new(), geometry: None }
}

/// Copy a wl_shm buffer out of its (live) pool as `0xAARRGGBB`.
fn decode_pixels(b: &Buffer) -> Vec<u32> {
    let mut out = Vec::with_capacity((b.width * b.height) as usize);
    let mut row = alloc::vec![0u8; b.width as usize * 4];
    let pool = b.pool.buf.borrow();
    for y in 0..b.height as usize {
        pool.read_at(b.offset + y * b.stride as usize, &mut row);
        for px in row.chunks_exact(4) {
            let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
            out.push(if b.format == 1 || b.format == 0x3432_5258 { v | 0xff00_0000 } else { v });
        }
    }
    out
}

fn words(v: &[u32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 4);
    for &x in v {
        put_u32(&mut b, x);
    }
    b
}

fn get_u32(b: &[u8]) -> Result<u32, Error> {
    let bytes: [u8; 4] = b.get(..4).ok_or(Error::Malformed)?.try_into().map_err(|_| Error::Malformed)?;
    Ok(u32::from_ne_bytes(bytes))
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_ne_bytes());
}

fn put_array(out: &mut Vec<u8>, data: &[u8]) {
    put_u32(out, data.len() as u32);
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn put_string(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32 + 1);
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn get_string(b: &[u8]) -> Result<(&str, usize), Error> {
    let len = get_u32(b)? as usize;
    if len == 0 {
        return Ok(("", 4));
    }
    let used = 4 + len.div_ceil(4) * 4;
    let raw = b.get(4..4 + len - 1).ok_or(Error::Malformed)?;
    if b.len() < used {
        return Err(Error::Malformed);
    }
    Ok((core::str::from_utf8(raw).map_err(|_| Error::Malformed)?, used))
}
