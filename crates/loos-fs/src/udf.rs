//! UDF (ECMA-167 / OSTA UDF 1.02–2.60) — read-only.
//!
//! Supports type-1 (physical), sparable and metadata partitions, short/long/extended/inline
//! allocation descriptors with continuation extents, File Entries and Extended File Entries,
//! symlinks and POSIX permission mapping. Virtual (VAT, CD-R incremental) partitions are
//! recognised and reported as [`FsError::Unsupported`].
use crate::fat::days_from_civil;
use crate::util::*;
use crate::{BlockDev, Entry, FileKind, FsError, Result};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const MAX_DIR_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
enum PartKind {
    Physical,
    Sparable { packet: u32, map: Vec<(u32, u32)> },
    /// Metadata file extents: (metadata block, host-partition block, block count).
    Metadata { extents: Vec<(u32, u32, u32)>, host: usize },
}

#[derive(Clone, Debug)]
struct Part {
    start: u64,
    #[allow(dead_code)]
    len: u64,
    kind: PartKind,
}

#[derive(Clone, Copy, Debug)]
struct LongAd {
    part: u16,
    lbn: u32,
}

#[derive(Clone, Copy, Debug)]
struct Extent {
    len: u32,
    kind: u8,
    part: u16,
    lbn: u32,
}

#[derive(Clone, Debug)]
struct Inode {
    ftype: u8,
    perms: u32,
    uid: u32,
    gid: u32,
    size: u64,
    mtime: u64,
    extents: Vec<Extent>,
    inline: Option<Vec<u8>>,
}

/// A mounted UDF volume.
pub struct Udf<D: BlockDev> {
    dev: D,
    bs: u64,
    parts: Vec<Part>,
    root: LongAd,
    label: String,
}

fn tag_ok(b: &[u8]) -> Option<u16> {
    if b.len() < 16 {
        return None;
    }
    let mut sum = 0u8;
    for (i, &x) in b[..16].iter().enumerate() {
        if i != 4 {
            sum = sum.wrapping_add(x);
        }
    }
    if sum != b[4] {
        return None;
    }
    Some(le16(b, 0))
}

fn dstring(b: &[u8]) -> String {
    if b.is_empty() {
        return String::new();
    }
    let len = *b.last().unwrap() as usize;
    if len == 0 || len > b.len() - 1 {
        return String::new();
    }
    decode_cs0(&b[..len])
}

/// OSTA compressed unicode (CS0): first byte is the compression id.
fn decode_cs0(b: &[u8]) -> String {
    match b.first() {
        Some(8) | Some(254) => b[1..].iter().map(|&c| c as char).collect(),
        Some(16) | Some(255) => utf16be_to_string(&b[1..b.len() & !1usize | 1]),
        _ => String::new(),
    }
}

fn timestamp(b: &[u8]) -> u64 {
    if b.len() < 12 {
        return 0;
    }
    let tz = {
        let v = (le16(b, 0) & 0x0fff) as i32;
        let v = if v & 0x800 != 0 { v - 0x1000 } else { v };
        if v == -2047 {
            0
        } else {
            v
        }
    };
    let year = le16(b, 2) as i64;
    let (mo, d, h, mi, s) = (b[4] as i64, b[5] as i64, b[6] as i64, b[7] as i64, b[8] as i64);
    if year < 1970 || mo == 0 || d == 0 || mo > 12 || d > 31 {
        return 0;
    }
    let t = days_from_civil(year, mo, d) * 86400 + h * 3600 + mi * 60 + s - tz as i64 * 60;
    if t < 0 {
        0
    } else {
        t as u64
    }
}

fn posix_mode(p: u32) -> u32 {
    let cls = |v: u32| ((v >> 2) & 1) << 2 | ((v >> 1) & 1) << 1 | (v & 1);
    cls(p >> 10) << 6 | cls(p >> 5) << 3 | cls(p)
}

fn parse_ads(kind: u16, area: &[u8], default_part: u16) -> Vec<Extent> {
    let mut out = Vec::new();
    let step = match kind {
        0 => 8,
        1 => 16,
        2 => 20,
        _ => return out,
    };
    let mut i = 0;
    while i + step <= area.len() {
        let l = le32(area, i);
        let len = l & 0x3fff_ffff;
        let ek = (l >> 30) as u8;
        if len == 0 && ek != 3 {
            break;
        }
        let (part, lbn) = match kind {
            0 => (default_part, le32(area, i + 4)),
            1 => (le16(area, i + 8), le32(area, i + 4)),
            _ => (le16(area, i + 16), le32(area, i + 12)),
        };
        out.push(Extent { len, kind: ek, part, lbn });
        i += step;
    }
    out
}

impl<D: BlockDev> Udf<D> {
    /// Mounts a UDF volume (the whole device is one volume).
    pub fn mount(mut dev: D) -> Result<Self> {
        // Locate the anchor volume descriptor pointer for the usual sector sizes.
        let mut found = None;
        for &bs in &[2048u64, 512, 4096, 1024] {
            let mut b = [0u8; 32];
            if dev.read_at(256 * bs, &mut b).is_ok() && tag_ok(&b) == Some(2) {
                found = Some((bs, b));
                break;
            }
        }
        let (bs, avdp) = found.ok_or(FsError::NotRecognized)?;
        let mut vds_ext = vec![(le32(&avdp, 16) as u64, le32(&avdp, 20) as u64), (le32(&avdp, 24) as u64, le32(&avdp, 28) as u64)];
        vds_ext.retain(|e| e.0 > 0 && e.1 > 0);
        let mut result: Result<Self> = Err(FsError::Corrupt);
        let mut dev = Some(dev);
        for (len, loc) in vds_ext {
            let d = dev.take().unwrap();
            match Self::mount_vds(d, bs, loc, len) {
                Ok(fs) => {
                    result = Ok(fs);
                    break;
                }
                Err((d, e)) => {
                    dev = Some(d);
                    result = Err(e);
                }
            }
        }
        result
    }

    fn mount_vds(mut dev: D, bs: u64, loc: u64, len: u64) -> core::result::Result<Self, (D, FsError)> {
        match Self::try_vds(&mut dev, bs, loc, len) {
            Ok((parts_raw, maps, lvd_label, fsd_loc, fsd_part, label_pvd)) => {
                let mut fs = Udf { dev, bs, parts: Vec::new(), root: LongAd { part: 0, lbn: 0 }, label: String::new() };
                match fs.finish_mount(parts_raw, maps, fsd_loc, fsd_part) {
                    Ok(()) => {
                        fs.label = if lvd_label.is_empty() { label_pvd } else { lvd_label };
                        Ok(fs)
                    }
                    Err(e) => Err((fs.dev, e)),
                }
            }
            Err(e) => Err((dev, e)),
        }
    }

    #[allow(clippy::type_complexity)]
    fn try_vds(dev: &mut D, bs: u64, loc: u64, len: u64) -> Result<(Vec<(u16, u64, u64)>, Vec<Vec<u8>>, String, u32, u16, String)> {
        let mut pds: Vec<(u32, u16, u64, u64)> = Vec::new(); // (seq, number, start, length)
        let mut lvd: Option<(u32, Vec<u8>)> = None;
        let mut pvd_label = String::new();
        let mut cur = (loc, len);
        let mut hops = 0;
        let mut sec = vec![0u8; bs as usize];
        'outer: loop {
            let n = (cur.1 / bs).min(256);
            for i in 0..n {
                dev.read_at((cur.0 + i) * bs, &mut sec)?;
                let id = match tag_ok(&sec) {
                    Some(id) => id,
                    None => continue,
                };
                let seq = le32(&sec, 16);
                match id {
                    1 => pvd_label = dstring(&sec[24..56]),
                    3 => {
                        hops += 1;
                        if hops > 8 {
                            return Err(FsError::Corrupt);
                        }
                        // Volume Descriptor Pointer: next VDS extent_ad (length, location) at offset 20.
                        cur = (le32(&sec, 24) as u64, le32(&sec, 20) as u64);
                        continue 'outer;
                    }
                    5 => pds.push((seq, le16(&sec, 22), le32(&sec, 188) as u64, le32(&sec, 192) as u64)),
                    6 => {
                        if lvd.as_ref().map_or(true, |(s, _)| seq >= *s) {
                            lvd = Some((seq, sec.clone()));
                        }
                    }
                    8 => break 'outer,
                    _ => {}
                }
            }
            break;
        }
        let (_, lv) = lvd.ok_or(FsError::Corrupt)?;
        if le32(&lv, 212) as u64 != bs {
            return Err(FsError::Unsupported);
        }
        let nmaps = le32(&lv, 268) as usize;
        let maps_len = le32(&lv, 264) as usize;
        if 440 + maps_len > lv.len() {
            return Err(FsError::Unsupported); // partition maps spilling past one sector are not produced by real tools
        }
        let mut maps = Vec::new();
        let mut off = 440usize;
        for _ in 0..nmaps {
            if off + 2 > lv.len() {
                return Err(FsError::Corrupt);
            }
            let l = lv[off + 1] as usize;
            if l < 2 || off + l > lv.len() {
                return Err(FsError::Corrupt);
            }
            maps.push(lv[off..off + l].to_vec());
            off += l;
        }
        let mut best: Vec<(u16, u64, u64)> = Vec::new();
        let mut seqs: Vec<(u16, u32)> = Vec::new();
        for (seq, num, start, plen) in pds {
            if let Some(i) = best.iter().position(|b| b.0 == num) {
                if seq >= seqs[i].1 {
                    best[i] = (num, start, plen);
                    seqs[i].1 = seq;
                }
            } else {
                best.push((num, start, plen));
                seqs.push((num, seq));
            }
        }
        let fsd_part = le16(&lv, 248 + 8);
        let fsd_loc = le32(&lv, 248 + 4);
        Ok((best, maps, dstring(&lv[84..212]), fsd_loc, fsd_part, pvd_label))
    }

    fn finish_mount(&mut self, pds: Vec<(u16, u64, u64)>, maps: Vec<Vec<u8>>, fsd_loc: u32, fsd_part: u16) -> Result<()> {
        // First pass: physical + sparable need only the partition descriptor.
        let pd_of = |num: u16| pds.iter().find(|p| p.0 == num).copied().ok_or(FsError::Corrupt);
        for m in &maps {
            match m[0] {
                1 => {
                    if m.len() < 6 {
                        return Err(FsError::Corrupt);
                    }
                    let (_, start, len) = pd_of(le16(m, 4))?;
                    self.parts.push(Part { start, len, kind: PartKind::Physical });
                }
                2 => {
                    if m.len() < 40 {
                        return Err(FsError::Corrupt);
                    }
                    let id = &m[5..28];
                    let (_, start, len) = pd_of(le16(m, 38))?;
                    if id.starts_with(b"*UDF Sparable Partition") {
                        let packet = le16(m, 40) as u32;
                        let ntab = m[42] as usize;
                        let tsize = le32(m, 44) as usize;
                        if packet == 0 || !packet.is_power_of_two() || tsize > 1 << 20 || ntab == 0 || m.len() < 48 + ntab * 4 {
                            return Err(FsError::Corrupt);
                        }
                        let mut map = Vec::new();
                        for t in 0..ntab {
                            let sector = le32(m, 48 + t * 4) as u64;
                            let mut raw = vec![0u8; tsize.max(64)];
                            if self.dev.read_at(sector * self.bs, &mut raw).is_err() {
                                continue;
                            }
                            if tag_ok(&raw) != Some(0) || !raw[16..].starts_with(b"\0*UDF Sparing Table") && !raw[17..].starts_with(b"*UDF Sparing Table") {
                                continue;
                            }
                            let n = le16(&raw, 56) as usize;
                            if 64 + n * 8 > raw.len() {
                                continue;
                            }
                            for e in 0..n {
                                let o = le32(&raw, 64 + e * 8);
                                let mp = le32(&raw, 68 + e * 8);
                                if o < 0xffff_fff0 {
                                    map.push((o, mp));
                                }
                            }
                            break;
                        }
                        self.parts.push(Part { start, len, kind: PartKind::Sparable { packet, map } });
                    } else if id.starts_with(b"*UDF Metadata Partition") {
                        self.parts.push(Part { start, len, kind: PartKind::Metadata { extents: Vec::new(), host: 0 } });
                    } else {
                        // VAT (virtual) partitions and unknown types.
                        return Err(FsError::Unsupported);
                    }
                }
                _ => return Err(FsError::Unsupported),
            }
        }
        // Second pass: resolve metadata partitions through their host physical partition.
        for (idx, m) in maps.iter().enumerate() {
            if m[0] == 2 && m[5..28].starts_with(b"*UDF Metadata Partition") {
                let host_num = le16(m, 38);
                // Host = the type-1 map referencing the same partition number.
                let host = maps
                    .iter()
                    .position(|x| x[0] == 1 && le16(x, 4) == host_num)
                    .or_else(|| maps.iter().position(|x| x[0] == 2 && x[5..28].starts_with(b"*UDF Sparable Partition") && le16(x, 38) == host_num))
                    .ok_or(FsError::Corrupt)?;
                let mut file = le32(m, 40);
                let mirror = le32(m, 44);
                let mut ino = self.read_inode_at(host, file);
                if ino.is_err() && mirror != 0xffff_ffff {
                    file = mirror;
                    ino = self.read_inode_at(host, file);
                }
                let ino = ino?;
                let mut extents = Vec::new();
                let mut blk = 0u32;
                let bs = self.bs;
                for e in &ino.extents {
                    let n = ((e.len as u64 + bs - 1) / bs) as u32;
                    if e.kind == 0 {
                        extents.push((blk, e.lbn, n));
                    }
                    blk = blk.checked_add(n).ok_or(FsError::Corrupt)?;
                }
                self.parts[idx].kind = PartKind::Metadata { extents, host };
            }
        }
        let fsd_off = self.phys(fsd_part as usize, fsd_loc)?;
        let mut b = vec![0u8; self.bs as usize];
        self.dev.read_at(fsd_off, &mut b)?;
        if tag_ok(&b) != Some(256) {
            return Err(FsError::Corrupt);
        }
        self.root = LongAd { part: le16(&b, 400 + 8), lbn: le32(&b, 400 + 4) };
        Ok(())
    }

    /// Byte offset of block `lbn` of logical partition `part`.
    fn phys(&self, part: usize, lbn: u32) -> Result<u64> {
        let p = self.parts.get(part).ok_or(FsError::Corrupt)?;
        let blk = match &p.kind {
            PartKind::Physical => p.start + lbn as u64,
            PartKind::Sparable { packet, map } => {
                let base = lbn & !(*packet - 1);
                match map.iter().find(|(o, _)| *o == base) {
                    Some((_, m)) => p.start + *m as u64 + (lbn & (*packet - 1)) as u64,
                    None => p.start + lbn as u64,
                }
            }
            PartKind::Metadata { extents, host } => {
                let e = extents.iter().find(|(m, _, n)| lbn >= *m && lbn < m + n).ok_or(FsError::Corrupt)?;
                let hp = self.parts.get(*host).ok_or(FsError::Corrupt)?;
                return Ok((hp.start + e.1 as u64 + (lbn - e.0) as u64) * self.bs);
            }
        };
        blk.checked_mul(self.bs).ok_or(FsError::Corrupt)
    }

    fn read_inode_at(&mut self, part: usize, lbn: u32) -> Result<Inode> {
        let mut at = (part, lbn);
        for _ in 0..8 {
            let off = self.phys(at.0, at.1)?;
            let mut b = vec![0u8; self.bs as usize];
            self.dev.read_at(off, &mut b)?;
            match tag_ok(&b) {
                Some(260) => {
                    // Indirect entry: follow the ICB at offset 36 (long_ad).
                    at = (le16(&b, 36 + 8) as usize, le32(&b, 36 + 4));
                    continue;
                }
                Some(261) | Some(266) => {}
                _ => return Err(FsError::Corrupt),
            }
            let ext = tag_ok(&b) == Some(266);
            let flags = le16(&b, 16 + 18);
            let ftype = b[16 + 11];
            let (perms, uid, gid, size, mtime_off, l_ea_off, l_ad_off, ad_base): (u32, u32, u32, u64, usize, usize, usize, usize) =
                if ext { (le32(&b, 44), le32(&b, 36), le32(&b, 40), le64(&b, 56), 92, 208, 212, 216) } else { (le32(&b, 44), le32(&b, 36), le32(&b, 40), le64(&b, 56), 84, 168, 172, 176) };
            let l_ea = le32(&b, l_ea_off) as usize;
            let l_ad = le32(&b, l_ad_off) as usize;
            let start = ad_base.checked_add(l_ea).ok_or(FsError::Corrupt)?;
            let end = start.checked_add(l_ad).ok_or(FsError::Corrupt)?;
            if end > b.len() {
                return Err(FsError::Corrupt);
            }
            let kind = flags & 7;
            let mut ino = Inode { ftype, perms, uid, gid, size, mtime: timestamp(&b[mtime_off..mtime_off + 12]), extents: Vec::new(), inline: None };
            if kind == 3 {
                ino.inline = Some(b[start..end].to_vec());
                return Ok(ino);
            }
            let mut exts = parse_ads(kind, &b[start..end], at.0 as u16);
            // continuation extents
            let mut guard = 0;
            let mut i = 0;
            while i < exts.len() {
                if exts[i].kind == 3 {
                    guard += 1;
                    if guard > 64 {
                        return Err(FsError::Corrupt);
                    }
                    let c = exts.remove(i);
                    let n = core::cmp::max(c.len as usize, 24);
                    let mut raw = vec![0u8; ((n as u64 + self.bs - 1) / self.bs * self.bs) as usize];
                    let off = self.phys(c.part as usize, c.lbn)?;
                    self.dev.read_at(off, &mut raw)?;
                    if tag_ok(&raw) != Some(258) {
                        return Err(FsError::Corrupt);
                    }
                    let l = le32(&raw, 20) as usize;
                    if 24 + l > raw.len() {
                        return Err(FsError::Corrupt);
                    }
                    let more = parse_ads(kind, &raw[24..24 + l], at.0 as u16);
                    for (k, m) in more.into_iter().enumerate() {
                        exts.insert(i + k, m);
                    }
                    continue;
                }
                i += 1;
            }
            ino.extents = exts;
            return Ok(ino);
        }
        Err(FsError::Corrupt)
    }

    fn read_inode(&mut self, ad: LongAd) -> Result<Inode> {
        self.read_inode_at(ad.part as usize, ad.lbn)
    }

    fn read_data(&mut self, ino: &Inode, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if offset >= ino.size {
            return Ok(0);
        }
        let want = core::cmp::min(buf.len() as u64, ino.size - offset) as usize;
        if let Some(d) = &ino.inline {
            let o = offset as usize;
            let n = core::cmp::min(want, d.len().saturating_sub(o));
            buf[..n].copy_from_slice(&d[o..o + n]);
            for b in &mut buf[n..want] {
                *b = 0;
            }
            return Ok(want);
        }
        let bs = self.bs;
        let mut done = 0usize;
        let mut base = 0u64;
        for e in &ino.extents {
            if done >= want {
                break;
            }
            let el = e.len as u64;
            if offset + done as u64 >= base + el {
                base += el;
                continue;
            }
            let within = offset + done as u64 - base;
            let n = core::cmp::min((el - within) as usize, want - done);
            if e.kind == 0 {
                // blocks of one extent are contiguous inside the partition (metadata partitions may still remap per block)
                let mut copied = 0usize;
                while copied < n {
                    let pos = within + copied as u64;
                    let blk = (pos / bs) as u32;
                    let bo = pos % bs;
                    let chunk = core::cmp::min((bs - bo) as usize, n - copied);
                    let off = self.phys(e.part as usize, e.lbn + blk)? + bo;
                    self.dev.read_at(off, &mut buf[done + copied..done + copied + chunk])?;
                    copied += chunk;
                }
            } else {
                for b in &mut buf[done..done + n] {
                    *b = 0;
                }
            }
            done += n;
            base += el;
        }
        for b in &mut buf[done..want] {
            *b = 0;
        }
        Ok(want)
    }

    fn read_all_ino(&mut self, ino: &Inode) -> Result<Vec<u8>> {
        if ino.size > MAX_DIR_BYTES && ino.ftype == 4 {
            return Err(FsError::Corrupt);
        }
        let mut v = vec![0u8; ino.size as usize];
        let n = self.read_data(ino, 0, &mut v)?;
        v.truncate(n);
        Ok(v)
    }

    fn entry_of(&mut self, name: String, ad: LongAd) -> Result<(Entry, Inode)> {
        let ino = self.read_inode(ad)?;
        let kind = match ino.ftype {
            4 => FileKind::Dir,
            12 => FileKind::Symlink,
            _ => FileKind::File,
        };
        let target = if kind == FileKind::Symlink { Some(self.link_target(&ino)?) } else { None };
        let e = Entry {
            name,
            kind,
            size: if kind == FileKind::Dir { 0 } else { ino.size },
            mtime: ino.mtime,
            mode: posix_mode(ino.perms),
            uid: if ino.uid == 0xffff_ffff { 0 } else { ino.uid },
            gid: if ino.gid == 0xffff_ffff { 0 } else { ino.gid },
            target,
            id: ((ad.part as u64) << 32) | ad.lbn as u64,
        };
        Ok((e, ino))
    }

    fn link_target(&mut self, ino: &Inode) -> Result<String> {
        let data = self.read_all_ino(ino)?;
        let mut out = String::new();
        let mut i = 0;
        while i + 4 <= data.len() {
            let t = data[i];
            let l = data[i + 1] as usize;
            if i + 4 + l > data.len() {
                return Err(FsError::Corrupt);
            }
            let id = &data[i + 4..i + 4 + l];
            match t {
                2 => out.push('/'),
                3 => {
                    out.push_str("..");
                    out.push('/');
                }
                4 => {
                    out.push('.');
                    out.push('/');
                }
                5 => {
                    out.push_str(&decode_cs0(id));
                    out.push('/');
                }
                _ => {}
            }
            i += 4 + l;
        }
        if out.len() > 1 && out.ends_with('/') {
            out.pop();
        }
        Ok(out)
    }

    fn dir_entries(&mut self, dir: &Inode) -> Result<Vec<(Entry, Inode, LongAd)>> {
        if dir.ftype != 4 {
            return Err(FsError::NotDir);
        }
        let data = self.read_all_ino(dir)?;
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 38 <= data.len() {
            if tag_ok(&data[i..]) != Some(257) {
                break;
            }
            let chars = data[i + 18];
            let l_fi = data[i + 19] as usize;
            let l_iu = le16(&data, i + 36) as usize;
            let total = (38 + l_iu + l_fi + 3) & !3;
            if i + total > data.len() {
                return Err(FsError::Corrupt);
            }
            if chars & 0x0c == 0 {
                let ad = LongAd { part: le16(&data, i + 20 + 8), lbn: le32(&data, i + 20 + 4) };
                let name = decode_cs0(&data[i + 38 + l_iu..i + 38 + l_iu + l_fi]);
                let (e, ino) = self.entry_of(name, ad)?;
                out.push((e, ino, ad));
            }
            i += total;
        }
        Ok(out)
    }

    fn lookup(&mut self, path: &str) -> Result<(Entry, Inode)> {
        let comps = split_path(path);
        let mut cur = self.read_inode(self.root)?;
        let mut last: Option<(Entry, Inode)> = None;
        for (i, comp) in comps.iter().enumerate() {
            let list = self.dir_entries(&cur)?;
            let want = fold(comp);
            let f = list
                .iter()
                .find(|x| x.0.name == *comp)
                .or_else(|| list.iter().find(|x| fold(&x.0.name) == want))
                .cloned()
                .ok_or(FsError::NotFound)?;
            if i + 1 < comps.len() {
                if f.0.kind != FileKind::Dir {
                    return Err(FsError::NotDir);
                }
                cur = f.1.clone();
            }
            last = Some((f.0, f.1));
        }
        match last {
            Some(x) => Ok(x),
            None => {
                let ad = self.root;
                self.entry_of(String::new(), ad)
            }
        }
    }

    /// Volume label (logical volume identifier, else primary volume identifier).
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Lists a directory.
    pub fn read_dir(&mut self, path: &str) -> Result<Vec<Entry>> {
        let (e, ino) = self.lookup(path)?;
        if e.kind != FileKind::Dir {
            return Err(FsError::NotDir);
        }
        Ok(self.dir_entries(&ino)?.into_iter().map(|x| x.0).collect())
    }

    /// Metadata of a path.
    pub fn stat(&mut self, path: &str) -> Result<Entry> {
        Ok(self.lookup(path)?.0)
    }

    /// Reads from `offset`; returns bytes read.
    pub fn read_file(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let (e, ino) = self.lookup(path)?;
        match e.kind {
            FileKind::Dir => Err(FsError::IsDir),
            FileKind::Symlink => Err(FsError::Unsupported),
            FileKind::File => self.read_data(&ino, offset, buf),
        }
    }

    /// Reads a whole file.
    pub fn read_all(&mut self, path: &str) -> Result<Vec<u8>> {
        let (e, ino) = self.lookup(path)?;
        match e.kind {
            FileKind::Dir => Err(FsError::IsDir),
            FileKind::Symlink => Err(FsError::Unsupported),
            FileKind::File => self.read_all_ino(&ino),
        }
    }
}
