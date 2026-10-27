//! OLE compound file (CFB v3/v4) reader. Flat access to streams by name; MSI files keep all
//! streams in the root storage.

use alloc::string::String;
use alloc::vec::Vec;

const FREE: u32 = 0xFFFF_FFFF;
const END: u32 = 0xFFFF_FFFE;
const SIG: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
/// Upper bound on a single stream.
pub const MAX_STREAM: u64 = 1 << 30;

/// CFB errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a compound file.
    BadSignature,
    /// Structure truncated, looped or inconsistent.
    Corrupt,
    /// Unsupported sector size.
    Unsupported,
    /// No such stream.
    NotFound,
    /// Stream too large.
    TooBig,
}

/// A directory entry.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Raw UTF-16 name decoded to a Rust string (MSI-mangled; see [`crate::database::decode_name`]).
    pub raw_name: Vec<u16>,
    /// 1 storage, 2 stream, 5 root.
    pub kind: u8,
    start: u32,
    size: u64,
}

/// An opened compound file.
pub struct Cfb<'a> {
    data: &'a [u8],
    sec: usize,
    mini_sec: usize,
    cutoff: u64,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    mini: Vec<u8>,
    /// Directory entries (index 0 = root).
    pub entries: Vec<Entry>,
}

fn le32(b: &[u8], o: usize) -> Result<u32, Error> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or(Error::Corrupt)
}

impl<'a> Cfb<'a> {
    fn sector(&self, i: u32) -> Result<&'a [u8], Error> {
        let off = (i as usize + 1).checked_mul(self.sec).ok_or(Error::Corrupt)?;
        self.data.get(off..off + self.sec).ok_or(Error::Corrupt)
    }

    fn chain(&self, fat: &[u32], mut s: u32, limit: usize) -> Result<Vec<u32>, Error> {
        let mut v = Vec::new();
        while s != END {
            if s as usize >= fat.len() || v.len() >= limit {
                return Err(Error::Corrupt);
            }
            v.push(s);
            s = fat[s as usize];
        }
        Ok(v)
    }

    /// Open and index.
    pub fn open(data: &'a [u8]) -> Result<Cfb<'a>, Error> {
        if data.len() < 512 || data[..8] != SIG {
            return Err(Error::BadSignature);
        }
        let shift = u16::from_le_bytes([data[30], data[31]]);
        let sec = match shift {
            9 => 512,
            12 => 4096,
            _ => return Err(Error::Unsupported),
        };
        if u16::from_le_bytes([data[32], data[33]]) != 6 {
            return Err(Error::Unsupported);
        }
        let mut c = Cfb { data, sec, mini_sec: 64, cutoff: le32(data, 56)? as u64, fat: Vec::new(), mini_fat: Vec::new(), mini: Vec::new(), entries: Vec::new() };
        let nsec = data.len() / sec; // upper bound on valid sector numbers
        // FAT sector list: 109 in header, rest via DIFAT chain.
        let mut fat_secs: Vec<u32> = (0..109).map(|i| le32(data, 76 + i * 4)).collect::<Result<_, _>>()?;
        let mut dif = le32(data, 68)?;
        let mut hops = 0;
        while dif != END && dif != FREE {
            hops += 1;
            if hops > nsec {
                return Err(Error::Corrupt);
            }
            let s = c.sector(dif)?;
            for i in 0..(sec / 4 - 1) {
                fat_secs.push(le32(s, i * 4)?);
            }
            dif = le32(s, sec - 4)?;
        }
        for fs in fat_secs.into_iter().filter(|&s| s != FREE) {
            let s = c.sector(fs)?;
            for i in 0..sec / 4 {
                c.fat.push(le32(s, i * 4)?);
            }
        }
        // Directory.
        let first_dir = le32(data, 48)?;
        let mut raw = Vec::new();
        for s in c.chain(&c.fat, first_dir, nsec)? {
            raw.extend_from_slice(c.sector(s)?);
        }
        for e in raw.chunks_exact(128) {
            let nlen = (u16::from_le_bytes([e[64], e[65]]) as usize).min(64);
            let units: Vec<u16> = e[..nlen.saturating_sub(2)].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            let size = u64::from_le_bytes(e[120..128].try_into().unwrap());
            c.entries.push(Entry { raw_name: units, kind: e[66], start: le32(e, 116)?, size: if sec == 512 { size & 0xFFFF_FFFF } else { size } });
        }
        if c.entries.first().map(|e| e.kind) != Some(5) {
            return Err(Error::Corrupt);
        }
        // Mini FAT + mini stream.
        let mf = le32(data, 60)?;
        if mf != END && mf != FREE {
            for s in c.chain(&c.fat, mf, nsec)? {
                let b = c.sector(s)?;
                for i in 0..sec / 4 {
                    c.mini_fat.push(le32(b, i * 4)?);
                }
            }
        }
        let root = c.entries[0].clone();
        if root.size > 0 {
            c.mini = c.read_chain(&root, false)?;
        }
        Ok(c)
    }

    fn read_chain(&self, e: &Entry, mini: bool) -> Result<Vec<u8>, Error> {
        if e.size > MAX_STREAM {
            return Err(Error::TooBig);
        }
        let (fat, ssz): (&[u32], usize) = if mini { (&self.mini_fat, self.mini_sec) } else { (&self.fat, self.sec) };
        let limit = e.size as usize / ssz + 2;
        let chain = self.chain(fat, e.start, limit)?;
        if (chain.len() * ssz) < e.size as usize {
            return Err(Error::Corrupt);
        }
        let mut out = Vec::with_capacity(e.size as usize);
        for s in chain {
            if mini {
                let o = s as usize * ssz;
                out.extend_from_slice(self.mini.get(o..o + ssz).ok_or(Error::Corrupt)?);
            } else {
                out.extend_from_slice(self.sector(s)?);
            }
        }
        out.truncate(e.size as usize);
        Ok(out)
    }

    /// Read stream entry `i`.
    pub fn read_entry(&self, i: usize) -> Result<Vec<u8>, Error> {
        let e = self.entries.get(i).ok_or(Error::NotFound)?;
        if e.kind != 2 {
            return Err(Error::NotFound);
        }
        self.read_chain(e, e.size < self.cutoff)
    }

    /// Stream indices.
    pub fn streams(&self) -> impl Iterator<Item = usize> + '_ {
        self.entries.iter().enumerate().filter(|(_, e)| e.kind == 2).map(|(i, _)| i)
    }

    /// Plain-UTF-16 name lookup (for non-MSI use, e.g. `\u{5}SummaryInformation`).
    pub fn find_plain(&self, name: &str) -> Option<usize> {
        let w: Vec<u16> = name.encode_utf16().collect();
        self.entries.iter().position(|e| e.kind == 2 && e.raw_name == w)
    }

    /// Decoded raw name as lossy string (diagnostics).
    pub fn name_lossy(&self, i: usize) -> String {
        String::from_utf16_lossy(&self.entries[i].raw_name)
    }
}
