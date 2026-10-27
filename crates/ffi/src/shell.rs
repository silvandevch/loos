//! Console host VT core (loos-vt) for the C console/terminal apps.

use crate::{bytes, put_str};
use loos_vt::{parser::Parser, screen::Screen};

pub struct Term {
    parser: Parser,
    screen: Screen,
}

#[no_mangle]
pub extern "C" fn loos_vt_new(cols: u32, rows: u32) -> *mut Term {
    Box::into_raw(Box::new(Term { parser: Parser::new(), screen: Screen::new(cols.clamp(1, 1000) as usize, rows.clamp(1, 1000) as usize) }))
}
#[no_mangle]
pub unsafe extern "C" fn loos_vt_free(t: *mut Term) {
    if !t.is_null() {
        // SAFETY: allocated by loos_vt_new.
        drop(unsafe { Box::from_raw(t) });
    }
}
/// Feeds output bytes (escape sequences are interpreted).
#[no_mangle]
pub unsafe extern "C" fn loos_vt_write(t: *mut Term, data: *const u8, len: usize) {
    // SAFETY: see crate contract.
    unsafe {
        if let Some(t) = t.as_mut() {
            t.screen.write(&mut t.parser, bytes(data, len));
        }
    }
}
/// Cell at (x,y): `*ch` codepoint, `*fg`/`*bg` = 0xAARRGGBB-ish tag (0 default, 0x01000000|idx palette, 0x02000000|rgb), `*attrs` bold=1 underline=2 reverse=4.
#[no_mangle]
pub unsafe extern "C" fn loos_vt_cell(t: *const Term, x: u32, y: u32, ch: *mut u32, fg: *mut u32, bg: *mut u32, attrs: *mut u8) -> i32 {
    use loos_vt::screen::Color;
    let tag = |c: Color| match c {
        Color::Default => 0,
        Color::Idx(i) => 0x0100_0000 | i as u32,
        Color::Rgb(r, g, b) => 0x0200_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32,
    };
    // SAFETY: see crate contract.
    unsafe {
        let Some(t) = t.as_ref() else { return -1 };
        if x as usize >= t.screen.cols || y as usize >= t.screen.rows {
            return -1;
        }
        let c = t.screen.cell(x as usize, y as usize);
        *ch = c.ch as u32;
        *fg = tag(c.fg);
        *bg = tag(c.bg);
        *attrs = c.attrs;
        0
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_vt_cursor(t: *const Term, x: *mut u32, y: *mut u32, visible: *mut bool) {
    // SAFETY: see crate contract.
    unsafe {
        if let Some(t) = t.as_ref() {
            *x = t.screen.cx as u32;
            *y = t.screen.cy as u32;
            *visible = t.screen.cursor_visible;
        }
    }
}
/// Row text with trailing blanks trimmed; returns its length.
#[no_mangle]
pub unsafe extern "C" fn loos_vt_row(t: *const Term, y: u32, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        match t.as_ref() {
            Some(t) if (y as usize) < t.screen.rows => put_str(&t.screen.row_text(y as usize), out, cap) as i64,
            _ => -1,
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_vt_title(t: *const Term, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe { t.as_ref().map_or(-1, |t| put_str(&t.screen.title, out, cap) as i64) }
}
