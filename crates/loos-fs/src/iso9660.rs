//! ISO 9660 with Joliet and Rock Ridge (SUSP 1.12, RRIP 1.12) — read-only, plus El Torito lookup.
//!
//! Naming flavour is chosen at mount time: Rock Ridge (POSIX names, modes, symlinks, deep
//! directory relocation) if present, else Joliet (UCS-2), else plain ISO names with the version
//! suffix stripped.
use crate::fat::days_from_civil;
use crate::util::*;
use crate::{BlockDev, Entry, FileKind, FsError, Result};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const SECTOR: u64 = 2048;
const MAX_DIR_BYTES: u64 = 64 * 1024 * 1024;

/// Which naming convention a mount presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    /// Plain ISO 9660 names.
    Plain,
    /// Joliet UCS-2 names.
    Joliet,
    /// Rock Ridge POSIX names and attributes (falls back to plain names where absent).
    RockRidge,
}

/// El Torito boot entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootEntry {
    /// 0 = x86 BIOS, 0xEF = EFI.
    pub platform: u8,
    /// Boot image first sector (2048-byte units).
    pub lba: u32,
    /// Sector count in 512-byte units as declared by the catalog.
    pub sectors: u16,
    /// Media type nibble (0 = no emulation).
    pub media: u8,
    /// True if marked bootable.
    pub bootable: bool,
}

#[derive(Clone, Debug)]
struct Rec {
    extent: u32,
    len: u32,
    flags: u8,
    name: Vec<u8>,
    su: Vec<u8>,
    time: [u8; 7],
    /// Extent list of a multi-extent file (empty for ordinary entries).
    extents: Vec<(u32, u32)>,
}

/// A mounted ISO image.
pub struct Iso<D: BlockDev> {
    dev: D,
    flavor: Flavor,
    joliet: bool,
    root: Rec,
    skip: usize,
    has_rr: bool,
    label: String,
    boot_catalog: Option<u32>,
    total_sectors: u64,
}

fn parse_rec(b: &[u8]) -> Option<Rec> {
    if b.len() < 34 || (b[0] as usize) < 34 || b[0] as usize > b.len() {
        return None;
    }
    let len = b[0] as usize;
    let nlen = b[32] as usize;
    if 33 + nlen > len {
        return None;
    }
    let su_start = 33 + nlen + if nlen % 2 == 0 { 1 } else { 0 };
    let su = if su_start < len { b[su_start..len].to_vec() } else { Vec::new() };
    let mut time = [0u8; 7];
    time.copy_from_slice(&b[18..25]);
    Some(Rec { extent: le32(b, 2), len: le32(b, 10), flags: b[25], name: b[33..33 + nlen].to_vec(), su, time, extents: Vec::new() })
}

fn iso_time(b: &[u8]) -> u64 {
    if b.len() < 7 || b[1] == 0 || b[2] == 0 {
        return 0;
    }
    let days = days_from_civil(1900 + b[0] as i64, b[1] as i64, b[2] as i64);
    let tz = (b[6] as i8) as i64 * 900;
    let t = days * 86400 + b[3] as i64 * 3600 + b[4] as i64 * 60 + b[5] as i64 - tz;
    if t < 0 {
        0
    } else {
        t as u64
    }
}

fn long_time(b: &[u8]) -> u64 {
    // YYYYMMDDHHMMSScc + tz
    let num = |r: core::ops::Range<usize>| -> Option<i64> {
        let mut v = 0i64;
        for &c in &b[r] {
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as i64;
        }
        Some(v)
    };
    if b.len() < 17 {
        return 0;
    }
    match (num(0..4), num(4..6), num(6..8), num(8..10), num(10..12), num(12..14)) {
        (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s)) if y >= 1970 && mo > 0 && d > 0 => {
            let t = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s - (b[16] as i8) as i64 * 900;
            if t < 0 {
                0
            } else {
                t as u64
            }
        }
        _ => 0,
    }
}

/// Rock Ridge fields gathered from a record's system-use area.
#[derive(Default)]
struct Rr {
    name: Option<Vec<u8>>,
    mode: Option<u32>,
    uid: u32,
    gid: u32,
    link: Option<String>,
    mtime: Option<u64>,
    child_link: Option<u32>,
}

impl<D: BlockDev> Iso<D> {
    /// Mounts, choosing the richest flavour available.
    pub fn mount(dev: D) -> Result<Self> {
        Self::mount_with(dev, None)
    }

    /// Mounts with an explicit naming flavour (`None` = best available).
    pub fn mount_with(mut dev: D, want: Option<Flavor>) -> Result<Self> {
        let mut pvd: Option<Vec<u8>> = None;
        let mut svd: Option<Vec<u8>> = None;
        let mut boot_catalog = None;
        let total = dev.len() / SECTOR;
        let mut sec = [0u8; 2048];
        for lba in 16..core::cmp::min(total, 16 + 64) {
            dev.read_at(lba * SECTOR, &mut sec)?;
            if &sec[1..6] != b"CD001" || sec[6] != 1 {
                if lba == 16 {
                    return Err(FsError::NotRecognized);
                }
                continue;
            }
            match sec[0] {
                1 if pvd.is_none() => pvd = Some(sec.to_vec()),
                2 if &sec[88..91] == b"%/@" || &sec[88..91] == b"%/C" || &sec[88..91] == b"%/E" => svd = Some(sec.to_vec()),
                0 if &sec[7..30] == b"EL TORITO SPECIFICATION" => boot_catalog = Some(le32(&sec, 71)),
                255 => break,
                _ => {}
            }
        }
        let pvd = pvd.ok_or(FsError::NotRecognized)?;
        // The logical block size is stored both-endian; this reader only supports 2048 and both halves must agree.
        let (bs_le, bs_be) = (u16::from_le_bytes([pvd[128], pvd[129]]), u16::from_be_bytes([pvd[130], pvd[131]]));
        if bs_le != 2048 || bs_be != 2048 {
            return Err(FsError::Corrupt);
        }
        let plain_root = parse_rec(&pvd[156..190]).ok_or(FsError::Corrupt)?;
        if plain_root.flags & 2 == 0 {
            return Err(FsError::Corrupt);
        }
        // Detect Rock Ridge via the SP entry of the root's own record.
        let mut tmp = Iso {
            dev,
            flavor: Flavor::Plain,
            joliet: false,
            root: plain_root.clone(),
            skip: 0,
            has_rr: false,
            label: String::from_utf8_lossy(&pvd[40..72]).trim_end().into(),
            boot_catalog,
            total_sectors: total,
        };
        // A damaged root directory must not block the mount (the C reader reported it on first lookup); sniffing just fails.
        let dot = tmp.read_dir_recs(&plain_root).unwrap_or_default();
        if let Some(first) = dot.first() {
            let su = &first.su;
            if su.len() >= 7 && &su[0..2] == b"SP" && su[4] == 0xbe && su[5] == 0xef {
                tmp.has_rr = true;
                tmp.skip = su[6] as usize;
            } else if first.su.windows(2).any(|w| w == b"RR") {
                tmp.has_rr = true;
            }
        }
        let flavor = match want {
            Some(f) => f,
            None if tmp.has_rr => Flavor::RockRidge,
            None if svd.is_some() => Flavor::Joliet,
            None => Flavor::Plain,
        };
        match flavor {
            Flavor::RockRidge if !tmp.has_rr => return Err(FsError::Unsupported),
            Flavor::Joliet => {
                let svd = svd.ok_or(FsError::Unsupported)?;
                tmp.root = parse_rec(&svd[156..190]).ok_or(FsError::Corrupt)?;
                tmp.joliet = true;
                tmp.label = utf16be_to_string(&svd[40..72]).trim_end().into();
            }
            _ => {}
        }
        tmp.flavor = flavor;
        Ok(tmp)
    }

    /// Volume identifier.
    pub fn label(&self) -> &str {
        &self.label
    }
    /// Flavour in use.
    pub fn flavor(&self) -> Flavor {
        self.flavor
    }
    /// Image size in 2048-byte sectors.
    pub fn sectors(&self) -> u64 {
        self.total_sectors
    }

    /// Returns El Torito boot entries (validation entry checked, default + section entries).
    pub fn boot_entries(&mut self) -> Result<Vec<BootEntry>> {
        let lba = match self.boot_catalog {
            Some(l) => l,
            None => return Ok(Vec::new()),
        };
        let mut cat = [0u8; 2048];
        self.dev.read_at(lba as u64 * SECTOR, &mut cat)?;
        // Validation entry: header 1, key 55 AA, 16-bit word checksum 0.
        let sum = (0..16).fold(0u16, |a, i| a.wrapping_add(le16(&cat, i * 2)));
        if cat[0] != 1 || cat[30] != 0x55 || cat[31] != 0xaa || sum != 0 {
            return Err(FsError::Corrupt);
        }
        let mut platform = cat[1];
        let mut out = Vec::new();
        let mut i = 32;
        while i + 32 <= cat.len() {
            let e = &cat[i..i + 32];
            if e.iter().all(|&b| b == 0) {
                break;
            }
            match e[0] {
                0x88 | 0x00 => out.push(BootEntry { platform, lba: le32(e, 8), sectors: le16(e, 6), media: e[1] & 0x0f, bootable: e[0] == 0x88 }),
                0x90 | 0x91 => platform = e[1],
                0x44 => {}
                _ => break,
            }
            i += 32;
        }
        Ok(out)
    }

    fn read_extent(&mut self, lba: u32, len: u64, off: u64, buf: &mut [u8]) -> Result<()> {
        if off + buf.len() as u64 > len {
            return Err(FsError::Corrupt);
        }
        let start = (lba as u64).checked_mul(SECTOR).and_then(|v| v.checked_add(off)).ok_or(FsError::Corrupt)?;
        self.dev.read_at(start, buf)
    }

    fn read_dir_recs(&mut self, dir: &Rec) -> Result<Vec<Rec>> {
        if dir.len as u64 > MAX_DIR_BYTES {
            return Err(FsError::Corrupt);
        }
        let mut data = vec![0u8; dir.len as usize];
        self.read_extent(dir.extent, dir.len as u64, 0, &mut data)?;
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < data.len() {
            let l = data[i] as usize;
            if l == 0 {
                // records never straddle sectors: skip to the next one
                i = (i / SECTOR as usize + 1) * SECTOR as usize;
                continue;
            }
            if i + l > data.len() {
                return Err(FsError::Corrupt);
            }
            match parse_rec(&data[i..i + l]) {
                Some(r) => out.push(r),
                None => return Err(FsError::Corrupt),
            }
            i += l;
        }
        Ok(out)
    }

    /// Collects the system-use bytes of a record including `CE` continuation areas.
    fn su_chain(&mut self, rec: &Rec, first_in_dir: bool) -> Result<Vec<u8>> {
        let _ = first_in_dir;
        let mut su = if rec.su.len() > self.skip { rec.su[self.skip..].to_vec() } else { Vec::new() };
        let mut all = Vec::new();
        let mut guard = 0;
        loop {
            let mut next: Option<(u32, u32, u32)> = None;
            let mut i = 0;
            while i + 4 <= su.len() {
                let sig = [su[i], su[i + 1]];
                let elen = su[i + 2] as usize;
                if elen < 4 || i + elen > su.len() {
                    break;
                }
                if &sig == b"CE" && elen >= 28 {
                    next = Some((le32(&su, i + 4), le32(&su, i + 12), le32(&su, i + 20)));
                } else if &sig == b"ST" {
                    break;
                }
                all.extend_from_slice(&su[i..i + elen]);
                i += elen;
            }
            match next {
                Some((blk, off, len)) if guard < 32 && len as u64 <= SECTOR * 16 => {
                    let mut buf = vec![0u8; len as usize];
                    let pos = blk as u64 * SECTOR + off as u64;
                    self.dev.read_at(pos, &mut buf)?;
                    su = buf;
                    guard += 1;
                }
                _ => break,
            }
        }
        Ok(all)
    }

    fn parse_rr(&mut self, rec: &Rec) -> Result<Rr> {
        let su = self.su_chain(rec, false)?;
        let mut rr = Rr::default();
        let mut name: Vec<u8> = Vec::new();
        let mut have_name = false;
        let mut link = String::new();
        let mut have_link = false;
        let mut i = 0;
        while i + 4 <= su.len() {
            let elen = su[i + 2] as usize;
            if elen < 4 || i + elen > su.len() {
                break;
            }
            let e = &su[i..i + elen];
            match &e[0..2] {
                b"NM" if elen >= 5 => {
                    let flags = e[4];
                    if flags & 0x06 == 0 {
                        name.extend_from_slice(&e[5..]);
                        have_name = true;
                    }
                }
                b"PX" if elen >= 36 => {
                    rr.mode = Some(le32(e, 4));
                    rr.uid = le32(e, 20);
                    rr.gid = le32(e, 28);
                }
                b"SL" if elen >= 5 => {
                    have_link = true;
                    let mut j = 5;
                    while j + 2 <= elen {
                        let cf = e[j];
                        let cl = e[j + 1] as usize;
                        if j + 2 + cl > elen {
                            break;
                        }
                        let content = &e[j + 2..j + 2 + cl];
                        if cf & 0x08 != 0 {
                            link.push('/');
                        } else if cf & 0x04 != 0 {
                            if !link.is_empty() && !link.ends_with('/') {
                                link.push('/');
                            }
                            link.push_str("..");
                            if cf & 1 == 0 {
                                link.push('/');
                            }
                        } else if cf & 0x02 != 0 {
                            link.push('.');
                            if cf & 1 == 0 {
                                link.push('/');
                            }
                        } else {
                            link.push_str(&String::from_utf8_lossy(content));
                            if cf & 1 == 0 {
                                link.push('/');
                            }
                        }
                        j += 2 + cl;
                    }
                }
                b"TF" if elen >= 5 => {
                    let flags = e[4];
                    let long = flags & 0x80 != 0;
                    let w = if long { 17 } else { 7 };
                    let mut pos = 5;
                    for bit in 0..7 {
                        if flags & (1 << bit) != 0 {
                            if pos + w > elen {
                                break;
                            }
                            if bit == 1 {
                                rr.mtime = Some(if long { long_time(&e[pos..pos + w]) } else { iso_time(&e[pos..pos + w]) });
                            }
                            pos += w;
                        }
                    }
                }
                b"CL" if elen >= 8 => rr.child_link = Some(le32(e, 4)),
                _ => {}
            }
            i += elen;
        }
        if have_name {
            rr.name = Some(name);
        }
        if have_link {
            while link.len() > 1 && link.ends_with('/') {
                link.pop();
            }
            rr.link = Some(link);
        }
        Ok(rr)
    }

    fn plain_name(&self, name: &[u8]) -> String {
        let mut s: String = if self.joliet { utf16be_to_string(name) } else { name.iter().map(|&b| b as char).collect() };
        if let Some(p) = s.rfind(';') {
            if s[p + 1..].bytes().all(|c| c.is_ascii_digit()) {
                s.truncate(p);
            }
        }
        if s.ends_with('.') && s.len() > 1 {
            s.pop();
        }
        s
    }

    /// Lists a directory record into entries, merging multi-extent files.
    fn entries(&mut self, dir: &Rec) -> Result<Vec<(Entry, Rec)>> {
        let recs = self.read_dir_recs(dir)?;
        let mut out: Vec<(Entry, Rec)> = Vec::new();
        let mut i = 0;
        while i < recs.len() {
            let r = recs[i].clone();
            i += 1;
            if r.name == [0] || r.name == [1] {
                continue;
            }
            let mut rr = Rr::default();
            if self.flavor == Flavor::RockRidge {
                rr = self.parse_rr(&r)?;
            }
            let mut name = match &rr.name {
                Some(n) => String::from_utf8_lossy(n).into_owned(),
                None => self.plain_name(&r.name),
            };
            if name.is_empty() {
                name = self.plain_name(&r.name);
            }
            let mut rec = r.clone();
            let mut is_dir = r.flags & 2 != 0;
            // Relocated deep directory: the placeholder points at the real directory.
            if let Some(cl) = rr.child_link {
                let mut hdr = [0u8; 34];
                self.dev.read_at(cl as u64 * SECTOR, &mut hdr)?;
                let target = parse_rec(&hdr).ok_or(FsError::Corrupt)?;
                rec = Rec { extent: cl, len: target.len, flags: 2, name: r.name.clone(), su: Vec::new(), time: r.time, extents: Vec::new() };
                is_dir = true;
            }
            // Multi-extent: continuation records follow until one without the multi-extent flag.
            let mut extents: Vec<(u32, u32)> = Vec::new();
            if r.flags & 0x80 != 0 {
                extents.push((r.extent, r.len));
                while i < recs.len() {
                    let n = recs[i].clone();
                    i += 1;
                    extents.push((n.extent, n.len));
                    if n.flags & 0x80 == 0 {
                        break;
                    }
                }
            }
            let total = if extents.is_empty() { rec.len as u64 } else { extents.iter().map(|e| e.1 as u64).sum() };
            let mode = rr.mode.unwrap_or(0);
            let (kind, target) = match &rr.link {
                Some(l) => (FileKind::Symlink, Some(l.clone())),
                None if is_dir => (FileKind::Dir, None),
                None => (FileKind::File, None),
            };
            let mtime = rr.mtime.unwrap_or_else(|| iso_time(&r.time));
            let id = rec.extent as u64;
            rec.extents = extents;
            rec.su = Vec::new();
            out.push((
                Entry {
                    name,
                    kind,
                    size: if is_dir { 0 } else { total },
                    mtime,
                    mode: mode & 0o7777,
                    uid: rr.uid,
                    gid: rr.gid,
                    target,
                    id,
                },
                rec,
            ));
        }
        Ok(out)
    }

    fn lookup(&mut self, path: &str) -> Result<(Entry, Rec)> {
        let comps = split_path(path);
        let mut cur = self.root.clone();
        let mut ent: Option<(Entry, Rec)> = None;
        for (i, comp) in comps.iter().enumerate() {
            let list = self.entries(&cur)?;
            let want = fold(comp);
            let found = list
                .iter()
                .find(|(e, _)| e.name == *comp)
                .or_else(|| list.iter().find(|(e, _)| fold(&e.name) == want))
                .cloned()
                .ok_or(FsError::NotFound)?;
            if i + 1 < comps.len() {
                if found.0.kind != FileKind::Dir {
                    return Err(FsError::NotDir);
                }
                cur = found.1.clone();
            }
            ent = Some(found);
        }
        match ent {
            Some(e) => Ok(e),
            None => Ok((
                Entry { name: String::new(), kind: FileKind::Dir, size: 0, mtime: 0, mode: 0o555, uid: 0, gid: 0, target: None, id: self.root.extent as u64 },
                self.root.clone(),
            )),
        }
    }

    /// Lists a directory.
    pub fn read_dir(&mut self, path: &str) -> Result<Vec<Entry>> {
        let (e, rec) = self.lookup(path)?;
        if e.kind != FileKind::Dir {
            return Err(FsError::NotDir);
        }
        Ok(self.entries(&rec)?.into_iter().map(|(e, _)| e).collect())
    }

    /// Metadata of a path.
    pub fn stat(&mut self, path: &str) -> Result<Entry> {
        Ok(self.lookup(path)?.0)
    }

    /// Reads from `offset`; returns bytes read.
    pub fn read_file(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let (e, rec) = self.lookup(path)?;
        match e.kind {
            FileKind::Dir => return Err(FsError::IsDir),
            FileKind::Symlink => return Err(FsError::Unsupported),
            FileKind::File => {}
        }
        if offset >= e.size {
            return Ok(0);
        }
        let want = core::cmp::min(buf.len() as u64, e.size - offset) as usize;
        let list: Vec<(u32, u32)> = if rec.extents.is_empty() { vec![(rec.extent, rec.len)] } else { rec.extents.clone() };
        let mut skip = offset;
        let mut done = 0usize;
        for (lba, len) in list {
            if done >= want {
                break;
            }
            if skip >= len as u64 {
                skip -= len as u64;
                continue;
            }
            let n = core::cmp::min((len as u64 - skip) as usize, want - done);
            self.read_extent(lba, len as u64, skip, &mut buf[done..done + n])?;
            done += n;
            skip = 0;
        }
        Ok(done)
    }

    /// Reads a whole file.
    pub fn read_all(&mut self, path: &str) -> Result<Vec<u8>> {
        let st = self.stat(path)?;
        if st.kind != FileKind::File {
            return Err(if st.kind == FileKind::Dir { FsError::IsDir } else { FsError::Unsupported });
        }
        let mut v = vec![0u8; st.size as usize];
        let n = self.read_file(path, 0, &mut v)?;
        v.truncate(n);
        Ok(v)
    }

    /// Reads raw sectors (for boot images addressed by LBA).
    pub fn read_sectors(&mut self, lba: u32, buf: &mut [u8]) -> Result<()> {
        self.dev.read_at(lba as u64 * SECTOR, buf)
    }
}

