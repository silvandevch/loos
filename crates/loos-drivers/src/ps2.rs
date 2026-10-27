//! i8042 PS/2 controller, keyboard (scancode set 1 as translated by the controller) and mouse
//! (standard 3-byte and IntelliMouse 4-byte protocols).

use crate::hw::PortIo;
use crate::input::{keys, mods, InputEvent, KeyEvent, MouseEvent};
use crate::DriverError;

const DATA: u16 = 0x60;
const CMD: u16 = 0x64;
const SPIN: u32 = 100_000;

/// Unshifted ASCII for set-1 make codes 0x00..=0x39.
const PLAIN: &[u8; 0x3a] = b"\0\x1b1234567890-=\x08\tqwertyuiop[]\r\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
/// Shifted ASCII for the same codes.
const SHIFTED: &[u8; 0x3a] = b"\0\x1b!@#$%^&*()_+\x08\tQWERTYUIOP{}\r\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";

/// Scancode-stream decoder: bytes in, [`KeyEvent`]s out.
#[derive(Debug, Default)]
pub struct KeyboardDecoder {
    ext: bool,
    skip: u8,
    shift: bool,
    ctrl: bool,
    alt: bool,
    sup: bool,
    caps: bool,
}

impl KeyboardDecoder {
    /// New decoder with no modifiers held.
    pub fn new() -> Self {
        Self::default()
    }

    fn mods(&self) -> u8 {
        (if self.shift { mods::SHIFT } else { 0 })
            | (if self.ctrl { mods::CTRL } else { 0 })
            | (if self.alt { mods::ALT } else { 0 })
            | (if self.sup { mods::SUPER } else { 0 })
    }

    /// Feed one scancode byte; returns an event when the byte completes a key transition that
    /// produces one. Pure modifier keys update state and return `None`.
    pub fn feed(&mut self, b: u8) -> Option<KeyEvent> {
        if self.skip > 0 {
            self.skip -= 1; // remainder of the Pause/Break sequence
            return None;
        }
        if b == 0xe1 {
            self.skip = 5;
            return None;
        }
        if b == 0xe0 {
            self.ext = true;
            return None;
        }
        if b == 0xfa || b == 0xfe || b == 0x00 || b == 0xff {
            self.ext = false; // ACK/resend/overrun noise
            return None;
        }
        let down = b & 0x80 == 0;
        let code = b & 0x7f;
        let e = core::mem::take(&mut self.ext);
        match (e, code) {
            (false, 0x2a) | (false, 0x36) => {
                self.shift = down;
                return None;
            }
            (_, 0x1d) => {
                self.ctrl = down;
                return None;
            }
            (_, 0x38) => {
                self.alt = down;
                return None;
            }
            (false, 0x3a) => {
                if down {
                    self.caps = !self.caps;
                }
                return None;
            }
            _ => {}
        }
        let (key, ch) = if e {
            let k = match code {
                0x48 => keys::UP,
                0x50 => keys::DOWN,
                0x4b => keys::LEFT,
                0x4d => keys::RIGHT,
                0x47 => keys::HOME,
                0x4f => keys::END,
                0x49 => keys::PGUP,
                0x51 => keys::PGDN,
                0x53 => keys::DELETE,
                0x5b | 0x5c => {
                    self.sup = down;
                    keys::SUPER
                }
                0x1c => 13, // keypad enter
                _ => return None,
            };
            (k, 0)
        } else if (0x3b..=0x44).contains(&code) {
            (keys::F1 + (code - 0x3b) as u32, 0)
        } else if code == 0x57 || code == 0x58 {
            (keys::F1 + 10 + (code - 0x57) as u32, 0)
        } else if (0x02..=0x39).contains(&code) {
            let base = PLAIN[code as usize];
            if base == 0 {
                return None;
            }
            let letter = base.is_ascii_lowercase();
            let upper = self.shift ^ (self.caps && letter);
            let c = if upper && (self.shift || letter) { SHIFTED[code as usize] } else { base };
            let ch = match c {
                b'\r' | 0x08 | b'\t' | 0x1b => 0,
                _ if self.ctrl => 0,
                _ => c as u32,
            };
            (if base == b'\r' { 13 } else { base as u32 }, ch)
        } else {
            return None;
        };
        Some(KeyEvent { key, ch, down, mods: self.mods() })
    }
}

/// Mouse packet decoder.
#[derive(Debug, Default)]
pub struct MouseDecoder {
    pkt: [u8; 4],
    n: usize,
    /// Packets carry a 4th (wheel) byte once IntelliMouse mode is negotiated.
    pub wheel: bool,
}

impl MouseDecoder {
    /// New decoder in 3-byte mode.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one byte; returns a report when a full packet arrives. Resynchronises on a first byte
    /// without the always-set bit 3, and drops packets flagged as overflowed.
    pub fn feed(&mut self, b: u8) -> Option<MouseEvent> {
        if self.n == 0 && b & 8 == 0 {
            return None;
        }
        self.pkt[self.n] = b;
        self.n += 1;
        let need = if self.wheel { 4 } else { 3 };
        if self.n < need {
            return None;
        }
        self.n = 0;
        let p = self.pkt;
        if p[0] & 0xc0 != 0 {
            return None;
        }
        let dx = p[1] as i32 - if p[0] & 0x10 != 0 { 256 } else { 0 };
        let dy = p[2] as i32 - if p[0] & 0x20 != 0 { 256 } else { 0 };
        let (wheel, extra) = if self.wheel {
            let w = (p[3] & 0x0f) as i32;
            (if w & 8 != 0 { w - 16 } else { w }, (p[3] >> 4) & 3)
        } else {
            (0, 0)
        };
        Some(MouseEvent { dx, dy: -dy, wheel: -wheel, buttons: (p[0] & 7) | (extra << 3) })
    }
}

/// The i8042 controller with an attached keyboard and (optional) mouse; polled.
pub struct Ps2<P: PortIo> {
    io: P,
    kbd: KeyboardDecoder,
    mouse: MouseDecoder,
    has_mouse: bool,
}

impl<P: PortIo> Ps2<P> {
    /// Wrap a port-I/O provider; call [`Ps2::init`] before polling.
    pub fn new(io: P) -> Self {
        Ps2 { io, kbd: KeyboardDecoder::new(), mouse: MouseDecoder::new(), has_mouse: false }
    }

    /// Borrow the port provider (tests inspect the mock through this).
    pub fn io(&mut self) -> &mut P {
        &mut self.io
    }

    /// Whether a mouse answered during init.
    pub fn has_mouse(&self) -> bool {
        self.has_mouse
    }

    /// Whether the mouse was switched into 4-byte wheel mode.
    pub fn has_wheel(&self) -> bool {
        self.mouse.wheel
    }

    fn wait_write(&mut self) -> Result<(), DriverError> {
        for _ in 0..SPIN {
            if self.io.inb(CMD) & 2 == 0 {
                return Ok(());
            }
        }
        Err(DriverError::Timeout)
    }

    fn wait_read(&mut self) -> Result<u8, DriverError> {
        for _ in 0..SPIN {
            if self.io.inb(CMD) & 1 != 0 {
                return Ok(self.io.inb(DATA));
            }
        }
        Err(DriverError::Timeout)
    }

    fn drain(&mut self) {
        for _ in 0..64 {
            if self.io.inb(CMD) & 1 == 0 {
                break;
            }
            let _ = self.io.inb(DATA);
        }
    }

    fn mouse_cmd(&mut self, b: u8) -> Result<(), DriverError> {
        self.wait_write()?;
        self.io.outb(CMD, 0xd4);
        self.wait_write()?;
        self.io.outb(DATA, b);
        if self.wait_read()? == 0xfa {
            Ok(())
        } else {
            Err(DriverError::Protocol)
        }
    }

    fn mouse_set_rate(&mut self, r: u8) -> Result<(), DriverError> {
        self.mouse_cmd(0xf3)?;
        self.mouse_cmd(r)
    }

    /// Bring up the controller: drain, enable the aux port, enable clocks, and probe the mouse
    /// (including the 200/100/80 IntelliMouse wheel handshake). A missing mouse is not an error.
    pub fn init(&mut self) -> Result<(), DriverError> {
        self.drain();
        self.wait_write()?;
        self.io.outb(CMD, 0xa8);
        self.wait_write()?;
        self.io.outb(CMD, 0x20);
        let cfg = self.wait_read()?;
        let cfg = (cfg & !0x20) | 0x03;
        self.wait_write()?;
        self.io.outb(CMD, 0x60);
        self.wait_write()?;
        self.io.outb(DATA, cfg);
        self.has_mouse = self.mouse_cmd(0xf6).is_ok();
        if self.has_mouse {
            for r in [200, 100, 80] {
                if self.mouse_set_rate(r).is_err() {
                    break;
                }
            }
            // Device id 3 = IntelliMouse.
            if self.mouse_cmd(0xf2).is_ok() {
                self.mouse.wheel = self.wait_read().map(|id| id == 3).unwrap_or(false);
            }
            self.has_mouse = self.mouse_cmd(0xf4).is_ok();
        }
        self.drain();
        Ok(())
    }

    /// Poll the controller output buffer and return the next decoded event, if any.
    pub fn poll(&mut self) -> Option<InputEvent> {
        for _ in 0..64 {
            let st = self.io.inb(CMD);
            if st & 1 == 0 {
                return None;
            }
            let b = self.io.inb(DATA);
            if st & 0x20 != 0 {
                if let Some(m) = self.mouse.feed(b) {
                    return Some(InputEvent::Mouse(m));
                }
            } else if let Some(k) = self.kbd.feed(b) {
                return Some(InputEvent::Key(k));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::VecDeque;

    #[test]
    fn keyboard_letters_shift_caps() {
        let mut k = KeyboardDecoder::new();
        let e = k.feed(0x1e).unwrap(); // a down
        assert_eq!((e.key, e.ch, e.down), (b'a' as u32, b'a' as u32, true));
        assert!(k.feed(0x2a).is_none()); // shift down
        let e = k.feed(0x1e).unwrap();
        assert_eq!((e.key, e.ch, e.mods), (b'a' as u32, b'A' as u32, mods::SHIFT));
        let e = k.feed(0x02).unwrap();
        assert_eq!(e.ch, b'!' as u32);
        k.feed(0xaa); // shift up
        assert_eq!(k.feed(0x1e).unwrap().ch, b'a' as u32);
        let mut k = KeyboardDecoder::new();
        k.feed(0x3a); // caps on
        assert_eq!(k.feed(0x1e).unwrap().ch, b'A' as u32);
        assert_eq!(k.feed(0x02).unwrap().ch, b'1' as u32); // caps doesn't shift digits
    }

    #[test]
    fn keyboard_extended_and_release() {
        let mut k = KeyboardDecoder::new();
        assert!(k.feed(0xe0).is_none());
        assert_eq!(k.feed(0x48).unwrap().key, keys::UP);
        k.feed(0xe0);
        let e = k.feed(0xc8).unwrap();
        assert!(!e.down && e.key == keys::UP);
        k.feed(0xe0);
        let e = k.feed(0x5b).unwrap();
        assert_eq!(e.mods, mods::SUPER);
        // F1, ctrl suppresses characters
        assert_eq!(k.feed(0x3b).unwrap().key, keys::F1);
        k.feed(0x1d);
        let e = k.feed(0x2e).unwrap(); // ctrl+c
        assert_eq!((e.key, e.ch, e.mods & mods::CTRL), (b'c' as u32, 0, mods::CTRL));
    }

    #[test]
    fn keyboard_pause_sequence_skipped() {
        let mut k = KeyboardDecoder::new();
        for b in [0xe1, 0x1d, 0x45, 0xe1, 0x9d, 0xc5] {
            assert!(k.feed(b).is_none());
        }
        assert!(k.feed(0x1e).is_some());
    }

    #[test]
    fn mouse_packets() {
        let mut m = MouseDecoder::new();
        assert!(m.feed(0x00).is_none()); // out of sync
        assert!(m.feed(0x09).is_none());
        assert!(m.feed(5).is_none());
        let e = m.feed(3).unwrap();
        assert_eq!((e.dx, e.dy, e.buttons), (5, -3, 1));
        // negative deltas
        m.feed(0x08 | 0x10 | 0x20 | 2);
        m.feed(0xfb); // -5
        let e = m.feed(0xfd).unwrap(); // -3
        assert_eq!((e.dx, e.dy, e.buttons), (-5, 3, 2));
        // overflow dropped
        m.feed(0x08 | 0x40);
        m.feed(1);
        assert!(m.feed(1).is_none());
    }

    #[test]
    fn mouse_wheel_packets() {
        let mut m = MouseDecoder::new();
        m.wheel = true;
        m.feed(0x08);
        m.feed(0);
        m.feed(0);
        let e = m.feed(0x0f).unwrap(); // wheel -1 (toward user)
        assert_eq!(e.wheel, 1);
        m.feed(0x08);
        m.feed(0);
        m.feed(0);
        let e = m.feed(0x10 | 0x01).unwrap(); // button 4 + wheel +1
        assert_eq!((e.wheel, e.buttons), (-1, 8));
    }

    /// Minimal i8042 + devices model.
    struct Mock {
        out: VecDeque<(u8, bool)>, // (byte, from_aux)
        wheel_mouse: bool,
        rates: alloc::vec::Vec<u8>,
        expect: Option<u8>, // pending data-port target: 0xd4 aux write, 0x60 config
        aux_pending_param: bool,
    }
    impl Mock {
        fn new(wheel: bool) -> Self {
            Mock { out: VecDeque::new(), wheel_mouse: wheel, rates: alloc::vec![], expect: None, aux_pending_param: false }
        }
    }
    impl PortIo for Mock {
        fn inb(&mut self, port: u16) -> u8 {
            match port {
                CMD => {
                    let mut s = 0;
                    if let Some(&(_, aux)) = self.out.front() {
                        s |= 1 | if aux { 0x20 } else { 0 };
                    }
                    s
                }
                DATA => self.out.pop_front().map(|x| x.0).unwrap_or(0),
                _ => 0,
            }
        }
        fn outb(&mut self, port: u16, v: u8) {
            if port == CMD {
                self.expect = match v {
                    0xd4 | 0x60 => Some(v),
                    0x20 => {
                        self.out.push_back((0x45, false));
                        None
                    }
                    _ => None,
                };
            } else if port == DATA {
                match self.expect.take() {
                    Some(0xd4) => {
                        if self.aux_pending_param {
                            self.aux_pending_param = false;
                            self.rates.push(v);
                            self.out.push_back((0xfa, true));
                        } else {
                            match v {
                                0xf3 => {
                                    self.aux_pending_param = true;
                                    self.out.push_back((0xfa, true));
                                }
                                0xf2 => {
                                    self.out.push_back((0xfa, true));
                                    let id = if self.wheel_mouse && self.rates == [200, 100, 80] { 3 } else { 0 };
                                    self.out.push_back((id, true));
                                }
                                _ => self.out.push_back((0xfa, true)),
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn controller_init_and_poll() {
        let mut p = Ps2::new(Mock::new(true));
        p.init().unwrap();
        assert!(p.has_mouse() && p.has_wheel());
        p.io().out.extend([(0x1e, false), (0x09, true), (2, true), (2, true), (0, true)]);
        match p.poll() {
            Some(InputEvent::Key(k)) => assert_eq!(k.ch, b'a' as u32),
            o => panic!("{:?}", o),
        }
        match p.poll() {
            Some(InputEvent::Mouse(m)) => assert_eq!((m.dx, m.dy, m.buttons), (2, -2, 1)),
            o => panic!("{:?}", o),
        }
        assert!(p.poll().is_none());
        let mut p = Ps2::new(Mock::new(false));
        p.init().unwrap();
        assert!(p.has_mouse() && !p.has_wheel());
    }
}
