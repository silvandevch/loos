//! Console host VT support (task #161): [`parser::Parser`] turns a byte stream into events,
//! [`screen::Screen`] applies them to a cell grid (cursor, SGR, erase, scroll, title).
#![no_std]

extern crate alloc;

pub mod parser;
pub mod screen;
