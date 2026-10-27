//! FAT12/16/32 with VFAT long names — read/write.
//!
//! The whole FAT is cached in memory (largest sane ESP/boot volumes are a few MiB of FAT) and
//! written back to every FAT copy by [`Fat::sync`]. Directories are read and rewritten as whole
//! cluster chains, so every mutating call leaves the on-disk structure `fsck.fat`-clean once
//! `sync` has run. Name lookup is case-insensitive (Windows semantics), case is preserved.
use crate::util::*;
use crate::{BlockDev, FileKind, FsError, Result};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// FAT variant, decided (as the spec requires) purely by the cluster count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FatKind {
    /// 12-bit entries.
    Fat12,
    /// 16-bit entries.
    Fat16,
    /// 28-bit entries.
    Fat32,
}

const ATTR_VOLUME: u8 = 0x08;
const ATTR_DIR: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LFN: u8 = 0x0f;
const MAX_LFN: usize = 255;

/// Directory entry as seen by callers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// Long (display) name.
    pub name: String,
    /// 8.3 name as stored (`NAME    EXT` joined with a dot, trimmed).
    pub short_name: String,
    /// File or directory.
    pub kind: FileKind,
    /// Size in bytes (0 for directories).
    pub size: u32,
    /// Raw attribute byte.
    pub attr: u8,
    /// First cluster (0 = empty file).
    pub first_cluster: u32,
    /// Last-write time, Unix seconds (2 s resolution).
    pub mtime: u64,
}

#[derive(Clone, Debug)]
struct Raw {
    name: String,
    short: [u8; 11],
    attr: u8,
    nt: u8,
    cluster: u32,
    size: u32,
    wtime: u16,
    wdate: u16,
    /// Index of the first slot (LFN start) and of the 8.3 slot.
    first_slot: usize,
    slot: usize,
}

struct Dir {
    /// Cluster chain; empty for the fixed FAT12/16 root.
    chain: Vec<u32>,
    data: Vec<u8>,
}

/// A mounted FAT volume.
pub struct Fat<D: BlockDev> {
    dev: D,
    base: u64,
    kind: FatKind,
    bps: u32,
    spc: u32,
    reserved: u32,
    nfats: u32,
    fat_sectors: u32,
    root_entries: u32,
    root_cluster: u32,
    first_data_sector: u32,
    clusters: u32,
    fsinfo_sector: u32,
    fat: Vec<u8>,
    fat_dirty: bool,
    next_hint: u32,
    label: String,
    now: u64,
    read_only: bool,
}

pub(crate) fn dos_to_unix(date: u16, time: u16) -> u64 {
    let (y, m, d) = (1980 + (date >> 9) as i64, ((date >> 5) & 15) as i64, (date & 31) as i64);
    if m == 0 || d == 0 {
        return 0;
    }
    let (h, mi, s) = ((time >> 11) as u64, ((time >> 5) & 63) as u64, ((time & 31) * 2) as u64);
    let days = days_from_civil(y, m, d);
    if days < 0 {
        return 0;
    }
    days as u64 * 86400 + h * 3600 + mi * 60 + s
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub(crate) fn unix_to_dos(t: u64) -> (u16, u16) {
    let days = (t / 86400) as i64;
    let (y, m, d) = civil_from_days(days);
    if y < 1980 {
        return (0x21, 0);
    }
    let y = core::cmp::min(y, 2107);
    let rem = t % 86400;
    let date = (((y - 1980) as u16) << 9) | ((m as u16) << 5) | d as u16;
    let time = (((rem / 3600) as u16) << 11) | ((((rem % 3600) / 60) as u16) << 5) | ((rem % 60 / 2) as u16);
    (date, time)
}

fn lfn_checksum(short: &[u8; 11]) -> u8 {
    let mut sum = 0u8;
    for &b in short.iter() {
        sum = ((sum & 1) << 7).wrapping_add(sum >> 1).wrapping_add(b);
    }
    sum
}

fn short_to_string(short: &[u8; 11], nt: u8) -> String {
    let mut base: String = short[..8].iter().map(|&b| b as char).collect::<String>().trim_end().into();
    let mut ext: String = short[8..].iter().map(|&b| b as char).collect::<String>().trim_end().into();
    if nt & 0x08 != 0 {
        base = base.to_lowercase();
    }
    if nt & 0x10 != 0 {
        ext = ext.to_lowercase();
    }
    if ext.is_empty() {
        base
    } else {
        let mut s = base;
        s.push('.');
        s.push_str(&ext);
        s
    }
}

fn valid_short_char(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || "$%'-_@~`!(){}^#&".contains(c)
}

/// Rejects names FAT cannot store.
pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(FsError::InvalidName);
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err(FsError::InvalidName);
    }
    if name.encode_utf16().count() > MAX_LFN {
        return Err(FsError::InvalidName);
    }
    for c in name.chars() {
        if (c as u32) < 0x20 || "\"*/:<>?\\|".contains(c) {
            return Err(FsError::InvalidName);
        }
    }
    Ok(())
}

/// Tries to express `name` exactly as an 8.3 entry (with NT case flags).
fn exact_short(name: &str) -> Option<([u8; 11], u8)> {
    let (base, ext) = match name.rfind('.') {
        Some(p) if p > 0 => (&name[..p], &name[p + 1..]),
        _ => (name, ""),
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || base.contains('.') {
        return None;
    }
    let lower_base = base.chars().any(|c| c.is_ascii_lowercase());
    let upper_base = base.chars().any(|c| c.is_ascii_uppercase());
    let lower_ext = ext.chars().any(|c| c.is_ascii_lowercase());
    let upper_ext = ext.chars().any(|c| c.is_ascii_uppercase());
    if (lower_base && upper_base) || (lower_ext && upper_ext) {
        return None;
    }
    let mut out = [b' '; 11];
    for (i, c) in base.chars().enumerate() {
        let u = c.to_ascii_uppercase();
        if !valid_short_char(u) {
            return None;
        }
        out[i] = u as u8;
    }
    for (i, c) in ext.chars().enumerate() {
        let u = c.to_ascii_uppercase();
        if !valid_short_char(u) {
            return None;
        }
        out[8 + i] = u as u8;
    }
    if out[0] == 0xe5 {
        out[0] = 0x05;
    }
    let mut nt = 0;
    if lower_base {
        nt |= 0x08;
    }
    if lower_ext {
        nt |= 0x10;
    }
    Some((out, nt))
}

impl<D: BlockDev> Fat<D> {
    /// Mounts the volume starting at byte `base` of `dev`.
    pub fn mount(mut dev: D, base: u64) -> Result<Self> {
        let mut bs = [0u8; 512];
        dev.read_at(base, &mut bs)?;
        if bs[510] != 0x55 || bs[511] != 0xaa {
            return Err(FsError::NotRecognized);
        }
        let bps = le16(&bs, 11) as u32;
        let spc = bs[13] as u32;
        let reserved = le16(&bs, 14) as u32;
        let nfats = bs[16] as u32;
        let root_entries = le16(&bs, 17) as u32;
        let ts16 = le16(&bs, 19) as u32;
        let fs16 = le16(&bs, 22) as u32;
        let ts32 = le32(&bs, 32);
        let fs32 = le32(&bs, 36);
        if !(bps == 512 || bps == 1024 || bps == 2048 || bps == 4096)
            || spc == 0
            || !spc.is_power_of_two()
            || reserved == 0
            || nfats == 0
            || nfats > 2
            || bps * spc > 65536
        {
            return Err(FsError::NotRecognized);
        }
        // exFAT / NTFS have a zero BPB here; keep them out.
        if &bs[3..11] == b"EXFAT   " || &bs[3..11] == b"NTFS    " {
            return Err(FsError::NotRecognized);
        }
        let fat_sectors = if fs16 != 0 { fs16 } else { fs32 };
        let total_sectors = if ts16 != 0 { ts16 } else { ts32 };
        if fat_sectors == 0 || total_sectors == 0 {
            return Err(FsError::NotRecognized);
        }
        let root_dir_sectors = (root_entries * 32 + bps - 1) / bps;
        let meta = reserved
            .checked_add(nfats.checked_mul(fat_sectors).ok_or(FsError::Corrupt)?)
            .and_then(|v| v.checked_add(root_dir_sectors))
            .ok_or(FsError::Corrupt)?;
        if meta >= total_sectors || (total_sectors as u64) * (bps as u64) > dev.len().saturating_sub(base) {
            return Err(FsError::Corrupt);
        }
        let clusters = (total_sectors - meta) / spc;
        // Linux/Windows disagree on tiny FAT32 volumes; the BPB layout (no 16-bit FAT size) is unambiguous.
        let kind = if fs16 == 0 {
            FatKind::Fat32
        } else if clusters < 4085 {
            FatKind::Fat12
        } else {
            FatKind::Fat16
        };
        let (root_cluster, fsinfo_sector) = if kind == FatKind::Fat32 {
            (le32(&bs, 44), le16(&bs, 48) as u32)
        } else {
            (0, 0)
        };
        if kind == FatKind::Fat32 && (root_cluster < 2 || root_cluster >= clusters + 2) {
            return Err(FsError::Corrupt);
        }
        // Need at least (clusters+2) entries in the FAT.
        let need_bytes = match kind {
            FatKind::Fat12 => ((clusters as u64 + 2) * 3 + 1) / 2,
            FatKind::Fat16 => (clusters as u64 + 2) * 2,
            FatKind::Fat32 => (clusters as u64 + 2) * 4,
        };
        if need_bytes > fat_sectors as u64 * bps as u64 {
            return Err(FsError::Corrupt);
        }
        let mut fat = vec![0u8; (fat_sectors * bps) as usize];
        dev.read_at(base + (reserved * bps) as u64, &mut fat)?;
        let mut fs = Fat {
            dev,
            base,
            kind,
            bps,
            spc,
            reserved,
            nfats,
            fat_sectors,
            root_entries,
            root_cluster,
            first_data_sector: meta,
            clusters,
            fsinfo_sector,
            fat,
            fat_dirty: false,
            next_hint: 2,
            label: String::new(),
            now: 0,
            read_only: false,
        };
        let lbl_off = if kind == FatKind::Fat32 { 71 } else { 43 };
        fs.label = bs[lbl_off..lbl_off + 11].iter().map(|&b| b as char).collect::<String>().trim_end().into();
        if let Ok(root) = fs.load_dir(0) {
            for r in fs.parse_dir(&root.data).0 {
                if r.attr & (ATTR_VOLUME | ATTR_LFN) == ATTR_VOLUME {
                    fs.label = String::from_utf8_lossy(&r.short).trim_end().into();
                }
            }
        }
        Ok(fs)
    }

    /// Variant of this volume.
    pub fn kind(&self) -> FatKind {
        self.kind
    }
    /// Volume label (root-directory label entry if present, else the BPB label).
    pub fn label(&self) -> &str {
        &self.label
    }
    /// Cluster size in bytes.
    pub fn cluster_bytes(&self) -> u32 {
        self.bps * self.spc
    }
    /// Sets the clock used for new/modified entries (Unix seconds).
    pub fn set_time(&mut self, unix: u64) {
        self.now = unix;
    }
    /// Refuses all mutation when set.
    pub fn set_read_only(&mut self, ro: bool) {
        self.read_only = ro;
    }
    /// Gives the device back (after a final `sync`).
    pub fn into_inner(self) -> D {
        self.dev
    }
    /// Free space in bytes.
    pub fn free_bytes(&self) -> u64 {
        let mut n = 0u64;
        for c in 2..self.clusters + 2 {
            if self.fat_get(c) == 0 {
                n += 1;
            }
        }
        n * self.cluster_bytes() as u64
    }

    // ---------------------------------------------------------------------------- FAT table

    fn fat_get(&self, c: u32) -> u32 {
        match self.kind {
            FatKind::Fat12 => {
                let o = (c as usize * 3) / 2;
                let v = self.fat[o] as u32 | (self.fat[o + 1] as u32) << 8;
                if c & 1 == 0 {
                    v & 0xfff
                } else {
                    v >> 4
                }
            }
            FatKind::Fat16 => le16(&self.fat, c as usize * 2) as u32,
            FatKind::Fat32 => le32(&self.fat, c as usize * 4) & 0x0fff_ffff,
        }
    }

    fn fat_set(&mut self, c: u32, v: u32) {
        self.fat_dirty = true;
        match self.kind {
            FatKind::Fat12 => {
                let o = (c as usize * 3) / 2;
                let old = self.fat[o] as u32 | (self.fat[o + 1] as u32) << 8;
                let new = if c & 1 == 0 { (old & 0xf000) | (v & 0xfff) } else { (old & 0x000f) | ((v & 0xfff) << 4) };
                self.fat[o] = new as u8;
                self.fat[o + 1] = (new >> 8) as u8;
            }
            FatKind::Fat16 => put16(&mut self.fat, c as usize * 2, v as u16),
            FatKind::Fat32 => {
                let old = le32(&self.fat, c as usize * 4);
                put32(&mut self.fat, c as usize * 4, (old & 0xf000_0000) | (v & 0x0fff_ffff));
            }
        }
    }

    fn eoc(&self) -> u32 {
        match self.kind {
            FatKind::Fat12 => 0xfff,
            FatKind::Fat16 => 0xffff,
            FatKind::Fat32 => 0x0fff_ffff,
        }
    }

    fn is_eoc(&self, v: u32) -> bool {
        match self.kind {
            FatKind::Fat12 => v >= 0xff8,
            FatKind::Fat16 => v >= 0xfff8,
            FatKind::Fat32 => v >= 0x0fff_fff8,
        }
    }

    fn valid_cluster(&self, c: u32) -> bool {
        c >= 2 && c < self.clusters + 2
    }

    fn chain(&self, first: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut c = first;
        while c != 0 {
            if !self.valid_cluster(c) || out.len() as u32 > self.clusters {
                return Err(FsError::Corrupt);
            }
            out.push(c);
            let n = self.fat_get(c);
            if self.is_eoc(n) {
                break;
            }
            if n == 0 {
                return Err(FsError::Corrupt);
            }
            c = n;
        }
        Ok(out)
    }

    fn alloc_one(&mut self) -> Result<u32> {
        let total = self.clusters;
        for i in 0..total {
            let c = 2 + (self.next_hint - 2 + i) % total;
            if self.fat_get(c) == 0 {
                let eoc = self.eoc();
                self.fat_set(c, eoc);
                self.next_hint = if c + 1 >= total + 2 { 2 } else { c + 1 };
                return Ok(c);
            }
        }
        Err(FsError::NoSpace)
    }

    fn free_chain(&mut self, first: u32) -> Result<()> {
        for c in self.chain(first)? {
            self.fat_set(c, 0);
        }
        Ok(())
    }

    fn cluster_off(&self, c: u32) -> u64 {
        self.base + ((self.first_data_sector + (c - 2) * self.spc) as u64) * self.bps as u64
    }

    fn read_cluster(&mut self, c: u32, buf: &mut [u8]) -> Result<()> {
        let off = self.cluster_off(c);
        self.dev.read_at(off, buf)
    }

    fn write_cluster(&mut self, c: u32, buf: &[u8]) -> Result<()> {
        let off = self.cluster_off(c);
        self.dev.write_at(off, buf)
    }

    /// Writes the FAT (all copies) and FSInfo and flushes the device.
    pub fn sync(&mut self) -> Result<()> {
        if self.fat_dirty {
            for i in 0..self.nfats {
                let off = self.base + ((self.reserved + i * self.fat_sectors) * self.bps) as u64;
                self.dev.write_at(off, &self.fat)?;
            }
            self.fat_dirty = false;
        }
        if self.kind == FatKind::Fat32 && self.fsinfo_sector != 0 && self.fsinfo_sector < self.reserved {
            let off = self.base + (self.fsinfo_sector * self.bps) as u64;
            let mut s = vec![0u8; self.bps as usize];
            self.dev.read_at(off, &mut s)?;
            if le32(&s, 0) == 0x4161_5252 && le32(&s, 484) == 0x6141_7272 {
                let free = (self.free_bytes() / self.cluster_bytes() as u64) as u32;
                put32(&mut s, 488, free);
                put32(&mut s, 492, self.next_hint);
                self.dev.write_at(off, &s)?;
            }
        }
        self.dev.flush()
    }

    // ---------------------------------------------------------------------------- directories

    fn load_dir(&mut self, first: u32) -> Result<Dir> {
        if first == 0 && self.kind != FatKind::Fat32 {
            let len = (self.root_entries * 32) as usize;
            let mut data = vec![0u8; len];
            let off = self.base + ((self.reserved + self.nfats * self.fat_sectors) * self.bps) as u64;
            self.dev.read_at(off, &mut data)?;
            return Ok(Dir { chain: Vec::new(), data });
        }
        let first = if first == 0 { self.root_cluster } else { first };
        let chain = self.chain(first)?;
        let cb = self.cluster_bytes() as usize;
        let mut data = vec![0u8; chain.len() * cb];
        for (i, &c) in chain.iter().enumerate() {
            self.read_cluster(c, &mut data[i * cb..(i + 1) * cb])?;
        }
        Ok(Dir { chain, data })
    }

    fn store_dir(&mut self, d: &Dir) -> Result<()> {
        if d.chain.is_empty() {
            let off = self.base + ((self.reserved + self.nfats * self.fat_sectors) * self.bps) as u64;
            return self.dev.write_at(off, &d.data);
        }
        let cb = self.cluster_bytes() as usize;
        for (i, &c) in d.chain.iter().enumerate() {
            self.write_cluster(c, &d.data[i * cb..(i + 1) * cb])?;
        }
        Ok(())
    }

    fn dir_grow(&mut self, d: &mut Dir) -> Result<()> {
        if d.chain.is_empty() || d.chain.len() as u64 * self.cluster_bytes() as u64 >= 2 * 1024 * 1024 {
            return Err(FsError::NoSpace); // fixed root, or the 65536-entry directory limit
        }
        let c = self.alloc_one()?;
        let last = *d.chain.last().unwrap();
        self.fat_set(last, c);
        d.chain.push(c);
        let cb = self.cluster_bytes() as usize;
        d.data.resize(d.data.len() + cb, 0);
        Ok(())
    }

    /// Parses live entries; also returns the number of slots used (up to the 0x00 terminator).
    fn parse_dir(&self, data: &[u8]) -> (Vec<Raw>, usize) {
        let mut out = Vec::new();
        let mut lfn: Vec<(u8, [u16; 13])> = Vec::new();
        let mut lfn_sum = 0u8;
        let mut lfn_first = 0usize;
        let slots = data.len() / 32;
        let mut end = slots;
        for i in 0..slots {
            let e = &data[i * 32..i * 32 + 32];
            if e[0] == 0 {
                end = i;
                break;
            }
            if e[0] == 0xe5 {
                lfn.clear();
                continue;
            }
            if e[11] & 0x3f == ATTR_LFN {
                let seq = e[0];
                if seq & 0x40 != 0 {
                    lfn.clear();
                    lfn_sum = e[13];
                    lfn_first = i;
                }
                let mut u = [0u16; 13];
                for (k, off) in [1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30].iter().enumerate() {
                    u[k] = le16(e, *off);
                }
                lfn.push((seq & 0x1f, u));
                continue;
            }
            let mut short = [0u8; 11];
            short.copy_from_slice(&e[..11]);
            if short[0] == 0x05 {
                short[0] = 0xe5;
            }
            let mut name = None;
            if !lfn.is_empty() && lfn_checksum(&{
                let mut s = [0u8; 11];
                s.copy_from_slice(&e[..11]);
                s
            }) == lfn_sum
            {
                // Entries arrive highest sequence first.
                let mut ok = true;
                let n = lfn.len();
                for (idx, (seq, _)) in lfn.iter().enumerate() {
                    if *seq as usize != n - idx {
                        ok = false;
                    }
                }
                if ok {
                    let mut units: Vec<u16> = Vec::new();
                    for (_, u) in lfn.iter().rev() {
                        units.extend_from_slice(u);
                    }
                    if let Some(z) = units.iter().position(|&x| x == 0) {
                        units.truncate(z);
                    }
                    name = Some(
                        core::char::decode_utf16(units.into_iter()).map(|r| r.unwrap_or('\u{fffd}')).collect::<String>(),
                    );
                }
            }
            let first_slot = if name.is_some() { lfn_first } else { i };
            lfn.clear();
            let attr = e[11];
            let nt = e[12];
            let name = name.unwrap_or_else(|| short_to_string(&short, nt));
            let cluster = if self.kind == FatKind::Fat32 {
                le16(e, 26) as u32 | (le16(e, 20) as u32) << 16
            } else {
                le16(e, 26) as u32
            };
            out.push(Raw {
                name,
                short,
                attr,
                nt,
                cluster,
                size: le32(e, 28),
                wtime: le16(e, 22),
                wdate: le16(e, 24),
                first_slot,
                slot: i,
            });
        }
        (out, end)
    }

    fn to_entry(r: &Raw) -> DirEntry {
        DirEntry {
            name: r.name.clone(),
            short_name: short_to_string(&r.short, r.nt),
            kind: if r.attr & ATTR_DIR != 0 { FileKind::Dir } else { FileKind::File },
            size: if r.attr & ATTR_DIR != 0 { 0 } else { r.size },
            attr: r.attr,
            first_cluster: r.cluster,
            mtime: dos_to_unix(r.wdate, r.wtime),
        }
    }

    /// Resolves a path to (directory first cluster, entry); the root has no entry.
    fn resolve(&mut self, path: &str) -> Result<(u32, Option<Raw>)> {
        let comps = split_path(path);
        let mut dir = 0u32;
        let mut entry: Option<Raw> = None;
        for (i, comp) in comps.iter().enumerate() {
            let d = self.load_dir(dir)?;
            let (list, _) = self.parse_dir(&d.data);
            let want = fold(comp);
            let found = list
                .into_iter()
                .filter(|r| r.attr & ATTR_VOLUME == 0)
                .find(|r| fold(&r.name) == want || fold(&short_to_string(&r.short, r.nt)) == want)
                .ok_or(FsError::NotFound)?;
            if i + 1 < comps.len() {
                if found.attr & ATTR_DIR == 0 {
                    return Err(FsError::NotDir);
                }
                dir = found.cluster;
                if dir == 0 && self.kind == FatKind::Fat32 {
                    dir = self.root_cluster;
                }
            }
            entry = Some(found);
        }
        Ok((dir, entry))
    }

    /// Lists a directory (without `.`/`..`).
    pub fn read_dir(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let (_, e) = self.resolve(path)?;
        let first = match &e {
            None => 0,
            Some(r) if r.attr & ATTR_DIR != 0 => r.cluster,
            Some(_) => return Err(FsError::NotDir),
        };
        let d = self.load_dir(first)?;
        Ok(self
            .parse_dir(&d.data)
            .0
            .iter()
            .filter(|r| r.attr & ATTR_VOLUME == 0 && r.short[0] != b'.')
            .map(Self::to_entry)
            .collect())
    }

    /// Metadata of one path.
    pub fn stat(&mut self, path: &str) -> Result<DirEntry> {
        match self.resolve(path)?.1 {
            Some(r) => Ok(Self::to_entry(&r)),
            None => Ok(DirEntry {
                name: String::new(),
                short_name: String::new(),
                kind: FileKind::Dir,
                size: 0,
                attr: ATTR_DIR,
                first_cluster: 0,
                mtime: 0,
            }),
        }
    }

    /// Reads up to `buf.len()` bytes from `offset`; returns the number read.
    pub fn read_file(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let r = self.resolve(path)?.1.ok_or(FsError::IsDir)?;
        if r.attr & ATTR_DIR != 0 {
            return Err(FsError::IsDir);
        }
        let size = r.size as u64;
        if offset >= size {
            return Ok(0);
        }
        let want = core::cmp::min(buf.len() as u64, size - offset) as usize;
        let chain = self.chain(r.cluster)?;
        let cb = self.cluster_bytes() as u64;
        if (chain.len() as u64) * cb < size {
            return Err(FsError::Corrupt);
        }
        let mut done = 0usize;
        let mut tmp = vec![0u8; cb as usize];
        while done < want {
            let pos = offset + done as u64;
            let ci = (pos / cb) as usize;
            let co = (pos % cb) as usize;
            let n = core::cmp::min(cb as usize - co, want - done);
            if co == 0 && n == cb as usize {
                self.read_cluster(chain[ci], &mut buf[done..done + n])?;
            } else {
                self.read_cluster(chain[ci], &mut tmp)?;
                buf[done..done + n].copy_from_slice(&tmp[co..co + n]);
            }
            done += n;
        }
        Ok(done)
    }

    /// Reads a whole file.
    pub fn read_all(&mut self, path: &str) -> Result<Vec<u8>> {
        let st = self.stat(path)?;
        if st.kind == FileKind::Dir {
            return Err(FsError::IsDir);
        }
        let mut v = vec![0u8; st.size as usize];
        let n = self.read_file(path, 0, &mut v)?;
        v.truncate(n);
        Ok(v)
    }

    // ---------------------------------------------------------------------------- mutation

    fn check_rw(&self) -> Result<()> {
        if self.read_only {
            Err(FsError::ReadOnly)
        } else {
            Ok(())
        }
    }

    fn split_parent<'a>(&mut self, path: &'a str) -> Result<(u32, &'a str)> {
        let comps = split_path(path);
        let name = *comps.last().ok_or(FsError::InvalidName)?;
        let parent_len = path.trim_end_matches(|c| c == '/' || c == '\\').len() - name.len();
        let (_, pe) = self.resolve(&path[..parent_len])?;
        let dir = match pe {
            None => 0,
            Some(r) if r.attr & ATTR_DIR != 0 => r.cluster,
            Some(_) => return Err(FsError::NotDir),
        };
        Ok((dir, name))
    }

    fn gen_short(&self, name: &str, taken: &[[u8; 11]]) -> ([u8; 11], u8, bool) {
        if let Some((s, nt)) = exact_short(name) {
            if !taken.contains(&s) {
                return (s, nt, false);
            }
        }
        // Lossy 8.3 with ~N tail.
        let up: String = fold(name);
        let (b, e) = match up.rfind('.') {
            Some(p) if p > 0 => (&up[..p], &up[p + 1..]),
            _ => (&up[..], ""),
        };
        let conv = |s: &str, max: usize| -> Vec<u8> {
            s.chars()
                .filter(|c| *c != ' ' && *c != '.')
                .map(|c| if valid_short_char(c) { c as u8 } else { b'_' })
                .take(max)
                .collect()
        };
        let base = conv(b, 8);
        let ext = conv(e, 3);
        for n in 1..1_000_000u32 {
            let tail = {
                let mut t = String::from("~");
                t.push_str(&alloc::format!("{}", n));
                t
            };
            let keep = core::cmp::min(base.len(), 8 - tail.len());
            let mut s = [b' '; 11];
            s[..keep].copy_from_slice(&base[..keep]);
            s[keep..keep + tail.len()].copy_from_slice(tail.as_bytes());
            s[8..8 + ext.len()].copy_from_slice(&ext);
            if s[0] == 0xe5 {
                s[0] = 0x05;
            }
            if !taken.contains(&s) {
                return (s, 0, true);
            }
        }
        ([b'_'; 11], 0, true)
    }

    fn build_slots(&self, name: &str, short: [u8; 11], nt: u8, need_lfn: bool, attr: u8, cluster: u32, size: u32) -> Vec<[u8; 32]> {
        let (date, time) = unix_to_dos(self.now);
        let mut slots: Vec<[u8; 32]> = Vec::new();
        if need_lfn || name != short_to_string(&short, nt) {
            let mut units: Vec<u16> = name.encode_utf16().collect();
            let n = (units.len() + 12) / 13;
            if units.len() % 13 != 0 {
                units.push(0);
            }
            while units.len() < n * 13 {
                units.push(0xffff);
            }
            let sum = lfn_checksum(&short);
            for k in (0..n).rev() {
                let mut e = [0u8; 32];
                e[0] = (k + 1) as u8 | if k == n - 1 { 0x40 } else { 0 };
                e[11] = ATTR_LFN;
                e[13] = sum;
                for (j, off) in [1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30].iter().enumerate() {
                    put16(&mut e, *off, units[k * 13 + j]);
                }
                slots.push(e);
            }
        }
        let mut e = [0u8; 32];
        e[..11].copy_from_slice(&short);
        e[11] = attr;
        e[12] = nt;
        put16(&mut e, 14, time);
        put16(&mut e, 16, date);
        put16(&mut e, 18, date);
        put16(&mut e, 20, (cluster >> 16) as u16);
        put16(&mut e, 22, time);
        put16(&mut e, 24, date);
        put16(&mut e, 26, cluster as u16);
        put32(&mut e, 28, size);
        slots.push(e);
        slots
    }

    /// Inserts a new entry into directory `dir`; returns its slot index (of the 8.3 slot).
    fn add_entry(&mut self, dir: u32, name: &str, attr: u8, cluster: u32, size: u32) -> Result<()> {
        validate_name(name)?;
        let mut d = self.load_dir(dir)?;
        let (list, end) = self.parse_dir(&d.data);
        let want = fold(name);
        if list.iter().any(|r| r.attr & ATTR_VOLUME == 0 && (fold(&r.name) == want || fold(&short_to_string(&r.short, r.nt)) == want)) {
            return Err(FsError::Exists);
        }
        let taken: Vec<[u8; 11]> = list.iter().map(|r| r.short).collect();
        let (short, nt, lfn) = self.gen_short(name, &taken);
        let slots = self.build_slots(name, short, nt, lfn, attr, cluster, size);
        // First run of deleted slots (or the tail) big enough.
        let total = d.data.len() / 32;
        let mut at = None;
        let mut run = 0usize;
        for i in 0..end {
            let free = d.data[i * 32] == 0xe5;
            if free {
                run += 1;
                if run == slots.len() {
                    at = Some(i + 1 - run);
                    break;
                }
            } else {
                run = 0;
            }
        }
        let at = match at {
            Some(a) => a,
            None => {
                // Append at the end, growing the directory (keep the terminator after us).
                let mut start = end;
                // Trailing deleted slots directly before `end` can be reused as part of the run.
                while start > 0 && d.data[(start - 1) * 32] == 0xe5 {
                    start -= 1;
                }
                while start + slots.len() + 1 > d.data.len() / 32 {
                    if start + slots.len() == d.data.len() / 32 && d.chain.is_empty() {
                        // Fixed root: the terminator slot may be omitted when the directory is exactly full.
                        break;
                    }
                    self.dir_grow(&mut d)?;
                }
                if start + slots.len() > d.data.len() / 32 {
                    return Err(FsError::NoSpace);
                }
                start
            }
        };
        let _ = total;
        for (i, s) in slots.iter().enumerate() {
            d.data[(at + i) * 32..(at + i) * 32 + 32].copy_from_slice(s);
        }
        self.store_dir(&d)
    }

    /// Updates cluster/size/mtime of the entry `name` inside `dir`.
    fn update_entry(&mut self, dir: u32, slot: usize, cluster: u32, size: u32) -> Result<()> {
        let mut d = self.load_dir(dir)?;
        let (date, time) = unix_to_dos(self.now);
        let e = &mut d.data[slot * 32..slot * 32 + 32];
        put16(e, 20, (cluster >> 16) as u16);
        put16(e, 26, cluster as u16);
        put32(e, 28, size);
        put16(e, 22, time);
        put16(e, 24, date);
        put16(e, 18, date);
        self.store_dir(&d)
    }

    /// Creates a directory.
    pub fn mkdir(&mut self, path: &str) -> Result<()> {
        self.check_rw()?;
        let (parent, name) = self.split_parent(path)?;
        validate_name(name)?;
        let c = self.alloc_one()?;
        let cb = self.cluster_bytes() as usize;
        let mut data = vec![0u8; cb];
        let (date, time) = unix_to_dos(self.now);
        for (i, nm) in [b".          ", b"..         "].iter().enumerate() {
            let e = &mut data[i * 32..i * 32 + 32];
            e[..11].copy_from_slice(&nm[..]);
            e[11] = ATTR_DIR;
            put16(e, 14, time);
            put16(e, 16, date);
            put16(e, 18, date);
            put16(e, 22, time);
            put16(e, 24, date);
            let target = if i == 0 { c } else if parent == self.root_cluster && self.kind == FatKind::Fat32 { 0 } else { parent };
            put16(e, 20, (target >> 16) as u16);
            put16(e, 26, target as u16);
        }
        self.write_cluster(c, &data)?;
        if let Err(e) = self.add_entry(parent, name, ATTR_DIR, c, 0) {
            self.fat_set(c, 0);
            return Err(e);
        }
        Ok(())
    }

    /// Creates or replaces a file with `data`.
    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<()> {
        self.check_rw()?;
        match self.resolve(path) {
            Ok((_, Some(r))) => {
                if r.attr & ATTR_DIR != 0 {
                    return Err(FsError::IsDir);
                }
                self.truncate(path, 0)?;
            }
            Ok((_, None)) => return Err(FsError::IsDir),
            Err(FsError::NotFound) => {
                let (parent, name) = self.split_parent(path)?;
                self.add_entry(parent, name, ATTR_ARCHIVE, 0, 0)?;
            }
            Err(e) => return Err(e),
        }
        self.write_at(path, 0, data)
    }

    fn entry_slot(&mut self, path: &str) -> Result<(u32, Raw)> {
        let comps = split_path(path);
        if comps.is_empty() {
            return Err(FsError::IsDir);
        }
        let (parent, _) = self.split_parent(path)?;
        let (_, e) = self.resolve(path)?;
        Ok((parent, e.ok_or(FsError::IsDir)?))
    }

    /// Writes `data` at `offset`, growing the file (gap is zero-filled).
    pub fn write_at(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        self.check_rw()?;
        let (parent, r) = self.entry_slot(path)?;
        if r.attr & ATTR_DIR != 0 {
            return Err(FsError::IsDir);
        }
        let end = offset.checked_add(data.len() as u64).ok_or(FsError::NoSpace)?;
        if end > u32::MAX as u64 {
            return Err(FsError::NoSpace);
        }
        let cb = self.cluster_bytes() as u64;
        let mut chain = self.chain(r.cluster)?;
        let need = ((end.max(r.size as u64) + cb - 1) / cb) as usize;
        let old_len = chain.len();
        // Allocate new clusters (zeroed) as needed.
        while chain.len() < need {
            match self.alloc_one() {
                Ok(c) => {
                    if let Some(&last) = chain.last() {
                        self.fat_set(last, c);
                    }
                    chain.push(c);
                }
                Err(e) => {
                    // roll back
                    if old_len == 0 {
                        for &c in &chain {
                            self.fat_set(c, 0);
                        }
                    } else {
                        let keep = old_len;
                        for &c in &chain[keep..] {
                            self.fat_set(c, 0);
                        }
                        let eoc = self.eoc();
                        self.fat_set(chain[keep - 1], eoc);
                    }
                    return Err(e);
                }
            }
        }
        let zero = vec![0u8; cb as usize];
        for &c in &chain[old_len..] {
            self.write_cluster(c, &zero)?;
        }
        // Zero the gap between the old size and `offset` within old clusters.
        let old_size = r.size as u64;
        if offset > old_size {
            let mut pos = old_size;
            let mut tmp = vec![0u8; cb as usize];
            while pos < offset {
                let ci = (pos / cb) as usize;
                let co = (pos % cb) as usize;
                let n = core::cmp::min(cb as usize - co, (offset - pos) as usize);
                if ci < old_len {
                    self.read_cluster(chain[ci], &mut tmp)?;
                    for b in &mut tmp[co..co + n] {
                        *b = 0;
                    }
                    self.write_cluster(chain[ci], &tmp)?;
                }
                pos += n as u64;
            }
        }
        // Copy payload.
        let mut done = 0usize;
        let mut tmp = vec![0u8; cb as usize];
        while done < data.len() {
            let pos = offset + done as u64;
            let ci = (pos / cb) as usize;
            let co = (pos % cb) as usize;
            let n = core::cmp::min(cb as usize - co, data.len() - done);
            if co == 0 && n == cb as usize {
                self.write_cluster(chain[ci], &data[done..done + n])?;
            } else {
                self.read_cluster(chain[ci], &mut tmp)?;
                tmp[co..co + n].copy_from_slice(&data[done..done + n]);
                self.write_cluster(chain[ci], &tmp)?;
            }
            done += n;
        }
        let new_size = core::cmp::max(r.size as u64, end) as u32;
        let first = chain.first().copied().unwrap_or(0);
        self.update_entry(parent, r.slot, first, new_size)
    }

    /// Sets the file size; growing zero-fills, shrinking frees clusters.
    pub fn truncate(&mut self, path: &str, size: u64) -> Result<()> {
        self.check_rw()?;
        let (parent, r) = self.entry_slot(path)?;
        if r.attr & ATTR_DIR != 0 {
            return Err(FsError::IsDir);
        }
        if size > r.size as u64 {
            return self.write_at(path, size, &[]);
        }
        let cb = self.cluster_bytes() as u64;
        let chain = self.chain(r.cluster)?;
        let keep = ((size + cb - 1) / cb) as usize;
        if keep < chain.len() {
            for &c in &chain[keep..] {
                self.fat_set(c, 0);
            }
            if keep > 0 {
                let eoc = self.eoc();
                self.fat_set(chain[keep - 1], eoc);
            }
        }
        let first = chain.first().copied().filter(|_| keep > 0).unwrap_or(0);
        self.update_entry(parent, r.slot, first, size as u32)
    }

    /// Deletes a file or an empty directory.
    pub fn remove(&mut self, path: &str) -> Result<()> {
        self.check_rw()?;
        let (parent, r) = self.entry_slot(path)?;
        if r.attr & ATTR_DIR != 0 {
            let d = self.load_dir(r.cluster)?;
            if self.parse_dir(&d.data).0.iter().any(|x| x.short[0] != b'.' && x.attr & ATTR_VOLUME == 0) {
                return Err(FsError::NotEmpty);
            }
        }
        self.free_chain(r.cluster)?;
        let mut d = self.load_dir(parent)?;
        for i in r.first_slot..=r.slot {
            d.data[i * 32] = 0xe5;
        }
        self.store_dir(&d)
    }

    /// Renames/moves within the volume (not over an existing target).
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        self.check_rw()?;
        let (src_parent, r) = self.entry_slot(from)?;
        let (dst_parent, dst_name) = self.split_parent(to)?;
        if r.attr & ATTR_DIR != 0 && dst_parent != src_parent {
            // Moving a directory changes its `..` entry; also forbid moving into itself.
            let mut c = dst_parent;
            let mut hops = 0;
            while c != 0 && hops < 4096 {
                if c == r.cluster {
                    return Err(FsError::InvalidName);
                }
                let d = self.load_dir(c)?;
                let up = self.parse_dir(&d.data).0.into_iter().find(|x| &x.short[..2] == b"..").map(|x| x.cluster).unwrap_or(0);
                if up == c {
                    break;
                }
                c = up;
                hops += 1;
            }
        }
        match self.add_entry(dst_parent, dst_name, r.attr, r.cluster, if r.attr & ATTR_DIR != 0 { 0 } else { r.size }) {
            Ok(()) => {}
            Err(e) => return Err(e),
        }
        // Delete the old entry (re-resolve: slots may have shifted if same directory grew — they do not move).
        let mut d = self.load_dir(src_parent)?;
        for i in r.first_slot..=r.slot {
            d.data[i * 32] = 0xe5;
        }
        self.store_dir(&d)?;
        if r.attr & ATTR_DIR != 0 && dst_parent != src_parent {
            let mut nd = self.load_dir(r.cluster)?;
            let up = if dst_parent == self.root_cluster && self.kind == FatKind::Fat32 { 0 } else { dst_parent };
            put16(&mut nd.data, 32 + 20, (up >> 16) as u16);
            put16(&mut nd.data, 32 + 26, up as u16);
            self.store_dir(&nd)?;
        }
        Ok(())
    }
}

