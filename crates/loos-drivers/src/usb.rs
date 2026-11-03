//! USB protocol pieces independent of the host controller: descriptors, HID boot reports, bulk-only mass storage.

use crate::input::{keys, mods, InputEvent, KeyEvent, MouseEvent};
use alloc::vec::Vec;

/// Standard request codes.
pub const REQ_GET_DESCRIPTOR: u8 = 6;
/// SET_CONFIGURATION.
pub const REQ_SET_CONFIGURATION: u8 = 9;

/// Builds an 8-byte SETUP packet.
pub fn setup(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> [u8; 8] {
    let mut s = [0u8; 8];
    s[0] = request_type;
    s[1] = request;
    s[2..4].copy_from_slice(&value.to_le_bytes());
    s[4..6].copy_from_slice(&index.to_le_bytes());
    s[6..8].copy_from_slice(&length.to_le_bytes());
    s
}

/// Endpoint transfer type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpKind {
    /// Control.
    Control,
    /// Isochronous.
    Iso,
    /// Bulk.
    Bulk,
    /// Interrupt.
    Interrupt,
}

/// One endpoint descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    /// Endpoint number (1..15).
    pub num: u8,
    /// True for IN (device to host).
    pub dir_in: bool,
    /// Transfer type.
    pub kind: EpKind,
    /// Max packet size.
    pub max_packet: u16,
    /// Polling interval.
    pub interval: u8,
}

impl Endpoint {
    /// xHCI device context index.
    pub fn dci(&self) -> u8 {
        self.num * 2 + u8::from(self.dir_in)
    }
}

/// One interface with its endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    /// Interface number.
    pub number: u8,
    /// bInterfaceClass.
    pub class: u8,
    /// bInterfaceSubClass.
    pub subclass: u8,
    /// bInterfaceProtocol.
    pub protocol: u8,
    /// Endpoints (excluding EP0).
    pub endpoints: Vec<Endpoint>,
}

/// Parsed configuration descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// bConfigurationValue.
    pub value: u8,
    /// Interfaces (alternate setting 0 only).
    pub interfaces: Vec<Interface>,
}

/// Walks a full configuration descriptor blob.
pub fn parse_config(b: &[u8]) -> Option<Config> {
    if b.len() < 9 || b[1] != 2 {
        return None;
    }
    let mut cfg = Config { value: b[5], interfaces: Vec::new() };
    let mut i = usize::from(b[0]);
    while i + 2 <= b.len() {
        let len = usize::from(b[i]);
        if len < 2 || i + len > b.len() {
            return None;
        }
        match b[i + 1] {
            4 if len >= 9 => {
                if b[i + 3] == 0 {
                    cfg.interfaces.push(Interface { number: b[i + 2], class: b[i + 5], subclass: b[i + 6], protocol: b[i + 7], endpoints: Vec::new() });
                } else {
                    // alternate setting: skip its endpoints
                    cfg.interfaces.push(Interface { number: 0xff, class: 0xff, subclass: 0, protocol: 0, endpoints: Vec::new() });
                }
            }
            5 if len >= 7 => {
                let a = b[i + 2];
                let kind = match b[i + 3] & 3 {
                    0 => EpKind::Control,
                    1 => EpKind::Iso,
                    2 => EpKind::Bulk,
                    _ => EpKind::Interrupt,
                };
                if let Some(itf) = cfg.interfaces.last_mut() {
                    itf.endpoints.push(Endpoint { num: a & 15, dir_in: a & 0x80 != 0, kind, max_packet: u16::from_le_bytes([b[i + 4], b[i + 5]]) & 0x7ff, interval: b[i + 6] });
                }
            }
            _ => {}
        }
        i += len;
    }
    cfg.interfaces.retain(|i| i.number != 0xff);
    Some(cfg)
}

/// Usage-ID to (unshifted, shifted) character for the boot keyboard page.
fn usage_chars(u: u8) -> Option<(u32, u32)> {
    let c = |a: char, b: char| Some((a as u32, b as u32));
    match u {
        4..=29 => {
            let l = (b'a' + u - 4) as char;
            c(l, l.to_ascii_uppercase())
        }
        30..=38 => {
            let d = (b'1' + u - 30) as char;
            c(d, b")!@#$%^&*("[usize::from(u) - 29] as char)
        }
        39 => c('0', ')'),
        40 => c('\n', '\n'),
        41 => c('\x1b', '\x1b'),
        42 => c('\x08', '\x08'),
        43 => c('\t', '\t'),
        44 => c(' ', ' '),
        45 => c('-', '_'),
        46 => c('=', '+'),
        47 => c('[', '{'),
        48 => c(']', '}'),
        49 => c('\\', '|'),
        51 => c(';', ':'),
        52 => c('\'', '"'),
        53 => c('`', '~'),
        54 => c(',', '<'),
        55 => c('.', '>'),
        56 => c('/', '?'),
        _ => None,
    }
}

fn usage_special(u: u8) -> Option<u32> {
    Some(match u {
        58..=69 => keys::F1 + u32::from(u - 58),
        74 => keys::HOME,
        75 => keys::PGUP,
        76 => keys::DELETE,
        77 => keys::END,
        78 => keys::PGDN,
        79 => keys::RIGHT,
        80 => keys::LEFT,
        81 => keys::DOWN,
        82 => keys::UP,
        _ => return None,
    })
}

/// Boot-protocol keyboard report decoder (diffs successive 8-byte reports into press/release events).
#[derive(Debug, Default)]
pub struct KeyboardDecoder {
    prev: [u8; 6],
    prev_mods: u8,
}

impl KeyboardDecoder {
    /// Decodes one report.
    pub fn decode(&mut self, r: &[u8]) -> Vec<InputEvent> {
        let mut out = Vec::new();
        if r.len() < 8 {
            return out;
        }
        let hm = r[0];
        let mut m = 0u8;
        if hm & 0x11 != 0 {
            m |= mods::CTRL;
        }
        if hm & 0x22 != 0 {
            m |= mods::SHIFT;
        }
        if hm & 0x44 != 0 {
            m |= mods::ALT;
        }
        if hm & 0x88 != 0 {
            m |= mods::SUPER;
        }
        if m & mods::SUPER != 0 && self.prev_mods & mods::SUPER == 0 {
            out.push(InputEvent::Key(KeyEvent { key: keys::SUPER, ch: 0, down: true, mods: m }));
        } else if m & mods::SUPER == 0 && self.prev_mods & mods::SUPER != 0 {
            out.push(InputEvent::Key(KeyEvent { key: keys::SUPER, ch: 0, down: false, mods: m }));
        }
        let cur: [u8; 6] = r[2..8].try_into().unwrap();
        let event = |u: u8, down: bool| {
            if let Some((lo, hi)) = usage_chars(u) {
                let ch = if m & mods::SHIFT != 0 { hi } else { lo };
                let ch = if m & mods::CTRL != 0 && ch.is_ascii_alphabetic_u32() { ch & 0x1f } else { ch };
                Some(InputEvent::Key(KeyEvent { key: lo, ch: if down { ch } else { 0 }, down, mods: m }))
            } else {
                usage_special(u).map(|k| InputEvent::Key(KeyEvent { key: k, ch: 0, down, mods: m }))
            }
        };
        for &u in self.prev.iter().filter(|&&u| u > 3 && !cur.contains(&u)) {
            out.extend(event(u, false));
        }
        for &u in cur.iter().filter(|&&u| u > 3 && !self.prev.contains(&u)) {
            out.extend(event(u, true));
        }
        self.prev = cur;
        self.prev_mods = m;
        out
    }
}

trait AsciiAlpha {
    fn is_ascii_alphabetic_u32(&self) -> bool;
}
impl AsciiAlpha for u32 {
    fn is_ascii_alphabetic_u32(&self) -> bool {
        char::from_u32(*self).is_some_and(|c| c.is_ascii_alphabetic())
    }
}

/// Decodes a boot-protocol mouse report: `[buttons, dx, dy, wheel?]`.
pub fn decode_mouse(r: &[u8]) -> Option<InputEvent> {
    if r.len() < 3 {
        return None;
    }
    Some(InputEvent::Mouse(MouseEvent { dx: i32::from(r[1] as i8), dy: i32::from(r[2] as i8), wheel: r.get(3).map_or(0, |&w| i32::from(w as i8)), buttons: r[0] & 0x1f }))
}

/// Bulk-only transport command block wrapper (31 bytes).
pub fn cbw(tag: u32, data_len: u32, dir_in: bool, cb: &[u8]) -> [u8; 31] {
    let mut w = [0u8; 31];
    w[0..4].copy_from_slice(&0x4342_5355u32.to_le_bytes());
    w[4..8].copy_from_slice(&tag.to_le_bytes());
    w[8..12].copy_from_slice(&data_len.to_le_bytes());
    w[12] = if dir_in { 0x80 } else { 0 };
    w[14] = cb.len() as u8;
    w[15..15 + cb.len()].copy_from_slice(cb);
    w
}

/// Validates a command status wrapper against `tag`; returns the SCSI status byte.
pub fn csw(b: &[u8], tag: u32) -> Option<u8> {
    (b.len() >= 13 && b[0..4] == 0x5342_5355u32.to_le_bytes() && b[4..8] == tag.to_le_bytes()).then(|| b[12])
}

/// SCSI READ(10) / WRITE(10) command block.
pub fn rw10(write: bool, lba: u32, blocks: u16) -> [u8; 10] {
    let mut c = [0u8; 10];
    c[0] = if write { 0x2a } else { 0x28 };
    c[2..6].copy_from_slice(&lba.to_be_bytes());
    c[7..9].copy_from_slice(&blocks.to_be_bytes());
    c
}
