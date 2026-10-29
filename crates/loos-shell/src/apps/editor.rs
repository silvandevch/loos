//! Plain-text editing buffer for Notepad: lines, caret, scrolling. Byte offsets, UTF-8 safe.

use alloc::string::String;
use alloc::vec::Vec;

/// Keys the editor understands (values match `KEY_*` in shell.h).
#[allow(missing_docs)]
pub mod key {
    pub const BACKSPACE: u32 = 8;
    pub const ENTER: u32 = 13;
    pub const TAB: u32 = 9;
    pub const DELETE: u32 = 127;
    pub const UP: u32 = 0x100;
    pub const DOWN: u32 = 0x101;
    pub const LEFT: u32 = 0x102;
    pub const RIGHT: u32 = 0x103;
    pub const HOME: u32 = 0x104;
    pub const END: u32 = 0x105;
    pub const PGUP: u32 = 0x106;
    pub const PGDN: u32 = 0x107;
}

/// Text buffer with a caret.
pub struct Editor {
    /// Lines without terminators.
    pub lines: Vec<String>,
    /// Caret line.
    pub row: usize,
    /// Caret byte offset in the line.
    pub col: usize,
    /// First visible line.
    pub top: usize,
    /// Modified since load/save.
    pub dirty: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Self { lines: alloc::vec![String::new()], row: 0, col: 0, top: 0, dirty: false }
    }
}

fn prev_boundary(s: &str, i: usize) -> usize {
    (0..i).rev().find(|&k| s.is_char_boundary(k)).unwrap_or(0)
}
fn next_boundary(s: &str, i: usize) -> usize {
    (i + 1..=s.len()).find(|&k| s.is_char_boundary(k)).unwrap_or(s.len())
}

impl Editor {
    /// Replaces the contents (CRLF or LF; invalid UTF-8 is replaced).
    pub fn load(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        self.lines = text.split('\n').map(|l| String::from(l.strip_suffix('\r').unwrap_or(l))).collect();
        *self = Editor { lines: core::mem::take(&mut self.lines), ..Default::default() };
    }
    /// Contents with LF line ends.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
    fn clamp_col(&mut self) {
        let l = &self.lines[self.row];
        self.col = self.col.min(l.len());
        while !l.is_char_boundary(self.col) {
            self.col -= 1;
        }
    }
    /// Inserts typed text at the caret.
    pub fn insert(&mut self, c: char) {
        self.lines[self.row].insert(self.col, c);
        self.col += c.len_utf8();
        self.dirty = true;
    }
    /// Handles an editing/navigation key; `page` = visible lines.
    pub fn key(&mut self, k: u32, page: usize) {
        match k {
            key::ENTER => {
                let rest = self.lines[self.row].split_off(self.col);
                self.row += 1;
                self.lines.insert(self.row, rest);
                self.col = 0;
                self.dirty = true;
            }
            key::TAB => (0..4).for_each(|_| self.insert(' ')),
            key::BACKSPACE => {
                if self.col > 0 {
                    let p = prev_boundary(&self.lines[self.row], self.col);
                    self.lines[self.row].replace_range(p..self.col, "");
                    self.col = p;
                } else if self.row > 0 {
                    let cur = self.lines.remove(self.row);
                    self.row -= 1;
                    self.col = self.lines[self.row].len();
                    self.lines[self.row].push_str(&cur);
                } else {
                    return;
                }
                self.dirty = true;
            }
            key::DELETE => {
                let len = self.lines[self.row].len();
                if self.col < len {
                    let n = next_boundary(&self.lines[self.row], self.col);
                    self.lines[self.row].replace_range(self.col..n, "");
                } else if self.row + 1 < self.lines.len() {
                    let next = self.lines.remove(self.row + 1);
                    self.lines[self.row].push_str(&next);
                } else {
                    return;
                }
                self.dirty = true;
            }
            key::LEFT => {
                if self.col > 0 {
                    self.col = prev_boundary(&self.lines[self.row], self.col);
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.lines[self.row].len();
                }
            }
            key::RIGHT => {
                if self.col < self.lines[self.row].len() {
                    self.col = next_boundary(&self.lines[self.row], self.col);
                } else if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = 0;
                }
            }
            key::UP | key::PGUP => {
                self.row = self.row.saturating_sub(if k == key::UP { 1 } else { page.max(1) });
                self.clamp_col();
            }
            key::DOWN | key::PGDN => {
                self.row = (self.row + if k == key::DOWN { 1 } else { page.max(1) }).min(self.lines.len() - 1);
                self.clamp_col();
            }
            key::HOME => self.col = 0,
            key::END => self.col = self.lines[self.row].len(),
            _ => {}
        }
    }
    /// Keeps the caret inside `page` visible lines.
    pub fn scroll_to_caret(&mut self, page: usize) {
        let page = page.max(1);
        if self.row < self.top {
            self.top = self.row;
        } else if self.row >= self.top + page {
            self.top = self.row + 1 - page;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_round_trip() {
        let mut e = Editor::default();
        e.load("ab\r\nc\u{e4}d".as_bytes());
        assert_eq!(e.lines, ["ab", "c\u{e4}d"]);
        e.key(key::DOWN, 10);
        e.key(key::END, 10);
        e.key(key::LEFT, 10);
        e.key(key::LEFT, 10);
        e.key(key::BACKSPACE, 10);                                     /* deletes 'c' */
        assert_eq!(e.lines[1], "\u{e4}d");
        e.key(key::BACKSPACE, 10);                                     /* joins lines */
        assert_eq!(e.text(), "ab\u{e4}d");
        e.key(key::ENTER, 10);
        e.insert('x');
        assert_eq!(e.text(), "ab\nx\u{e4}d");
        e.key(key::DELETE, 10);
        assert_eq!(e.text(), "ab\nxd");
        assert!(e.dirty);
    }
}
