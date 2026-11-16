//! CoreFoundation's value logic: strings, numbers and the storage of data, arrays and
//! dictionaries. CF objects are Objective-C objects living in the program's memory (so a
//! `CFStringRef` is an `NSString *`); the thin class layer in `libc/cf*.m` calls these functions
//! for everything beyond reading a field.
//!
//! Every CF value object starts `{isa, flags, data, count, capacity}`: for strings the layout of
//! the compiler's constant strings (`data` = characters, `count` = length), for data and arrays a
//! buffer of bytes or object pointers, for dictionaries and sets a hash table.

pub mod collection;
pub mod number;
pub mod string;

use crate::libsys::Call;

/// Field offsets in a CF value object.
pub(crate) const O_FLAGS: u64 = 8;
pub(crate) const O_DATA: u64 = 16;
pub(crate) const O_COUNT: u64 = 24;
pub(crate) const O_CAP: u64 = 32;
/// The buffer at `data` was allocated for this object (it is freed and may be resized).
pub(crate) const F_OWNED: u64 = 1 << 16;

pub(crate) fn rd(c: &mut Call, a: u64) -> u64 {
    c.mem.read_u64(a).unwrap_or(0)
}
pub(crate) fn wr(c: &mut Call, a: u64, v: u64) {
    let _ = c.mem.write_u64(a, v);
}

pub mod plist;
pub mod url;

/// The bytes `p..p + n` of the program's memory.
pub(crate) fn bytes(c: &mut Call, p: u64, n: u64) -> alloc::vec::Vec<u8> {
    if p == 0 || n == 0 { alloc::vec::Vec::new() } else { c.mem.read_vec(p, n as usize).unwrap_or_default() }
}
/// A malloc'ed NUL-terminated copy of `b` in the program (the caller frees it).
pub(crate) fn cstr_out(c: &mut Call, b: &[u8]) -> u64 {
    let p = c.st.heap.alloc(c.mem, b.len() as u64 + 1);
    let _ = c.mem.write(p, b);
    let _ = c.mem.write(p + b.len() as u64, &[0]);
    p
}
