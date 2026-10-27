//! DEC/ECMA-48 state machine (after Paul Williams' vt100.net diagram) with UTF-8 decoding.

use alloc::vec::Vec;

/// A parsed terminal event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Printable character.
    Print(char),
    /// C0 control (BS, HT, LF, CR, BEL ...).
    Exec(u8),
    /// CSI sequence. `private` is the `? < = >` marker if present.
    Csi { params: Vec<u16>, private: Option<u8>, inter: Option<u8>, fin: char },
    /// ESC sequence (`ESC 7`, `ESC M`, `ESC ( B` ...).
    Esc { inter: Option<u8>, fin: u8 },
    /// OSC string (without terminator).
    Osc(Vec<u8>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Ground,
    Escape,
    EscInter,
    CsiEntry,
    CsiParam,
    CsiInter,
    CsiIgnore,
    Osc,
    OscEsc,
    Ignore, // DCS/SOS/PM/APC payload until ST
    IgnoreEsc,
}

/// Incremental parser; feed bytes, collect events.
pub struct Parser {
    st: St,
    params: Vec<u16>,
    cur: Option<u32>,
    private: Option<u8>,
    inter: Option<u8>,
    osc: Vec<u8>,
    utf: [u8; 4],
    utf_have: usize,
    utf_need: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    /// New parser in ground state.
    pub fn new() -> Self {
        Self { st: St::Ground, params: Vec::new(), cur: None, private: None, inter: None, osc: Vec::new(), utf: [0; 4], utf_have: 0, utf_need: 0 }
    }

    fn clear(&mut self) {
        self.params.clear();
        self.cur = None;
        self.private = None;
        self.inter = None;
    }

    fn push_param(&mut self) {
        if self.params.len() < 32 {
            self.params.push(self.cur.take().map_or(0, |v| v.min(u16::MAX as u32) as u16));
        }
        self.cur = None;
    }

    /// Feed a chunk, appending events to `out`.
    pub fn feed(&mut self, data: &[u8], out: &mut Vec<Event>) {
        for &b in data {
            self.byte(b, out);
        }
    }

    fn byte(&mut self, b: u8, out: &mut Vec<Event>) {
        // CAN/SUB abort any sequence; ESC restarts one (except inside string states handled below).
        if matches!(b, 0x18 | 0x1a) {
            self.st = St::Ground;
            self.utf_need = 0;
            return;
        }
        match self.st {
            St::Osc => match b {
                0x07 => self.end_osc(out, St::Ground),
                0x1b => self.st = St::OscEsc,
                _ => {
                    if self.osc.len() < 4096 {
                        self.osc.push(b)
                    }
                }
            },
            St::OscEsc => {
                if b == b'\\' {
                    self.end_osc(out, St::Ground);
                } else {
                    // Malformed: drop the OSC and treat as new escape.
                    self.osc.clear();
                    self.st = St::Escape;
                    self.byte(b, out);
                }
            }
            St::Ignore => match b {
                0x1b => self.st = St::IgnoreEsc,
                0x07 => self.st = St::Ground,
                _ => {}
            },
            St::IgnoreEsc => {
                self.st = if b == b'\\' { St::Ground } else { St::Escape };
                if self.st == St::Escape {
                    self.byte(b, out);
                }
            }
            _ if b == 0x1b => {
                self.clear();
                self.utf_need = 0;
                self.st = St::Escape;
            }
            St::Ground => self.ground(b, out),
            St::Escape => match b {
                0x00..=0x1f => out.push(Event::Exec(b)),
                b'[' => {
                    self.clear();
                    self.st = St::CsiEntry;
                }
                b']' => {
                    self.osc.clear();
                    self.st = St::Osc;
                }
                b'P' | b'X' | b'^' | b'_' => self.st = St::Ignore,
                0x20..=0x2f => {
                    self.inter = Some(b);
                    self.st = St::EscInter;
                }
                0x30..=0x7e => {
                    out.push(Event::Esc { inter: None, fin: b });
                    self.st = St::Ground;
                }
                _ => self.st = St::Ground,
            },
            St::EscInter => match b {
                0x20..=0x2f => self.inter = Some(b),
                0x30..=0x7e => {
                    out.push(Event::Esc { inter: self.inter, fin: b });
                    self.st = St::Ground;
                }
                _ => self.st = St::Ground,
            },
            St::CsiEntry | St::CsiParam => match b {
                0x00..=0x1f => out.push(Event::Exec(b)),
                b'0'..=b'9' => {
                    self.cur = Some(self.cur.unwrap_or(0).saturating_mul(10).saturating_add((b - b'0') as u32));
                    self.st = St::CsiParam;
                }
                b';' | b':' => {
                    self.push_param();
                    self.st = St::CsiParam;
                }
                b'<'..=b'?' if self.st == St::CsiEntry => {
                    self.private = Some(b);
                    self.st = St::CsiParam;
                }
                b'<'..=b'?' => self.st = St::CsiIgnore,
                0x20..=0x2f => {
                    self.inter = Some(b);
                    self.st = St::CsiInter;
                }
                0x40..=0x7e => self.dispatch(b, out),
                _ => self.st = St::Ground,
            },
            St::CsiInter => match b {
                0x00..=0x1f => out.push(Event::Exec(b)),
                0x20..=0x2f => self.inter = Some(b),
                0x40..=0x7e => self.dispatch(b, out),
                _ => self.st = St::CsiIgnore,
            },
            St::CsiIgnore => {
                if (0x40..=0x7e).contains(&b) {
                    self.st = St::Ground;
                } else if b < 0x20 {
                    out.push(Event::Exec(b));
                }
            }
        }
    }

    fn dispatch(&mut self, fin: u8, out: &mut Vec<Event>) {
        if self.cur.is_some() || !self.params.is_empty() {
            self.push_param();
        }
        out.push(Event::Csi { params: core::mem::take(&mut self.params), private: self.private, inter: self.inter, fin: fin as char });
        self.st = St::Ground;
    }

    fn end_osc(&mut self, out: &mut Vec<Event>, next: St) {
        out.push(Event::Osc(core::mem::take(&mut self.osc)));
        self.st = next;
    }

    fn ground(&mut self, b: u8, out: &mut Vec<Event>) {
        if self.utf_need > 0 {
            if b & 0xC0 == 0x80 {
                self.utf[self.utf_have] = b;
                self.utf_have += 1;
                if self.utf_have == self.utf_need {
                    let c = core::str::from_utf8(&self.utf[..self.utf_need]).ok().and_then(|s| s.chars().next());
                    out.push(Event::Print(c.unwrap_or('\u{FFFD}')));
                    self.utf_need = 0;
                }
                return;
            }
            out.push(Event::Print('\u{FFFD}'));
            self.utf_need = 0;
        }
        match b {
            0x00..=0x1f | 0x7f => {
                if b != 0x7f {
                    out.push(Event::Exec(b))
                }
            }
            0x20..=0x7e => out.push(Event::Print(b as char)),
            0xC2..=0xDF => self.start_utf(b, 2),
            0xE0..=0xEF => self.start_utf(b, 3),
            0xF0..=0xF4 => self.start_utf(b, 4),
            _ => out.push(Event::Print('\u{FFFD}')),
        }
    }

    fn start_utf(&mut self, b: u8, need: usize) {
        self.utf[0] = b;
        self.utf_have = 1;
        self.utf_need = need;
    }
}
