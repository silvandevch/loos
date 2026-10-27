//! Read-only ZIP (stored + deflate), no ZIP64. Defends against path traversal and
//! decompression bombs: every entry name is validated up front and inflation is size-capped to
//! the declared size.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Per-entry and total uncompressed size caps.
pub const MAX_ENTRY: u64 = 2 << 30;
/// Max entries.
pub const MAX_ENTRIES: usize = 100_000;

/// ZIP errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No end-of-central-directory record.
    NoEocd,
    /// Structure truncated or inconsistent.
    Corrupt,
    /// Unsupported feature (ZIP64, encryption, method).
    Unsupported,
    /// Unsafe entry name (absolute, `..`, backslash, drive letter, NUL).
    UnsafePath,
    /// Entry not found.
    NotFound,
    /// Size cap exceeded.
    TooBig,
    /// CRC mismatch or inflated size differs from the header.
    BadData,
}

/// A central-directory entry.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Name (forward slashes).
    pub name: String,
    /// Compression method (0 stored, 8 deflate).
    pub method: u16,
    /// CRC-32.
    pub crc: u32,
    /// Compressed size.
    pub csize: u64,
    /// Uncompressed size.
    pub size: u64,
    offset: u64,
}

impl Entry {
    /// Directory entry?
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }
}

/// An opened archive over a byte slice.
pub struct Archive<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
}

fn u16le(b: &[u8], o: usize) -> Result<u16, Error> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or(Error::Corrupt)
}
fn u32le(b: &[u8], o: usize) -> Result<u32, Error> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or(Error::Corrupt)
}

/// CRC-32 (IEEE).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Is `name` a safe relative path (no traversal)?
pub fn safe_name(name: &str) -> bool {
    if name.is_empty() || name.contains(['\0', '\\']) || name.starts_with('/') {
        return false;
    }
    let b = name.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        return false;
    }
    !name.split('/').any(|c| c == "..")
}

impl<'a> Archive<'a> {
    /// Parse the central directory.
    pub fn open(data: &'a [u8]) -> Result<Archive<'a>, Error> {
        if data.len() < 22 {
            return Err(Error::NoEocd);
        }
        let lo = data.len().saturating_sub(22 + 65_535);
        let eocd = (lo..=data.len() - 22).rev().find(|&i| data[i..i + 4] == [0x50, 0x4b, 5, 6]).ok_or(Error::NoEocd)?;
        let (n, cd_size, cd_off) = (u16le(data, eocd + 10)? as usize, u32le(data, eocd + 12)? as usize, u32le(data, eocd + 16)? as usize);
        if n == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF {
            return Err(Error::Unsupported);
        }
        if n > MAX_ENTRIES || cd_off.checked_add(cd_size).is_none_or(|e| e > eocd) {
            return Err(Error::Corrupt);
        }
        let mut entries = Vec::with_capacity(n);
        let mut p = cd_off;
        for _ in 0..n {
            if u32le(data, p)? != 0x0201_4b50 {
                return Err(Error::Corrupt);
            }
            let flags = u16le(data, p + 8)?;
            if flags & 1 != 0 {
                return Err(Error::Unsupported); // encrypted
            }
            let method = u16le(data, p + 10)?;
            let (crc, csize, size) = (u32le(data, p + 16)?, u32le(data, p + 20)? as u64, u32le(data, p + 24)? as u64);
            let (nl, el, cl) = (u16le(data, p + 28)? as usize, u16le(data, p + 30)? as usize, u16le(data, p + 32)? as usize);
            let offset = u32le(data, p + 42)? as u64;
            let nb = data.get(p + 46..p + 46 + nl).ok_or(Error::Corrupt)?;
            let name = core::str::from_utf8(nb).map_err(|_| Error::UnsafePath)?;
            if !safe_name(name) {
                return Err(Error::UnsafePath);
            }
            if size > MAX_ENTRY || size == 0xFFFF_FFFF {
                return Err(Error::TooBig);
            }
            entries.push(Entry { name: name.to_string(), method, crc, csize, size, offset });
            p += 46 + nl + el + cl;
        }
        Ok(Archive { data, entries })
    }

    /// All entries.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Case-insensitive lookup (APPX paths are case-insensitive).
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Read, inflate and CRC-verify an entry by name.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, Error> {
        let e = self.find(name).ok_or(Error::NotFound)?;
        self.extract(e)
    }

    /// Read, inflate and CRC-verify `e`.
    pub fn extract(&self, e: &Entry) -> Result<Vec<u8>, Error> {
        let d = self.data;
        let lh = e.offset as usize;
        if u32le(d, lh)? != 0x0403_4b50 {
            return Err(Error::Corrupt);
        }
        let start = lh + 30 + u16le(d, lh + 26)? as usize + u16le(d, lh + 28)? as usize;
        let raw = d.get(start..start.checked_add(e.csize as usize).ok_or(Error::Corrupt)?).ok_or(Error::Corrupt)?;
        let out = match e.method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, e.size as usize).map_err(|_| Error::BadData)?,
            _ => return Err(Error::Unsupported),
        };
        if out.len() as u64 != e.size || crc32(&out) != e.crc {
            return Err(Error::BadData);
        }
        Ok(out)
    }
}
