//! C ABI for the Rust crates. Every exported symbol is `loos_*`, takes plain pointers/lengths, never
//! unwinds (panics abort in release builds, and each entry point is panic-free on valid input), and is
//! declared in `src/rust/loos_rust.h`. Functions returning `int` use 0 / negative LOOS_E* codes unless noted.
//!
//! Safety contract for every function: pointers must be valid for the stated length, or null where noted.
#![allow(clippy::missing_safety_doc, missing_docs)]

use std::slice;

pub mod crypto;
/// Shell drawing core: its `extern "C"` symbols land in libloos_ffi.a.
pub use loos_shell;
pub mod exec;
pub mod iso;
pub mod kernel;
pub mod storage;
pub mod misc;
pub mod pkg;
pub mod shell;

/// Builds a `&[u8]` from a C pointer/length; null or zero length yields an empty slice.
pub(crate) unsafe fn bytes<'a>(p: *const u8, n: usize) -> &'a [u8] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        // SAFETY: caller guarantees `p` is valid for `n` bytes (module contract).
        unsafe { slice::from_raw_parts(p, n) }
    }
}

/// Mutable counterpart of [`bytes`].
pub(crate) unsafe fn bytes_mut<'a>(p: *mut u8, n: usize) -> &'a mut [u8] {
    if p.is_null() || n == 0 {
        &mut []
    } else {
        // SAFETY: caller guarantees `p` is valid for `n` writable bytes.
        unsafe { slice::from_raw_parts_mut(p, n) }
    }
}

/// UTF-8 view of a NUL-terminated C string (lossy decoding is avoided: invalid UTF-8 yields `None`).
pub(crate) unsafe fn cstr<'a>(p: *const std::ffi::c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: caller guarantees a NUL-terminated string.
    unsafe { std::ffi::CStr::from_ptr(p) }.to_str().ok()
}

/// Copies `s` into `out` (cap bytes) with a NUL terminator; returns the full length needed (excluding NUL).
pub(crate) unsafe fn put_str(s: &str, out: *mut u8, cap: usize) -> usize {
    if cap > 0 && !out.is_null() {
        let n = s.len().min(cap - 1);
        // SAFETY: `out` valid for `cap` bytes; n < cap.
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), out, n);
            *out.add(n) = 0;
        }
    }
    s.len()
}
