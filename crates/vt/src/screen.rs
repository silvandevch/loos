//! Cell-grid screen model driven by parser events.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::parser::Event;

/// Cell color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// Terminal default.
    Default,
    /// 256-color palette index.
    Idx(u8),
    /// 24-bit color.
    Rgb(u8, u8, u8),
}

/// Attribute bits.
pub mod attr {
    /// Bold.
    pub const BOLD: u8 = 1;
    /// Underline.
    pub const UNDERLINE: u8 = 2;
    /// Reverse video.
    pub const REVERSE: u8 = 4;
}

/// One screen cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Character.
    pub ch: char,
    /// Foreground.
    pub fg: Color,
    /// Background.
    pub bg: Color,
    /// Attribute bits.
    pub attrs: u8,
}

const BLANK: Cell = Cell { ch: ' ', fg: Color::Default, bg: Color::Default, attrs: 0 };

/// The screen.
pub struct Screen {
    /// Columns.
    pub cols: usize,
    /// Rows.
    pub rows: usize,
    cells: Vec<Cell>,
    /// Cursor column.
    pub cx: usize,
    /// Cursor row.
    pub cy: usize,
    wrap_pending: bool,
    pen: Cell,
    saved: (usize, usize),
    /// Cursor visible (DECTCEM).
    pub cursor_visible: bool,
    /// Window title (OSC 0/2).
    pub title: String,
    /// Lines scrolled off the top, oldest first (bounded).
    pub scrollback: Vec<Vec<Cell>>,
}

impl Screen {
    /// New blank screen.
    pub fn new(cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Self {
            cols,
            rows,
            cells: vec![BLANK; cols * rows],
            cx: 0,
            cy: 0,
            wrap_pending: false,
            pen: BLANK,
            saved: (0, 0),
            cursor_visible: true,
            title: String::new(),
            scrollback: Vec::new(),
        }
    }

    /// Cell at (col,row).
    pub fn cell(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.cols + x]
    }

    /// Row text with trailing blanks trimmed.
    pub fn row_text(&self, y: usize) -> String {
        let s: String = (0..self.cols).map(|x| self.cell(x, y).ch).collect();
        s.trim_end().into()
    }

    fn scroll_up(&mut self, n: usize) {
        for _ in 0..n.min(self.rows) {
            if self.scrollback.len() >= 1000 {
                self.scrollback.remove(0);
            }
            self.scrollback.push(self.cells[..self.cols].to_vec());
            self.cells.copy_within(self.cols.., 0);
            let l = self.cells.len();
            let b = self.blank();
            self.cells[l - self.cols..].fill(b);
        }
    }

    fn scroll_down(&mut self, n: usize) {
        for _ in 0..n.min(self.rows) {
            let l = self.cells.len();
            self.cells.copy_within(..l - self.cols, self.cols);
            let b = self.blank();
            self.cells[..self.cols].fill(b);
        }
    }

    fn blank(&self) -> Cell {
        Cell { ch: ' ', bg: self.pen.bg, ..BLANK }
    }

    fn linefeed(&mut self) {
        if self.cy + 1 == self.rows {
            self.scroll_up(1);
        } else {
            self.cy += 1;
        }
    }

    fn print(&mut self, c: char) {
        if self.wrap_pending {
            self.cx = 0;
            self.linefeed();
            self.wrap_pending = false;
        }
        let i = self.cy * self.cols + self.cx;
        self.cells[i] = Cell { ch: c, ..self.pen };
        if self.cx + 1 == self.cols {
            self.wrap_pending = true;
        } else {
            self.cx += 1;
        }
    }

    /// Apply one parser event.
    pub fn apply(&mut self, e: &Event) {
        match e {
            Event::Print(c) => self.print(*c),
            Event::Exec(b) => match b {
                b'\n' | 0x0b | 0x0c => {
                    self.wrap_pending = false;
                    self.linefeed()
                }
                b'\r' => {
                    self.cx = 0;
                    self.wrap_pending = false;
                }
                0x08 => {
                    self.cx = self.cx.saturating_sub(1);
                    self.wrap_pending = false;
                }
                b'\t' => {
                    self.cx = ((self.cx / 8 + 1) * 8).min(self.cols - 1);
                    self.wrap_pending = false;
                }
                _ => {}
            },
            Event::Esc { inter: None, fin } => match fin {
                b'7' => self.saved = (self.cx, self.cy),
                b'8' => {
                    self.cx = self.saved.0.min(self.cols - 1);
                    self.cy = self.saved.1.min(self.rows - 1);
                }
                b'D' => self.linefeed(),
                b'M' => {
                    if self.cy == 0 {
                        self.scroll_down(1)
                    } else {
                        self.cy -= 1
                    }
                }
                b'E' => {
                    self.cx = 0;
                    self.linefeed()
                }
                b'c' => *self = Screen::new(self.cols, self.rows),
                _ => {}
            },
            Event::Esc { .. } => {}
            Event::Osc(s) => {
                let mut it = s.splitn(2, |&b| b == b';');
                if matches!(it.next(), Some(b"0") | Some(b"2")) {
                    if let Some(t) = it.next() {
                        self.title = String::from_utf8_lossy(t).into_owned();
                    }
                }
            }
            Event::Csi { params, private, inter, fin } => {
                if inter.is_some() {
                    return;
                }
                if *private == Some(b'?') {
                    let on = *fin == 'h';
                    if matches!(fin, 'h' | 'l') && params.contains(&25) {
                        self.cursor_visible = on;
                    }
                    return;
                }
                if private.is_some() {
                    return;
                }
                self.csi(params, *fin);
            }
        }
    }

    fn csi(&mut self, p: &[u16], fin: char) {
        let n = |i: usize, d: usize| p.get(i).copied().filter(|&v| v != 0).map_or(d, |v| v as usize);
        self.wrap_pending = false;
        match fin {
            'A' => self.cy = self.cy.saturating_sub(n(0, 1)),
            'B' => self.cy = (self.cy + n(0, 1)).min(self.rows - 1),
            'C' => self.cx = (self.cx + n(0, 1)).min(self.cols - 1),
            'D' => self.cx = self.cx.saturating_sub(n(0, 1)),
            'E' => {
                self.cy = (self.cy + n(0, 1)).min(self.rows - 1);
                self.cx = 0;
            }
            'F' => {
                self.cy = self.cy.saturating_sub(n(0, 1));
                self.cx = 0;
            }
            'G' => self.cx = (n(0, 1) - 1).min(self.cols - 1),
            'd' => self.cy = (n(0, 1) - 1).min(self.rows - 1),
            'H' | 'f' => {
                self.cy = (n(0, 1) - 1).min(self.rows - 1);
                self.cx = (n(1, 1) - 1).min(self.cols - 1);
            }
            'J' => {
                let b = self.blank();
                let at = self.cy * self.cols + self.cx;
                match p.first().copied().unwrap_or(0) {
                    0 => self.cells[at..].fill(b),
                    1 => self.cells[..=at].fill(b),
                    2 | 3 => self.cells.fill(b),
                    _ => {}
                }
            }
            'K' => {
                let b = self.blank();
                let row = self.cy * self.cols;
                match p.first().copied().unwrap_or(0) {
                    0 => self.cells[row + self.cx..row + self.cols].fill(b),
                    1 => self.cells[row..=row + self.cx].fill(b),
                    2 => self.cells[row..row + self.cols].fill(b),
                    _ => {}
                }
            }
            'L' | 'M' => {
                // insert/delete lines within cy..rows
                let k = n(0, 1).min(self.rows - self.cy);
                let (c, y, rows) = (self.cols, self.cy, self.rows);
                let b = self.blank();
                if fin == 'L' {
                    self.cells.copy_within(y * c..(rows - k) * c, (y + k) * c);
                    self.cells[y * c..(y + k) * c].fill(b);
                } else {
                    self.cells.copy_within((y + k) * c..rows * c, y * c);
                    self.cells[(rows - k) * c..].fill(b);
                }
            }
            'S' => self.scroll_up(n(0, 1)),
            'T' => self.scroll_down(n(0, 1)),
            's' => self.saved = (self.cx, self.cy),
            'u' => {
                self.cx = self.saved.0.min(self.cols - 1);
                self.cy = self.saved.1.min(self.rows - 1);
            }
            'm' => self.sgr(p),
            _ => {}
        }
    }

    fn sgr(&mut self, p: &[u16]) {
        if p.is_empty() {
            self.pen = BLANK;
            return;
        }
        let mut i = 0;
        while i < p.len() {
            match p[i] {
                0 => self.pen = BLANK,
                1 => self.pen.attrs |= attr::BOLD,
                4 => self.pen.attrs |= attr::UNDERLINE,
                7 => self.pen.attrs |= attr::REVERSE,
                22 => self.pen.attrs &= !attr::BOLD,
                24 => self.pen.attrs &= !attr::UNDERLINE,
                27 => self.pen.attrs &= !attr::REVERSE,
                v @ 30..=37 => self.pen.fg = Color::Idx((v - 30) as u8),
                v @ 90..=97 => self.pen.fg = Color::Idx((v - 90 + 8) as u8),
                v @ 40..=47 => self.pen.bg = Color::Idx((v - 40) as u8),
                v @ 100..=107 => self.pen.bg = Color::Idx((v - 100 + 8) as u8),
                39 => self.pen.fg = Color::Default,
                49 => self.pen.bg = Color::Default,
                v @ (38 | 48) => {
                    let col = match p.get(i + 1) {
                        Some(5) if i + 2 < p.len() => {
                            i += 2;
                            Some(Color::Idx(p[i] as u8))
                        }
                        Some(2) if i + 4 < p.len() => {
                            i += 4;
                            Some(Color::Rgb(p[i - 2] as u8, p[i - 1] as u8, p[i] as u8))
                        }
                        _ => None,
                    };
                    if let Some(c) = col {
                        if v == 38 {
                            self.pen.fg = c
                        } else {
                            self.pen.bg = c
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// Convenience: parse `bytes` and apply all events.
    pub fn write(&mut self, parser: &mut crate::parser::Parser, bytes: &[u8]) {
        let mut ev = Vec::new();
        parser.feed(bytes, &mut ev);
        for e in &ev {
            self.apply(e);
        }
    }
}
