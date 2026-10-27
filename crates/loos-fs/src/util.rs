//! Little-endian field helpers and path splitting shared by the filesystems.
use alloc::string::String;
use alloc::vec::Vec;

pub fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
pub fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
pub fn le64(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}
pub fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
pub fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// Splits `a/b\c` into components; empty and `.` components are dropped, `..` pops (clamped at root).
pub fn split_path(path: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for c in path.split(|ch| ch == '/' || ch == '\\') {
        match c {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// UTF-16LE bytes -> String (lossy on unpaired surrogates, as Windows treats them).
pub fn utf16le_to_string(b: &[u8]) -> String {
    let units = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
    core::char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')).collect()
}

/// UTF-16BE bytes -> String (Joliet, UDF).
pub fn utf16be_to_string(b: &[u8]) -> String {
    let units = b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]]));
    core::char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')).collect()
}

/// Case folding used for Windows-style name comparison (simple, per-char, locale independent).
pub fn fold(s: &str) -> String {
    s.chars().flat_map(|c| c.to_uppercase()).collect()
}
