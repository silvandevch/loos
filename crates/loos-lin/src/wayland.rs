//! Minimal Wayland wire protocol core for the Lin compositor.
//!
//! This module is transport-independent: the kernel/socket adapter feeds
//! complete client bytes to [`Server::receive`] and drains encoded events with
//! [`Server::take_events`]. It implements the object lifecycle needed by
//! wl_shm/xdg-shell clients (registry, compositor, shm, surface, xdg surface,
//! and toplevel). Unix socket ancillary-fd transport and shared-buffer import
//! are deliberately separate integration work.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Protocol parsing or object-state failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The request header or payload is truncated/malformed.
    Malformed,
    /// The request references an unknown object.
    UnknownObject,
    /// The request is valid Wayland but outside this core's supported slice.
    Unsupported,
    /// The request violates object lifecycle rules.
    InvalidState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Display,
    Registry,
    Compositor,
    Shm,
    Surface,
    XdgWmBase,
    XdgSurface,
    XdgToplevel,
    Buffer,
    Callback,
    Seat,
    Pointer,
    Keyboard,
    Touch,
    Output,
}

#[derive(Debug)]
struct Object {
    kind: Kind,
    configured: bool,
    attached: Option<u32>,
    pool: Option<Pool>,
    buffer: Option<Buffer>,
    surface: Option<u32>,
    title: Vec<u8>,
}

#[derive(Debug)]
struct Pool {
    data: Vec<u8>,
    size: u32,
}

#[derive(Clone, Debug)]
struct Buffer {
    pool: u32,
    offset: u32,
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
}

/// Bytes received through `SCM_RIGHTS` for a Wayland shared-memory fd.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedFd {
    /// A snapshot of the readable file bytes at message processing time.
    pub data: Vec<u8>,
}

/// A committed wl_shm frame ready for compositor presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Wayland surface object id.
    pub surface: u32,
    /// Buffer width in pixels.
    pub width: u32,
    /// Buffer height in pixels.
    pub height: u32,
    /// Pixels in LoOS `0xAARRGGBB` format.
    pub pixels: Vec<u32>,
    /// Application title from `xdg_toplevel.set_title`, if supplied.
    pub title: Vec<u8>,
}

/// A supported global advertised by the registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Global {
    /// Registry numeric name.
    pub name: u32,
    /// Interface name.
    pub interface: &'static str,
    /// Maximum supported protocol version.
    pub version: u32,
}

/// A transport-independent Wayland client connection.
pub struct Server {
    objects: BTreeMap<u32, Object>,
    globals: [Global; 5],
    events: Vec<u8>,
    frames: Vec<Frame>,
    serial: u32,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// Creates a connection with the core globals advertised by LoOS.
    pub fn new() -> Self {
        let mut objects = BTreeMap::new();
        objects.insert(1, empty_object(Kind::Display));
        Self {
            objects,
            globals: [
                Global { name: 1, interface: "wl_compositor", version: 4 },
                Global { name: 2, interface: "wl_shm", version: 1 },
                Global { name: 3, interface: "xdg_wm_base", version: 6 },
                Global { name: 4, interface: "wl_seat", version: 7 },
                Global { name: 5, interface: "wl_output", version: 3 },
            ],
            events: Vec::new(),
            frames: Vec::new(),
            serial: 1,
        }
    }

    /// Returns the globals that `wl_registry.global` advertises.
    pub fn globals(&self) -> &[Global] {
        &self.globals
    }

    /// Feeds one or more complete Wayland requests to the server.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.receive_with_fds(bytes, &[])
    }

    /// Feeds requests plus the file descriptors carried by the same `sendmsg`.
    pub fn receive_with_fds(&mut self, bytes: &[u8], fds: &[SharedFd]) -> Result<(), Error> {
        let mut off = 0;
        while off < bytes.len() {
            if bytes.len() - off < 8 {
                return Err(Error::Malformed);
            }
            let object = get_u32(&bytes[off..off + 4])?;
            let word = get_u32(&bytes[off + 4..off + 8])?;
            let size = (word >> 16) as usize;
            let opcode = (word & 0xffff) as u16;
            if size < 8 || size % 4 != 0 || size > bytes.len() - off {
                return Err(Error::Malformed);
            }
            self.request(object, opcode, &bytes[off + 8..off + size], fds)?;
            off += size;
        }
        Ok(())
    }

    /// Takes and clears encoded server events.
    pub fn take_events(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.events)
    }

    /// Takes committed shared-memory frames for presentation.
    pub fn take_frames(&mut self) -> Vec<Frame> {
        core::mem::take(&mut self.frames)
    }

    fn request(&mut self, object: u32, opcode: u16, p: &[u8], fds: &[SharedFd]) -> Result<(), Error> {
        let kind = self.objects.get(&object).ok_or(Error::UnknownObject)?.kind;
        match (kind, opcode) {
            (Kind::Display, 1) => {
                let registry = get_u32(p)?;
                self.insert(registry, Kind::Registry)?;
                for g in self.globals {
                    let mut body = Vec::new();
                    put_u32(&mut body, g.name);
                    put_string(&mut body, g.interface);
                    put_u32(&mut body, g.version);
                    self.event(registry, 0, &body);
                }
                Ok(())
            }
            (Kind::Display, 0) => {
                let callback = get_u32(p)?;
                self.insert(callback, Kind::Callback)?;
                let mut body = Vec::new();
                put_u32(&mut body, self.serial);
                self.event(callback, 0, &body);
                Ok(())
            }
            (Kind::Registry, 0) => self.bind(p),
            (Kind::Compositor, 0) => {
                let surface = get_u32(p)?;
                self.insert(surface, Kind::Surface)
            }
            (Kind::Shm, 0) => {
                if self.objects.get(&object).and_then(|o| o.pool.as_ref()).is_some() {
                    let new_id = get_u32(p)?;
                    let offset = get_u32(p.get(4..).ok_or(Error::Malformed)?)?;
                    let width = get_u32(p.get(8..).ok_or(Error::Malformed)?)?;
                    let height = get_u32(p.get(12..).ok_or(Error::Malformed)?)?;
                    let stride = get_u32(p.get(16..).ok_or(Error::Malformed)?)?;
                    let format = get_u32(p.get(20..).ok_or(Error::Malformed)?)?;
                    self.insert(new_id, Kind::Buffer)?;
                    self.objects.get_mut(&new_id).unwrap().buffer = Some(Buffer { pool: object, offset, width, height, stride, format });
                    return Ok(());
                }
                let pool = get_u32(p)?;
                let size = get_u32(p.get(4..).ok_or(Error::Malformed)?)?;
                let fd = fds.first().ok_or(Error::InvalidState)?;
                if size as usize > fd.data.len() {
                    return Err(Error::InvalidState);
                }
                self.insert(pool, Kind::Shm)?;
                self.objects.get_mut(&pool).unwrap().pool = Some(Pool { data: fd.data.clone(), size });
                Ok(())
            }
            (Kind::Shm, 1) | (Kind::Shm, 2) => Ok(()),
            (Kind::Surface, 1) => {
                let buffer = get_u32(p)?;
                let s = self.objects.get_mut(&object).ok_or(Error::UnknownObject)?;
                s.attached = if buffer == u32::MAX { None } else { Some(buffer) };
                Ok(())
            }
            (Kind::Surface, 3) => {
                let callback = get_u32(p)?;
                self.insert(callback, Kind::Callback)?;
                let mut body = Vec::new();
                put_u32(&mut body, self.serial);
                self.event(callback, 0, &body);
                Ok(())
            }
            (Kind::Surface, 6) => {
                let buffer_id = self.objects.get(&object).and_then(|s| s.attached).ok_or(Error::InvalidState)?;
                let buffer = self.objects.get(&buffer_id).and_then(|b| b.buffer.clone()).ok_or(Error::InvalidState)?;
                let pool = self.objects.get(&buffer.pool).and_then(|p| p.pool.as_ref()).ok_or(Error::InvalidState)?;
                let pixels = decode_pixels(pool, &buffer)?;
                let xdg_id = self.objects.iter().find(|(_, o)| o.kind == Kind::XdgSurface && o.surface == Some(object)).map(|(&id, _)| id);
                let title = xdg_id.and_then(|id| self.objects.values().find(|o| o.kind == Kind::XdgToplevel && o.surface == Some(id))).map(|o| o.title.clone()).unwrap_or_default();
                self.frames.push(Frame { surface: object, width: buffer.width, height: buffer.height, pixels, title });
                self.event(buffer_id, 0, &[]);
                Ok(())
            }
            (Kind::Surface, 8) | (Kind::Surface, 9) => Ok(()),
            (Kind::XdgWmBase, 2) => {
                let surface = get_u32(p)?;
                let xdg_surface = get_u32(p.get(4..).ok_or(Error::Malformed)?)?;
                self.insert(xdg_surface, Kind::XdgSurface)?;
                self.objects.get_mut(&xdg_surface).unwrap().surface = Some(surface);
                self.serial = self.serial.wrapping_add(1).max(1);
                let mut body = Vec::new();
                put_u32(&mut body, self.serial);
                self.event(xdg_surface, 0, &body);
                Ok(())
            }
            (Kind::XdgWmBase, 3) => {
                let serial = get_u32(p)?;
                let mut body = Vec::new();
                put_u32(&mut body, serial);
                self.event(object, 0, &body);
                Ok(())
            }
            (Kind::XdgSurface, 1) => {
                let top = get_u32(p)?;
                self.insert(top, Kind::XdgToplevel)?;
                self.objects.get_mut(&top).unwrap().surface = Some(object);
                self.event(top, 0, &[]);
                self.event(top, 1, &[]);
                let mut body = Vec::new();
                put_u32(&mut body, self.serial);
                self.event(object, 0, &body);
                Ok(())
            }
            (Kind::XdgSurface, 4) => {
                let _serial = get_u32(p)?;
                let s = self.objects.get_mut(&object).ok_or(Error::UnknownObject)?;
                s.configured = true;
                Ok(())
            }
            (Kind::XdgSurface, 0) | (Kind::XdgToplevel, 2) => {
                if kind == Kind::XdgToplevel && opcode == 2 {
                    let (text, _) = get_string(p)?;
                    let top = self.objects.get_mut(&object).ok_or(Error::UnknownObject)?;
                    top.title = text.as_bytes().to_vec();
                }
                Ok(())
            }
            (Kind::XdgToplevel, 3)
            | (Kind::XdgToplevel, 5) | (Kind::XdgToplevel, 6) => Ok(()),
            (Kind::Surface, 0) | (Kind::XdgWmBase, 0) => Ok(()),
            (Kind::Seat, 0) => {
                let id = get_u32(p)?;
                self.insert(id, Kind::Pointer)
            },
            (Kind::Seat, 1) => {
                let id = get_u32(p)?;
                self.insert(id, Kind::Keyboard)
            },
            (Kind::Seat, 2) => {
                let id = get_u32(p)?;
                self.insert(id, Kind::Touch)
            },
            (Kind::Seat, 3)
            | (Kind::Pointer, 0) | (Kind::Pointer, 1) | (Kind::Pointer, 2)
            | (Kind::Keyboard, 0) | (Kind::Keyboard, 1) | (Kind::Keyboard, 2)
            | (Kind::Touch, 0) | (Kind::Touch, 1) | (Kind::Output, 0) => Ok(()),
            (Kind::Output, 1) | (Kind::Output, 2) | (Kind::Output, 3) => Ok(()),
            (Kind::Buffer, 0) | (Kind::Callback, 0) => Ok(()),
            _ => Err(Error::Unsupported),
        }
    }

    fn bind(&mut self, p: &[u8]) -> Result<(), Error> {
        let name = get_u32(p)?;
        let (interface, used) = get_string(&p[4..])?;
        let version = get_u32(p.get(4 + used..).ok_or(Error::Malformed)?)?;
        let new_id = get_u32(p.get(8 + used..).ok_or(Error::Malformed)?)?;
        let global = self.globals.iter().find(|g| g.name == name && g.interface == interface).ok_or(Error::Unsupported)?;
        if version == 0 || version > global.version {
            return Err(Error::Unsupported);
        }
        let kind = match interface {
            "wl_compositor" => Kind::Compositor,
            "wl_shm" => Kind::Shm,
            "xdg_wm_base" => Kind::XdgWmBase,
            "wl_seat" => {
                self.insert(new_id, Kind::Seat)?;
                let mut caps = Vec::new();
                put_u32(&mut caps, 3);
                self.event(new_id, 0, &caps);
                put_string_event(&mut self.events, new_id, 1, "LoOS seat");
                return Ok(())
            }
            "wl_output" => {
                self.insert(new_id, Kind::Output)?;
                let mut geometry = Vec::new();
                put_u32(&mut geometry, 0);
                put_u32(&mut geometry, 0);
                put_u32(&mut geometry, 0);
                put_u32(&mut geometry, 0);
                put_string(&mut geometry, "LoOS");
                put_string(&mut geometry, "LoOS");
                put_u32(&mut geometry, 0);
                self.event(new_id, 0, &geometry);
                let mut mode = Vec::new();
                put_u32(&mut mode, 1);
                put_u32(&mut mode, 1024);
                put_u32(&mut mode, 768);
                put_u32(&mut mode, 60000);
                self.event(new_id, 1, &mode);
                self.event(new_id, 2, &[]);
                let mut scale = Vec::new();
                put_u32(&mut scale, 1);
                self.event(new_id, 3, &scale);
                return Ok(())
            }
            _ => return Err(Error::Unsupported),
        };
        self.insert(new_id, kind)?;
        if interface == "wl_shm" {
            let mut fmt = Vec::new();
            put_u32(&mut fmt, 1);
            self.event(new_id, 0, &fmt);
            fmt.clear();
            put_u32(&mut fmt, 0);
            self.event(new_id, 0, &fmt);
        }
        Ok(())
    }

    fn insert(&mut self, id: u32, kind: Kind) -> Result<(), Error> {
        if id == 0 || self.objects.contains_key(&id) {
            return Err(Error::InvalidState);
        }
        self.objects.insert(id, empty_object(kind));
        Ok(())
    }

    fn event(&mut self, object: u32, opcode: u16, body: &[u8]) {
        put_u32(&mut self.events, object);
        put_u32(&mut self.events, (((body.len() + 8) as u32) << 16) | opcode as u32);
        self.events.extend_from_slice(body);
    }
}

fn empty_object(kind: Kind) -> Object {
    Object { kind, configured: false, attached: None, pool: None, buffer: None, surface: None, title: Vec::new() }
}

fn decode_pixels(pool: &Pool, buffer: &Buffer) -> Result<Vec<u32>, Error> {
    if buffer.format > 1 || buffer.width == 0 || buffer.height == 0 || buffer.stride < buffer.width.saturating_mul(4) {
        return Err(Error::InvalidState);
    }
    let end = (buffer.offset as usize)
        .checked_add(buffer.stride as usize * buffer.height as usize)
        .ok_or(Error::InvalidState)?;
    if end > pool.size as usize || end > pool.data.len() {
        return Err(Error::InvalidState);
    }
    let mut out = Vec::with_capacity(buffer.width as usize * buffer.height as usize);
    for y in 0..buffer.height as usize {
        let row = buffer.offset as usize + y * buffer.stride as usize;
        for x in 0..buffer.width as usize {
            let p = row + x * 4;
            let b = pool.data[p] as u32;
            let g = pool.data[p + 1] as u32;
            let r = pool.data[p + 2] as u32;
            let a = if buffer.format == 0 { pool.data[p + 3] as u32 } else { 0xff };
            out.push((a << 24) | (r << 16) | (g << 8) | b);
        }
    }
    Ok(out)
}

fn get_u32(b: &[u8]) -> Result<u32, Error> {
    let a = b.get(..4).ok_or(Error::Malformed)?;
    Ok(u32::from_ne_bytes([a[0], a[1], a[2], a[3]]))
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_ne_bytes());
}

fn put_string(out: &mut Vec<u8>, s: &str) {
    let n = s.len() + 1;
    put_u32(out, n as u32);
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn put_string_event(events: &mut Vec<u8>, object: u32, opcode: u16, s: &str) {
    let mut body = Vec::new();
    put_string(&mut body, s);
    put_u32(events, object);
    put_u32(events, (((body.len() + 8) as u32) << 16) | opcode as u32);
    events.extend_from_slice(&body);
}

fn get_string(b: &[u8]) -> Result<(&str, usize), Error> {
    let n = get_u32(b)? as usize;
    if n == 0 || n > b.len().saturating_sub(4) {
        return Err(Error::Malformed);
    }
    let start = 4;
    let end = start + n;
    let raw = b.get(start..end).ok_or(Error::Malformed)?;
    if raw[n - 1] != 0 {
        return Err(Error::Malformed);
    }
    let s = core::str::from_utf8(&raw[..n - 1]).map_err(|_| Error::Malformed)?;
    Ok((s, (n + 3) & !3))
}
