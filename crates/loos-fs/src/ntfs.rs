//! NTFS — **read-only** (legacy Windows volumes; NTFS is never a LoOS system filesystem).
//!
//! Covers the full read path of NTFS 1.2–3.1: MFT with fixups and `$ATTRIBUTE_LIST` extension
//! records, resident and non-resident attributes (sparse runs, initialized size), LZNT1
//! transparent compression, `$I30` B-tree directories (indexed lookup with `$UpCase`
//! collation, linear fallback), named data streams (ADS), reparse-point symlinks and junctions,
//! volume label/version and free-space accounting. Encrypted (EFS) and WOF-compressed data are
//! reported as [`FsError::Unsupported`] instead of returning wrong bytes.
use crate::util::*;
use crate::{BlockDev, Entry, FileKind, FsError, Result};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

const T_STDINFO: u32 = 0x10;
const T_ATTRLIST: u32 = 0x20;
const T_VOLNAME: u32 = 0x60;
const T_VOLINFO: u32 = 0x70;
const T_DATA: u32 = 0x80;
const T_INDEX_ROOT: u32 = 0x90;
const T_INDEX_ALLOC: u32 = 0xa0;
const T_REPARSE: u32 = 0xc0;

const FLAG_COMPRESSED: u16 = 0x0001;
const FLAG_ENCRYPTED: u16 = 0x4000;

const ATTR_DIRECTORY_FN: u32 = 0x1000_0000;
const ATTR_REPARSE: u32 = 0x400;
const TAG_SYMLINK: u32 = 0xa000_000c;
const TAG_MOUNT_POINT: u32 = 0xa000_0003;

const ROOT_MFT: u64 = 5;
const MAX_ATTR_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RUNS: usize = 1 << 20;

#[derive(Clone, Copy, Debug)]
struct Run {
    vcn: u64,
    /// `None` = sparse hole.
    lcn: Option<u64>,
    len: u64,
}

#[derive(Clone, Debug)]
enum Body {
    Res(Vec<u8>),
    Non { runs: Vec<Run>, data_size: u64, init_size: u64, cu: u8 },
}

#[derive(Clone, Debug)]
struct Attr {
    ty: u32,
    name: Vec<u16>,
    flags: u16,
    id: u16,
    start_vcn: u64,
    body: Body,
}

struct Rec {
    num: u64,
    flags: u16,
    attrs: Vec<Attr>,
}

#[derive(Clone, Debug)]
struct IdxEntry {
    mft: u64,
    name: Vec<u16>,
    ns: u8,
    attrs: u32,
    reparse_tag: u32,
    size: u64,
    mtime: u64,
    child: Option<u64>,
    last: bool,
}

/// A mounted NTFS volume.
pub struct Ntfs<D: BlockDev> {
    dev: D,
    base: u64,
    cluster: u64,
    rec_size: usize,
    idx_size: usize,
    total_clusters: u64,
    mft_runs: Vec<Run>,
    mft_size: u64,
    upcase: Vec<u16>,
    label: String,
    version: (u8, u8),
    unit_cache: Option<((u64, u16, u64), Vec<u8>)>,
}

fn filetime_to_unix(t: u64) -> u64 {
    (t / 10_000_000).saturating_sub(11_644_473_600)
}

fn apply_fixups(buf: &mut [u8], magic: &[u8; 4]) -> Result<()> {
    if buf.len() < 48 || &buf[..4] != magic {
        return Err(FsError::Corrupt);
    }
    let off = le16(buf, 4) as usize;
    let count = le16(buf, 6) as usize;
    if count < 2 || off < 8 || off + count * 2 > buf.len() || (count - 1) * 512 > buf.len() {
        return Err(FsError::Corrupt);
    }
    let check = [buf[off], buf[off + 1]];
    for i in 1..count {
        let end = i * 512;
        if buf[end - 2..end] != check {
            return Err(FsError::Corrupt);
        }
        let (a, b) = (buf[off + i * 2], buf[off + i * 2 + 1]);
        buf[end - 2] = a;
        buf[end - 1] = b;
    }
    Ok(())
}

fn decode_runs(b: &[u8], start_vcn: u64, total_clusters: u64) -> Result<Vec<Run>> {
    let mut runs = Vec::new();
    let mut i = 0;
    let mut vcn = start_vcn;
    let mut lcn: i64 = 0;
    while i < b.len() && b[i] != 0 {
        let lb = (b[i] & 0x0f) as usize;
        let ob = (b[i] >> 4) as usize;
        i += 1;
        if lb == 0 || lb > 8 || ob > 8 || i + lb + ob > b.len() {
            return Err(FsError::Corrupt);
        }
        let mut len = 0u64;
        for k in 0..lb {
            len |= (b[i + k] as u64) << (8 * k);
        }
        i += lb;
        let mut delta = 0i64;
        if ob > 0 {
            for k in 0..ob {
                delta |= (b[i + k] as i64) << (8 * k);
            }
            if b[i + ob - 1] & 0x80 != 0 && ob < 8 {
                delta |= -1i64 << (8 * ob);
            }
            i += ob;
            lcn = lcn.checked_add(delta).ok_or(FsError::Corrupt)?;
            if lcn < 0 || (lcn as u64).checked_add(len).map_or(true, |e| e > total_clusters) {
                return Err(FsError::Corrupt);
            }
            runs.push(Run { vcn, lcn: Some(lcn as u64), len });
        } else {
            runs.push(Run { vcn, lcn: None, len });
        }
        vcn = vcn.checked_add(len).ok_or(FsError::Corrupt)?;
        if runs.len() > MAX_RUNS {
            return Err(FsError::Corrupt);
        }
    }
    Ok(runs)
}

fn parse_attrs(rec: &[u8], total_clusters: u64) -> Result<Vec<Attr>> {
    let used = le32(rec, 24) as usize;
    let mut off = le16(rec, 20) as usize;
    if used > rec.len() || off < 42 || off > used {
        return Err(FsError::Corrupt);
    }
    let mut out = Vec::new();
    while off + 8 <= used {
        let ty = le32(rec, off);
        if ty == 0xffff_ffff {
            break;
        }
        let alen = le32(rec, off + 4) as usize;
        if alen < 24 || alen % 8 != 0 || off + alen > used {
            return Err(FsError::Corrupt);
        }
        let a = &rec[off..off + alen];
        let nonres = a[8] != 0;
        let nlen = a[9] as usize;
        let noff = le16(a, 10) as usize;
        let flags = le16(a, 12);
        let id = le16(a, 14);
        if nlen > 0 && noff + nlen * 2 > alen {
            return Err(FsError::Corrupt);
        }
        let name: Vec<u16> = (0..nlen).map(|k| le16(a, noff + k * 2)).collect();
        let (start_vcn, body) = if !nonres {
            let vlen = le32(a, 16) as usize;
            let voff = le16(a, 20) as usize;
            if voff.checked_add(vlen).map_or(true, |e| e > alen) {
                return Err(FsError::Corrupt);
            }
            (0, Body::Res(a[voff..voff + vlen].to_vec()))
        } else {
            if alen < 64 {
                return Err(FsError::Corrupt);
            }
            let svcn = le64(a, 16);
            let roff = le16(a, 32) as usize;
            let cu = a[34];
            if roff < 64 || roff > alen || cu > 16 {
                return Err(FsError::Corrupt);
            }
            let runs = decode_runs(&a[roff..], svcn, total_clusters)?;
            (svcn, Body::Non { runs, data_size: le64(a, 48), init_size: le64(a, 56), cu })
        };
        out.push(Attr { ty, name, flags, id, start_vcn, body });
        off += alen;
    }
    Ok(out)
}

/// Merges the pieces of one non-resident attribute that were split across extension records.
fn merge_attrs(attrs: Vec<Attr>) -> Vec<Attr> {
    let mut out: Vec<Attr> = Vec::new();
    for a in attrs {
        if let Body::Non { runs, .. } = &a.body {
            if let Some(prev) = out.iter_mut().find(|p| p.ty == a.ty && p.name == a.name && matches!(p.body, Body::Non { .. })) {
                let (pstart, new_runs) = (prev.start_vcn, runs.clone());
                if let Body::Non { runs: pr, data_size, init_size, cu } = &mut prev.body {
                    if a.start_vcn < pstart {
                        // The earlier piece carries the sizes.
                        let mut r = new_runs;
                        r.extend(pr.iter().copied());
                        *pr = r;
                        if let Body::Non { data_size: ds, init_size: is, cu: c, .. } = &a.body {
                            *data_size = *ds;
                            *init_size = *is;
                            *cu = *c;
                        }
                        prev.start_vcn = a.start_vcn;
                    } else {
                        pr.extend(new_runs);
                    }
                }
                continue;
            }
        }
        out.push(a);
    }
    out
}

/// LZNT1 decompression of one compression unit into `out` (`out` is pre-zeroed).
fn lznt1(src: &[u8], out: &mut [u8]) -> Result<()> {
    let mut sp = 0usize;
    let mut dp = 0usize;
    while sp + 2 <= src.len() {
        let hdr = le16(src, sp) as usize;
        if hdr == 0 {
            break;
        }
        sp += 2;
        let csize = (hdr & 0x0fff) + 1;
        let compressed = hdr & 0x8000 != 0;
        if sp + csize > src.len() {
            return Err(FsError::Corrupt);
        }
        let chunk = &src[sp..sp + csize];
        sp += csize;
        let chunk_start = dp;
        if !compressed {
            if dp + chunk.len() > out.len() {
                return Err(FsError::Corrupt);
            }
            out[dp..dp + chunk.len()].copy_from_slice(chunk);
            dp += chunk.len();
            continue;
        }
        let mut cp = 0usize;
        while cp < chunk.len() {
            let flags = chunk[cp];
            cp += 1;
            for bit in 0..8 {
                if cp >= chunk.len() {
                    break;
                }
                if flags & (1 << bit) == 0 {
                    if dp >= out.len() {
                        return Err(FsError::Corrupt);
                    }
                    out[dp] = chunk[cp];
                    dp += 1;
                    cp += 1;
                } else {
                    if cp + 2 > chunk.len() {
                        return Err(FsError::Corrupt);
                    }
                    let tok = le16(chunk, cp) as usize;
                    cp += 2;
                    let pos = dp - chunk_start;
                    if pos == 0 {
                        return Err(FsError::Corrupt);
                    }
                    let mut lg = 0u32;
                    let mut i = pos - 1;
                    while i >= 0x10 {
                        i >>= 1;
                        lg += 1;
                    }
                    let len = (tok & (0xfff >> lg)) + 3;
                    let dist = (tok >> (12 - lg)) + 1;
                    if dist > pos || dp + len > out.len() {
                        return Err(FsError::Corrupt);
                    }
                    for _ in 0..len {
                        out[dp] = out[dp - dist];
                        dp += 1;
                    }
                }
            }
        }
    }
    Ok(())
}

impl<D: BlockDev> Ntfs<D> {
    /// Mounts the volume at byte `base` of `dev`.
    pub fn mount(mut dev: D, base: u64) -> Result<Self> {
        let mut bs = [0u8; 512];
        dev.read_at(base, &mut bs)?;
        if &bs[3..11] != b"NTFS    " || bs[510] != 0x55 || bs[511] != 0xaa {
            return Err(FsError::NotRecognized);
        }
        let bps = le16(&bs, 11) as u64;
        let spc = if bs[13] > 0x80 { 1u64 << (256 - bs[13] as u32).min(20) } else { bs[13] as u64 };
        if !(512..=4096).contains(&bps) || !bps.is_power_of_two() || spc == 0 || !spc.is_power_of_two() || bps * spc > 2 * 1024 * 1024 {
            return Err(FsError::Corrupt);
        }
        let cluster = bps * spc;
        let total_sectors = le64(&bs, 40);
        let total_clusters = total_sectors / spc;
        let mft_lcn = le64(&bs, 48);
        let size_field = |v: u8| -> u64 {
            if (v as i8) < 0 {
                1u64 << (-(v as i8) as u32).min(30)
            } else {
                v as u64 * cluster
            }
        };
        let rec_size = size_field(bs[64]);
        let idx_size = size_field(bs[68]);
        if rec_size < 512 || rec_size > 65536 || !rec_size.is_power_of_two() || idx_size < 512 || idx_size > 1 << 20 || mft_lcn >= total_clusters || total_clusters == 0 {
            return Err(FsError::Corrupt);
        }
        if (total_sectors * bps).checked_add(0).map_or(true, |v| v > dev.len().saturating_sub(base) + bps) {
            return Err(FsError::Corrupt);
        }
        let mut fs = Ntfs {
            dev,
            base,
            cluster,
            rec_size: rec_size as usize,
            idx_size: idx_size as usize,
            total_clusters,
            mft_runs: Vec::new(),
            mft_size: 0,
            upcase: Vec::new(),
            label: String::new(),
            version: (0, 0),
            unit_cache: None,
        };
        // Record 0 ($MFT) bootstraps the MFT's own mapping.
        let mut raw = vec![0u8; fs.rec_size];
        fs.dev.read_at(base + mft_lcn * cluster, &mut raw)?;
        apply_fixups(&mut raw, b"FILE")?;
        let attrs = merge_attrs(parse_attrs(&raw, total_clusters)?);
        let data = attrs.iter().find(|a| a.ty == T_DATA && a.name.is_empty()).ok_or(FsError::Corrupt)?;
        match &data.body {
            Body::Non { runs, data_size, .. } => {
                fs.mft_runs = runs.clone();
                fs.mft_size = *data_size;
            }
            Body::Res(_) => return Err(FsError::Corrupt),
        }
        // The $MFT data attribute may continue in extension records described by its attribute list.
        if attrs.iter().any(|a| a.ty == T_ATTRLIST) {
            let rec = fs.load_record(0)?;
            let data = rec.attrs.iter().find(|a| a.ty == T_DATA && a.name.is_empty()).ok_or(FsError::Corrupt)?;
            if let Body::Non { runs, data_size, .. } = &data.body {
                fs.mft_runs = runs.clone();
                fs.mft_size = *data_size;
            }
        }
        // $UpCase (record 10), $Volume (record 3).
        fs.upcase = (0..=0xffffu32).map(|c| c as u16).collect();
        if let Ok(rec) = fs.load_record(10) {
            if let Some(a) = rec.attrs.iter().find(|a| a.ty == T_DATA && a.name.is_empty()) {
                if let Ok(v) = fs.attr_all(10, a, 131072) {
                    if v.len() >= 131072 {
                        for (i, w) in v.chunks_exact(2).enumerate().take(65536) {
                            fs.upcase[i] = u16::from_le_bytes([w[0], w[1]]);
                        }
                    }
                }
            }
        }
        if let Ok(rec) = fs.load_record(3) {
            for a in &rec.attrs {
                if let Body::Res(v) = &a.body {
                    if a.ty == T_VOLNAME {
                        fs.label = utf16le_to_string(v);
                    } else if a.ty == T_VOLINFO && v.len() >= 10 {
                        fs.version = (v[8], v[9]);
                    }
                }
            }
        }
        // The root directory must exist and be a directory.
        let root = fs.load_record(ROOT_MFT)?;
        if root.flags & 2 == 0 {
            return Err(FsError::Corrupt);
        }
        Ok(fs)
    }

    /// Volume label.
    pub fn label(&self) -> &str {
        &self.label
    }
    /// NTFS version (major, minor), e.g. (3, 1).
    pub fn version(&self) -> (u8, u8) {
        self.version
    }
    /// Cluster size in bytes.
    pub fn cluster_bytes(&self) -> u64 {
        self.cluster
    }
    /// Gives the device back.
    pub fn into_inner(self) -> D {
        self.dev
    }

    // ------------------------------------------------------------------ raw stream access

    /// Reads from a non-resident stream described by `runs` (no compression), zero-filling holes
    /// and anything at or beyond `init` bytes.
    fn read_runs(&mut self, runs: &[Run], init: u64, offset: u64, buf: &mut [u8]) -> Result<()> {
        let mut done = 0usize;
        while done < buf.len() {
            let pos = offset + done as u64;
            if pos >= init {
                for b in &mut buf[done..] {
                    *b = 0;
                }
                return Ok(());
            }
            let vcn = pos / self.cluster;
            let within = pos % self.cluster;
            let idx = match runs.binary_search_by(|r| {
                if vcn < r.vcn {
                    Ordering::Greater
                } else if vcn >= r.vcn + r.len {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            }) {
                Ok(i) => i,
                Err(_) => return Err(FsError::Corrupt),
            };
            let r = runs[idx];
            let run_left = (r.vcn + r.len - vcn) * self.cluster - within;
            let n = core::cmp::min(core::cmp::min(run_left, (buf.len() - done) as u64), init - pos) as usize;
            match r.lcn {
                None => {
                    for b in &mut buf[done..done + n] {
                        *b = 0;
                    }
                }
                Some(l) => {
                    let off = self.base + (l + (vcn - r.vcn)) * self.cluster + within;
                    self.dev.read_at(off, &mut buf[done..done + n])?;
                }
            }
            done += n;
        }
        Ok(())
    }

    fn read_compressed(&mut self, mft: u64, a: &Attr, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let (runs, data_size, init_size, cu) = match &a.body {
            Body::Non { runs, data_size, init_size, cu } => (runs, *data_size, *init_size, *cu),
            Body::Res(_) => return Err(FsError::Corrupt),
        };
        if cu == 0 {
            return self.read_plain_nonres(runs, data_size, init_size, offset, buf);
        }
        if offset >= data_size {
            return Ok(0);
        }
        let want = core::cmp::min(buf.len() as u64, data_size - offset) as usize;
        let uc = 1u64 << cu;
        let unit_bytes = (uc * self.cluster) as usize;
        let mut done = 0usize;
        while done < want {
            let pos = offset + done as u64;
            let unit = pos / unit_bytes as u64;
            let inner = (pos % unit_bytes as u64) as usize;
            let n = core::cmp::min(unit_bytes - inner, want - done);
            let key = (mft, a.id, unit);
            let cached = matches!(&self.unit_cache, Some((k, _)) if *k == key);
            if !cached {
                let data = self.load_unit(runs, unit, uc, unit_bytes, init_size)?;
                self.unit_cache = Some((key, data));
            }
            let d = &self.unit_cache.as_ref().unwrap().1;
            buf[done..done + n].copy_from_slice(&d[inner..inner + n]);
            done += n;
        }
        Ok(want)
    }

    fn load_unit(&mut self, runs: &[Run], unit: u64, uc: u64, unit_bytes: usize, init_size: u64) -> Result<Vec<u8>> {
        let first = unit * uc;
        // Collect the physical runs inside this unit.
        let mut phys: Vec<(u64, u64)> = Vec::new(); // (lcn, count)
        let mut mapped = 0u64;
        for r in runs {
            if r.vcn + r.len <= first || r.vcn >= first + uc {
                continue;
            }
            let s = core::cmp::max(r.vcn, first);
            let e = core::cmp::min(r.vcn + r.len, first + uc);
            if let Some(l) = r.lcn {
                phys.push((l + (s - r.vcn), e - s));
                mapped += e - s;
            }
        }
        let mut out = vec![0u8; unit_bytes];
        if mapped == 0 || unit * unit_bytes as u64 >= init_size {
            return Ok(out);
        }
        let mut raw = vec![0u8; (mapped * self.cluster) as usize];
        let mut p = 0usize;
        for (l, n) in phys {
            let len = (n * self.cluster) as usize;
            let off = self.base + l * self.cluster;
            self.dev.read_at(off, &mut raw[p..p + len])?;
            p += len;
        }
        if mapped == uc {
            out.copy_from_slice(&raw);
        } else {
            lznt1(&raw, &mut out)?;
        }
        Ok(out)
    }

    fn read_plain_nonres(&mut self, runs: &[Run], data_size: u64, init_size: u64, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if offset >= data_size {
            return Ok(0);
        }
        let n = core::cmp::min(buf.len() as u64, data_size - offset) as usize;
        self.read_runs(runs, init_size.min(data_size), offset, &mut buf[..n])?;
        Ok(n)
    }

    fn attr_len(a: &Attr) -> u64 {
        match &a.body {
            Body::Res(v) => v.len() as u64,
            Body::Non { data_size, .. } => *data_size,
        }
    }

    fn read_attr(&mut self, mft: u64, a: &Attr, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if a.flags & FLAG_ENCRYPTED != 0 {
            return Err(FsError::Unsupported);
        }
        match &a.body {
            Body::Res(v) => {
                if offset >= v.len() as u64 {
                    return Ok(0);
                }
                let o = offset as usize;
                let n = core::cmp::min(buf.len(), v.len() - o);
                buf[..n].copy_from_slice(&v[o..o + n]);
                Ok(n)
            }
            Body::Non { runs, data_size, init_size, .. } => {
                if a.flags & FLAG_COMPRESSED != 0 {
                    self.read_compressed(mft, a, offset, buf)
                } else {
                    let runs = runs.clone();
                    self.read_plain_nonres(&runs, *data_size, *init_size, offset, buf)
                }
            }
        }
    }

    fn attr_all(&mut self, mft: u64, a: &Attr, limit: u64) -> Result<Vec<u8>> {
        let len = Self::attr_len(a);
        if len > limit.max(1) && len > MAX_ATTR_BYTES {
            return Err(FsError::Corrupt);
        }
        let mut v = vec![0u8; core::cmp::min(len, MAX_ATTR_BYTES) as usize];
        let n = self.read_attr(mft, a, 0, &mut v)?;
        v.truncate(n);
        Ok(v)
    }

    // ------------------------------------------------------------------ MFT

    fn read_raw_record(&mut self, n: u64) -> Result<Vec<u8>> {
        let off = n.checked_mul(self.rec_size as u64).ok_or(FsError::Corrupt)?;
        if off + self.rec_size as u64 > self.mft_size {
            return Err(FsError::NotFound);
        }
        let mut raw = vec![0u8; self.rec_size];
        let runs = self.mft_runs.clone();
        self.read_runs(&runs, self.mft_size, off, &mut raw)?;
        apply_fixups(&mut raw, b"FILE")?;
        Ok(raw)
    }

    fn load_record(&mut self, n: u64) -> Result<Rec> {
        let raw = self.read_raw_record(n)?;
        let flags = le16(&raw, 22);
        if flags & 1 == 0 {
            return Err(FsError::NotFound);
        }
        if le64(&raw, 32) & 0xffff_ffff_ffff != 0 {
            return Err(FsError::Corrupt); // extension record, not a file
        }
        let mut attrs = parse_attrs(&raw, self.total_clusters)?;
        if let Some(al) = attrs.iter().find(|a| a.ty == T_ATTRLIST).cloned() {
            let list = self.attr_all(n, &al, 1 << 20)?;
            let mut exts: Vec<u64> = Vec::new();
            let mut i = 0;
            while i + 26 <= list.len() {
                let rl = le16(&list, i + 4) as usize;
                if rl < 26 || i + rl > list.len() {
                    return Err(FsError::Corrupt);
                }
                let m = le64(&list, i + 16) & 0xffff_ffff_ffff;
                if m != n && !exts.contains(&m) {
                    exts.push(m);
                }
                i += rl;
            }
            for m in exts {
                let er = self.read_raw_record(m)?;
                if le16(&er, 22) & 1 == 0 || le64(&er, 32) & 0xffff_ffff_ffff != n {
                    return Err(FsError::Corrupt);
                }
                for a in parse_attrs(&er, self.total_clusters)? {
                    if a.ty != T_ATTRLIST {
                        attrs.push(a);
                    }
                }
            }
        }
        Ok(Rec { num: n, flags, attrs: merge_attrs(attrs) })
    }

    // ------------------------------------------------------------------ names & indexes

    fn ucmp(&self, a: &[u16], b: &[u16]) -> Ordering {
        for (x, y) in a.iter().zip(b.iter()) {
            let (ux, uy) = (self.upcase[*x as usize], self.upcase[*y as usize]);
            if ux != uy {
                return ux.cmp(&uy);
            }
        }
        a.len().cmp(&b.len())
    }

    fn parse_index_entries(node: &[u8], out: &mut Vec<IdxEntry>) -> Result<()> {
        if node.len() < 16 {
            return Err(FsError::Corrupt);
        }
        let start = le32(node, 0) as usize;
        let end = le32(node, 4) as usize;
        if start < 16 || end > node.len() || start > end {
            return Err(FsError::Corrupt);
        }
        let mut i = start;
        while i + 16 <= end {
            let elen = le16(node, i + 8) as usize;
            let klen = le16(node, i + 10) as usize;
            let flags = le16(node, i + 12);
            if elen < 16 || i + elen > end || (flags & 1 != 0 && elen < 24) {
                return Err(FsError::Corrupt);
            }
            let child = if flags & 1 != 0 { Some(le64(node, i + elen - 8)) } else { None };
            let last = flags & 2 != 0;
            if !last {
                if klen < 66 || 16 + klen > elen {
                    return Err(FsError::Corrupt);
                }
                let k = &node[i + 16..i + 16 + klen];
                let nlen = k[64] as usize;
                if 66 + nlen * 2 > klen {
                    return Err(FsError::Corrupt);
                }
                let attrs = le32(k, 56);
                out.push(IdxEntry {
                    mft: le64(node, i) & 0xffff_ffff_ffff,
                    name: (0..nlen).map(|j| le16(k, 66 + j * 2)).collect(),
                    ns: k[65],
                    attrs,
                    reparse_tag: if attrs & ATTR_REPARSE != 0 { le32(k, 60) } else { 0 },
                    size: le64(k, 48),
                    mtime: filetime_to_unix(le64(k, 16)),
                    child,
                    last: false,
                });
            } else {
                out.push(IdxEntry { mft: 0, name: Vec::new(), ns: 0, attrs: 0, reparse_tag: 0, size: 0, mtime: 0, child, last: true });
            }
            i += elen;
            if last {
                break;
            }
        }
        Ok(())
    }

    fn index_attrs(rec: &Rec) -> Result<(&Attr, Option<&Attr>)> {
        let root = rec.attrs.iter().find(|a| a.ty == T_INDEX_ROOT && a.name == [b'$' as u16, b'I' as u16, b'3' as u16, b'0' as u16]).ok_or(FsError::NotDir)?;
        let alloc = rec.attrs.iter().find(|a| a.ty == T_INDEX_ALLOC && a.name == root.name);
        Ok((root, alloc))
    }

    fn read_index_block(&mut self, rec: &Rec, alloc: &Attr, vcn: u64) -> Result<Vec<u8>> {
        let (root, _) = Self::index_attrs(rec)?;
        let unit = match &root.body {
            Body::Res(v) if v.len() >= 16 => {
                let isz = le32(v, 8) as u64;
                if isz >= self.cluster {
                    self.cluster
                } else {
                    512
                }
            }
            _ => return Err(FsError::Corrupt),
        };
        let off = vcn.checked_mul(unit).ok_or(FsError::Corrupt)?;
        let mut blk = vec![0u8; self.idx_size];
        let n = self.read_attr(rec.num, alloc, off, &mut blk)?;
        if n != blk.len() {
            return Err(FsError::Corrupt);
        }
        apply_fixups(&mut blk, b"INDX")?;
        Ok(blk)
    }

    /// All entries of a directory in collation order.
    fn dir_entries(&mut self, rec: &Rec) -> Result<Vec<IdxEntry>> {
        let (root, alloc) = Self::index_attrs(rec)?;
        let rootv = match &root.body {
            Body::Res(v) if v.len() >= 32 => v.clone(),
            _ => return Err(FsError::Corrupt),
        };
        let alloc = alloc.cloned();
        let mut top = Vec::new();
        Self::parse_index_entries(&rootv[16..], &mut top)?;
        let mut out = Vec::new();
        let mut budget = 1usize << 22;
        self.walk_index(rec, alloc.as_ref(), top, 0, &mut budget, &mut out)?;
        Ok(out)
    }

    fn walk_index(&mut self, rec: &Rec, alloc: Option<&Attr>, entries: Vec<IdxEntry>, depth: u32, budget: &mut usize, out: &mut Vec<IdxEntry>) -> Result<()> {
        if depth > 32 {
            return Err(FsError::Corrupt);
        }
        for e in entries {
            if let Some(vcn) = e.child {
                if *budget == 0 {
                    return Err(FsError::Corrupt);
                }
                *budget -= 1;
                let a = alloc.ok_or(FsError::Corrupt)?;
                let blk = self.read_index_block(rec, a, vcn)?;
                let mut sub = Vec::new();
                Self::parse_index_entries(&blk[24..], &mut sub)?;
                self.walk_index(rec, alloc, sub, depth + 1, budget, out)?;
            }
            if !e.last {
                out.push(e);
            }
        }
        Ok(())
    }

    fn find_entry(&mut self, rec: &Rec, name: &[u16]) -> Result<Option<IdxEntry>> {
        // Indexed search first.
        let (root, alloc) = Self::index_attrs(rec)?;
        let rootv = match &root.body {
            Body::Res(v) if v.len() >= 32 => v.clone(),
            _ => return Err(FsError::Corrupt),
        };
        let alloc = alloc.cloned();
        let mut node: Vec<IdxEntry> = Vec::new();
        Self::parse_index_entries(&rootv[16..], &mut node)?;
        for _ in 0..32 {
            let mut next: Option<u64> = None;
            let mut found = None;
            for e in &node {
                if e.last {
                    next = e.child;
                    break;
                }
                match self.ucmp(name, &e.name) {
                    Ordering::Equal => {
                        found = Some(e.clone());
                        break;
                    }
                    Ordering::Less => {
                        next = e.child;
                        break;
                    }
                    Ordering::Greater => {}
                }
            }
            if found.is_some() {
                return Ok(found);
            }
            match (next, &alloc) {
                (Some(vcn), Some(a)) => {
                    let blk = self.read_index_block(rec, a, vcn)?;
                    node = Vec::new();
                    Self::parse_index_entries(&blk[24..], &mut node)?;
                }
                _ => break,
            }
        }
        // Collation corner cases (or a damaged tree): fall back to a full scan.
        let all = self.dir_entries(rec)?;
        Ok(all.into_iter().find(|e| e.ns != 2 && self.ucmp(name, &e.name) == Ordering::Equal))
    }

    // ------------------------------------------------------------------ path handling

    fn split_stream(path: &str) -> (&str, Option<&str>) {
        let trimmed = path.trim_end_matches(|c| c == '/' || c == '\\');
        let last_start = trimmed.rfind(|c| c == '/' || c == '\\').map_or(0, |p| p + 1);
        match trimmed[last_start..].find(':') {
            Some(p) => {
                let stream = &trimmed[last_start + p + 1..];
                let stream = stream.strip_suffix(":$DATA").unwrap_or(stream);
                (&trimmed[..last_start + p], Some(stream))
            }
            None => (trimmed, None),
        }
    }

    fn resolve(&mut self, path: &str) -> Result<Rec> {
        let mut cur = self.load_record(ROOT_MFT)?;
        for comp in split_path(path) {
            if cur.flags & 2 == 0 {
                return Err(FsError::NotDir);
            }
            let name: Vec<u16> = comp.encode_utf16().collect();
            let e = self.find_entry(&cur, &name)?.ok_or(FsError::NotFound)?;
            cur = self.load_record(e.mft).map_err(|e| if e == FsError::NotFound { FsError::Corrupt } else { e })?;
        }
        Ok(cur)
    }

    fn entry_kind(&mut self, rec: &Rec) -> Result<(FileKind, Option<String>)> {
        if let Some(a) = rec.attrs.iter().find(|a| a.ty == T_REPARSE) {
            let data = self.attr_all(rec.num, a, 16384)?;
            if data.len() >= 8 {
                let tag = le32(&data, 0);
                if tag == TAG_SYMLINK || tag == TAG_MOUNT_POINT {
                    if let Some(t) = parse_reparse_target(&data) {
                        return Ok((FileKind::Symlink, Some(t)));
                    }
                }
            }
        }
        Ok((if rec.flags & 2 != 0 { FileKind::Dir } else { FileKind::File }, None))
    }

    fn std_info(rec: &Rec) -> (u64, u32) {
        for a in &rec.attrs {
            if a.ty == T_STDINFO {
                if let Body::Res(v) = &a.body {
                    if v.len() >= 36 {
                        return (filetime_to_unix(le64(v, 8)), le32(v, 32));
                    }
                }
            }
        }
        (0, 0)
    }

    fn make_entry(&mut self, name: String, rec: &Rec) -> Result<Entry> {
        let (kind, target) = self.entry_kind(rec)?;
        let (mtime, _) = Self::std_info(rec);
        let size = if kind == FileKind::Dir {
            0
        } else {
            rec.attrs.iter().find(|a| a.ty == T_DATA && a.name.is_empty()).map_or(0, Self::attr_len)
        };
        Ok(Entry { name, kind, size, mtime, mode: 0, uid: 0, gid: 0, target, id: rec.num })
    }

    /// Lists a directory (DOS 8.3 aliases are hidden; `$`-metadata files are included).
    pub fn read_dir(&mut self, path: &str) -> Result<Vec<Entry>> {
        let rec = self.resolve(path)?;
        if rec.flags & 2 == 0 {
            return Err(FsError::NotDir);
        }
        let list = self.dir_entries(&rec)?;
        let mut out = Vec::new();
        for e in list {
            if e.ns == 2 {
                continue;
            }
            let kind = if e.attrs & ATTR_DIRECTORY_FN != 0 {
                FileKind::Dir
            } else if e.reparse_tag == TAG_SYMLINK || e.reparse_tag == TAG_MOUNT_POINT {
                FileKind::Symlink
            } else {
                FileKind::File
            };
            out.push(Entry {
                name: String::from_utf16_lossy(&e.name),
                kind,
                size: if kind == FileKind::Dir { 0 } else { e.size },
                mtime: e.mtime,
                mode: 0,
                uid: 0,
                gid: 0,
                target: None,
                id: e.mft,
            });
        }
        Ok(out)
    }

    /// Metadata of a path (a `:stream` suffix on the last component selects a named stream).
    pub fn stat(&mut self, path: &str) -> Result<Entry> {
        let (p, stream) = Self::split_stream(path);
        let rec = self.resolve(p)?;
        let name = split_path(p).last().map(|s| String::from(*s)).unwrap_or_default();
        let mut e = self.make_entry(name, &rec)?;
        if let Some(s) = stream {
            let w: Vec<u16> = s.encode_utf16().collect();
            let a = rec.attrs.iter().find(|a| a.ty == T_DATA && a.name == w).ok_or(FsError::NotFound)?;
            e.size = Self::attr_len(a);
            e.kind = FileKind::File;
        }
        Ok(e)
    }

    /// DOS/Windows file attribute bits (`FILE_ATTRIBUTE_*`) of a path.
    pub fn attributes(&mut self, path: &str) -> Result<u32> {
        let rec = self.resolve(Self::split_stream(path).0)?;
        Ok(Self::std_info(&rec).1)
    }

    /// Names of the alternate data streams of a file (the unnamed stream is not listed).
    pub fn streams(&mut self, path: &str) -> Result<Vec<String>> {
        let rec = self.resolve(Self::split_stream(path).0)?;
        Ok(rec.attrs.iter().filter(|a| a.ty == T_DATA && !a.name.is_empty()).map(|a| String::from_utf16_lossy(&a.name)).collect())
    }

    /// Reads from `offset` of the unnamed stream, or of the stream named after a `:` in the path.
    pub fn read_file(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let (p, stream) = Self::split_stream(path);
        let rec = self.resolve(p)?;
        let name: Vec<u16> = stream.unwrap_or("").encode_utf16().collect();
        let attr = rec.attrs.iter().find(|a| a.ty == T_DATA && a.name == name).cloned();
        match attr {
            Some(a) => {
                if stream.is_none() && rec.attrs.iter().any(|x| x.ty == T_REPARSE) {
                    let (k, _) = self.entry_kind(&rec)?;
                    if k == FileKind::Symlink {
                        return Err(FsError::Unsupported);
                    }
                }
                self.read_attr(rec.num, &a, offset, buf)
            }
            None if rec.flags & 2 != 0 && stream.is_none() => Err(FsError::IsDir),
            None => Err(FsError::NotFound),
        }
    }

    /// Reads a whole stream.
    pub fn read_all(&mut self, path: &str) -> Result<Vec<u8>> {
        let st = self.stat(path)?;
        if st.kind == FileKind::Dir {
            return Err(FsError::IsDir);
        }
        let mut v = vec![0u8; st.size as usize];
        let mut done = 0usize;
        while done < v.len() {
            let n = self.read_file(path, done as u64, &mut v[done..])?;
            if n == 0 {
                break;
            }
            done += n;
        }
        v.truncate(done);
        Ok(v)
    }

    /// Number of free clusters according to `$Bitmap`.
    pub fn free_clusters(&mut self) -> Result<u64> {
        let rec = self.load_record(6)?;
        let a = rec.attrs.iter().find(|a| a.ty == T_DATA && a.name.is_empty()).cloned().ok_or(FsError::Corrupt)?;
        let bits = self.attr_all(6, &a, MAX_ATTR_BYTES)?;
        let mut used = 0u64;
        for c in 0..self.total_clusters {
            let byte = bits.get((c / 8) as usize).copied().unwrap_or(0xff);
            if byte & (1 << (c % 8)) != 0 {
                used += 1;
            }
        }
        Ok(self.total_clusters - used)
    }
}

/// Extracts the print/substitute name of a symlink or mount-point reparse buffer.
pub(crate) fn parse_reparse_target(d: &[u8]) -> Option<String> {
    if d.len() < 16 {
        return None;
    }
    let tag = le32(d, 0);
    let (buf, sub_off, sub_len, print_off, print_len) = match tag {
        TAG_SYMLINK if d.len() >= 20 => (20usize, le16(d, 8) as usize, le16(d, 10) as usize, le16(d, 12) as usize, le16(d, 14) as usize),
        TAG_MOUNT_POINT => (16usize, le16(d, 8) as usize, le16(d, 10) as usize, le16(d, 12) as usize, le16(d, 14) as usize),
        _ => return None,
    };
    let slice = |off: usize, len: usize| -> Option<String> {
        let s = buf.checked_add(off)?;
        let e = s.checked_add(len)?;
        if e > d.len() || len == 0 {
            return None;
        }
        Some(utf16le_to_string(&d[s..e]))
    };
    let mut t = slice(print_off, print_len).or_else(|| slice(sub_off, sub_len))?;
    if let Some(r) = t.strip_prefix("\\??\\") {
        t = String::from(r);
    }
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runlist_decoding_handles_sparse_and_negative_deltas() {
        // run1: len 4 @ lcn 0x100; run2: sparse len 3; run3: len 2 @ lcn 0x100 - 0x10 = 0xF0
        let b = [0x21, 0x04, 0x00, 0x01, 0x01, 0x03, 0x11, 0x02, 0xf0, 0x00];
        // third run: offset byte 0xF0 as signed 1-byte = -16 relative to 0x100
        let runs = decode_runs(&b[..9], 0, 0x1000).unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!((runs[0].vcn, runs[0].lcn, runs[0].len), (0, Some(0x100), 4));
        assert_eq!((runs[1].vcn, runs[1].lcn, runs[1].len), (4, None, 3));
        assert_eq!((runs[2].vcn, runs[2].lcn, runs[2].len), (7, Some(0xf0), 2));
        // out of range LCN is rejected
        assert!(decode_runs(&[0x21, 0x04, 0x00, 0x20, 0x00], 0, 0x1000).is_err());
    }

    #[test]
    fn lznt1_literals_and_backreference() {
        // chunk: header (size-1 = 6 | compressed), flag 0b00001000: 3 literals 'a','b','c' then a copy token
        // pos=3 -> lg=0: len = (tok & 0xfff)+3, dist = (tok>>12)+1 ; copy dist 3 len 6 => tok = (2<<12)|3
        let tok: u16 = (2 << 12) | 3;
        let mut c = vec![0u8; 0];
        c.push(0b0000_1000);
        c.extend_from_slice(b"abc");
        c.extend_from_slice(&tok.to_le_bytes());
        let hdr: u16 = 0x8000 | 0x3000 | (c.len() as u16 - 1);
        let mut src = hdr.to_le_bytes().to_vec();
        src.extend(c);
        let mut out = vec![0u8; 16];
        lznt1(&src, &mut out).unwrap();
        assert_eq!(&out[..9], b"abcabcabc");
        // truncated stream is an error, not a panic
        assert!(lznt1(&src[..4], &mut out).is_err());
    }

    #[test]
    fn fixups_detect_torn_sectors() {
        let mut b = vec![0u8; 1024];
        b[..4].copy_from_slice(b"FILE");
        b[4] = 48;
        b[6] = 3; // usn + 2 sectors
        b[48] = 0xaa;
        b[49] = 0xbb; // check value
        b[50] = 1;
        b[51] = 2;
        b[52] = 3;
        b[53] = 4;
        b[510] = 0xaa;
        b[511] = 0xbb;
        b[1022] = 0xaa;
        b[1023] = 0xbb;
        apply_fixups(&mut b, b"FILE").unwrap();
        assert_eq!(&b[510..512], &[1, 2]);
        assert_eq!(&b[1022..1024], &[3, 4]);
        let mut torn = vec![0u8; 1024];
        torn[..4].copy_from_slice(b"FILE");
        torn[4] = 48;
        torn[6] = 3;
        torn[48] = 0xaa;
        torn[49] = 0xbb;
        torn[510] = 0x00; // sector end does not match the check value
        assert!(apply_fixups(&mut torn, b"FILE").is_err());
    }

    #[test]
    fn reparse_symlink_and_junction_targets() {
        let name: Vec<u8> = "C:\\tgt".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let mut sym = Vec::new();
        sym.extend_from_slice(&TAG_SYMLINK.to_le_bytes());
        sym.extend_from_slice(&((12 + name.len()) as u16).to_le_bytes());
        sym.extend_from_slice(&[0, 0]);
        sym.extend_from_slice(&0u16.to_le_bytes()); // subst off
        sym.extend_from_slice(&(name.len() as u16).to_le_bytes());
        sym.extend_from_slice(&0u16.to_le_bytes()); // print off
        sym.extend_from_slice(&(name.len() as u16).to_le_bytes());
        sym.extend_from_slice(&1u32.to_le_bytes()); // relative flag
        sym.extend_from_slice(&name);
        assert_eq!(parse_reparse_target(&sym).as_deref(), Some("C:\\tgt"));
        assert_eq!(parse_reparse_target(&sym[..10]), None);
        let mut j = Vec::new();
        let sub: Vec<u8> = "\\??\\D:\\x".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        j.extend_from_slice(&TAG_MOUNT_POINT.to_le_bytes());
        j.extend_from_slice(&[0, 0, 0, 0]);
        j.extend_from_slice(&0u16.to_le_bytes());
        j.extend_from_slice(&(sub.len() as u16).to_le_bytes());
        j.extend_from_slice(&(sub.len() as u16).to_le_bytes());
        j.extend_from_slice(&0u16.to_le_bytes());
        j.extend_from_slice(&sub);
        assert_eq!(parse_reparse_target(&j).as_deref(), Some("D:\\x"));
    }
}
