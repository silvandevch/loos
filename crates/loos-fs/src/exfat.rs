//! exFAT — read/write.
//!
//! Handles contiguous (`NoFatChain`) and FAT-chained files, the allocation bitmap, the upcase
//! table (compare + name hash), entry-set checksums and directory growth. Files are created
//! contiguous when a big enough run exists and silently convert to a FAT chain when they must
//! grow around an obstacle. The boot region is never modified.
use crate::fat::{dos_to_unix, unix_to_dos, validate_name};
use crate::util::*;
use crate::{BlockDev, FileKind, FsError, Result};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const ATTR_DIR: u16 = 0x10;
const ATTR_ARCHIVE: u16 = 0x20;
const FAT_EOC: u32 = 0xffff_ffff;

/// Directory entry as seen by callers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// Name.
    pub name: String,
    /// File or directory.
    pub kind: FileKind,
    /// Valid data length in bytes.
    pub size: u64,
    /// Raw attribute word.
    pub attr: u16,
    /// First cluster (0 = empty).
    pub first_cluster: u32,
    /// Last-modified time, Unix seconds.
    pub mtime: u64,
}

#[derive(Clone, Debug)]
struct DirRef {
    first: u32,
    nofat: bool,
    size: u64,
    /// Where this directory's own entry set lives (parent, index); `None` for the root.
    loc: Option<(Box<DirRef>, usize)>,
}

struct Dir {
    r: DirRef,
    cl: Vec<u32>,
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
struct Set {
    idx: usize,
    count: usize,
    attr: u16,
    name: String,
    nofat: bool,
    valid: u64,
    len: u64,
    first: u32,
    mtime: u32,
    raw: Vec<u8>,
}

/// A mounted exFAT volume.
pub struct ExFat<D: BlockDev> {
    dev: D,
    base: u64,
    bps_shift: u32,
    spc_shift: u32,
    fat_off: u64,
    heap_off: u64,
    clusters: u32,
    root: u32,
    fat: Vec<u8>,
    fat_dirty: bool,
    bitmap: Vec<u8>,
    bitmap_dirty: bool,
    bitmap_cl: Vec<u32>,
    upcase: Vec<u16>,
    label: String,
    hint: u32,
    now: u64,
    read_only: bool,
}

fn ts_from_unix(t: u64) -> u32 {
    let (d, tm) = unix_to_dos(t);
    (d as u32) << 16 | tm as u32
}

fn set_checksum(set: &[u8]) -> u16 {
    let mut s = 0u16;
    for (i, &b) in set.iter().enumerate() {
        if i == 2 || i == 3 {
            continue;
        }
        s = (if s & 1 != 0 { 0x8000u16 } else { 0 }).wrapping_add(s >> 1).wrapping_add(b as u16);
    }
    s
}

fn name_hash(up: &[u16]) -> u16 {
    let mut s = 0u16;
    for &u in up {
        for b in [u as u8, (u >> 8) as u8] {
            s = (if s & 1 != 0 { 0x8000u16 } else { 0 }).wrapping_add(s >> 1).wrapping_add(b as u16);
        }
    }
    s
}

impl<D: BlockDev> ExFat<D> {
    /// Mounts the volume starting at byte `base`.
    pub fn mount(mut dev: D, base: u64) -> Result<Self> {
        let mut bs = [0u8; 512];
        dev.read_at(base, &mut bs)?;
        if &bs[3..11] != b"EXFAT   " || bs[510] != 0x55 || bs[511] != 0xaa {
            return Err(FsError::NotRecognized);
        }
        let bps_shift = bs[108] as u32;
        let spc_shift = bs[109] as u32;
        if !(9..=12).contains(&bps_shift) || bps_shift + spc_shift > 25 || bs[110] != 1 {
            return Err(FsError::Corrupt);
        }
        let fat_off = le32(&bs, 80) as u64;
        let fat_len = le32(&bs, 84) as u64;
        let heap_off = le32(&bs, 88) as u64;
        let clusters = le32(&bs, 92);
        let root = le32(&bs, 96);
        let vol_len = le64(&bs, 72);
        if clusters == 0 || clusters > 0x0fff_fff5 || root < 2 || root >= clusters + 2 {
            return Err(FsError::Corrupt);
        }
        let heap_end = heap_off + ((clusters as u64) << spc_shift);
        if heap_end > vol_len || (vol_len << bps_shift) > dev.len().saturating_sub(base) {
            return Err(FsError::Corrupt);
        }
        if (fat_len << bps_shift) < (clusters as u64 + 2) * 4 || fat_off + fat_len > heap_off {
            return Err(FsError::Corrupt);
        }
        let mut fat = vec![0u8; ((clusters as usize) + 2) * 4];
        dev.read_at(base + (fat_off << bps_shift), &mut fat)?;
        let mut fs = ExFat {
            dev,
            base,
            bps_shift,
            spc_shift,
            fat_off,
            heap_off,
            clusters,
            root,
            fat,
            fat_dirty: false,
            bitmap: Vec::new(),
            bitmap_dirty: false,
            bitmap_cl: Vec::new(),
            upcase: Vec::new(),
            label: String::new(),
            hint: 2,
            now: 0,
            read_only: false,
        };
        // Root directory: find bitmap, upcase table and label.
        let root = fs.load_dir(&DirRef { first: root, nofat: false, size: 0, loc: None })?;
        let mut bitmap_ent = None;
        let mut up_ent = None;
        let mut i = 0;
        while i + 32 <= root.data.len() {
            let e = &root.data[i..i + 32];
            match e[0] {
                0x00 => break,
                0x81 => bitmap_ent = Some((le32(e, 20), le64(e, 24))),
                0x82 => up_ent = Some((le32(e, 20), le64(e, 24))),
                0x83 => {
                    let n = core::cmp::min(e[1] as usize, 11);
                    fs.label = utf16le_to_string(&e[2..2 + n * 2]);
                }
                _ => {}
            }
            i += 32;
        }
        let (bc, bl) = bitmap_ent.ok_or(FsError::Corrupt)?;
        if bl < (clusters as u64 + 7) / 8 || bl > (1 << 28) {
            return Err(FsError::Corrupt);
        }
        fs.bitmap_cl = fs.run_clusters(bc, true, bl)?;
        fs.bitmap = fs.read_clusters(&fs.bitmap_cl.clone(), bl as usize)?;
        // Upcase table.
        fs.upcase = (0..=0xffffu32).map(|c| c as u16).collect();
        if let Some((uc, ul)) = up_ent {
            if ul > (1 << 20) {
                return Err(FsError::Corrupt);
            }
            let cl = fs.run_clusters(uc, true, ul)?;
            let raw = fs.read_clusters(&cl, ul as usize)?;
            let mut idx = 0usize;
            let mut words = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
            while let Some(w) = words.next() {
                if w == 0xffff {
                    if let Some(skip) = words.next() {
                        idx += skip as usize;
                    }
                } else if idx <= 0xffff {
                    fs.upcase[idx] = w;
                    idx += 1;
                }
            }
        }
        Ok(fs)
    }

    /// Volume label.
    pub fn label(&self) -> &str {
        &self.label
    }
    /// Cluster size in bytes.
    pub fn cluster_bytes(&self) -> u32 {
        1 << (self.bps_shift + self.spc_shift)
    }
    /// Sets the clock for new/modified entries.
    pub fn set_time(&mut self, unix: u64) {
        self.now = unix;
    }
    /// Refuses mutation when set.
    pub fn set_read_only(&mut self, ro: bool) {
        self.read_only = ro;
    }
    /// Gives the device back (after `sync`).
    pub fn into_inner(self) -> D {
        self.dev
    }
    /// Free space in bytes.
    pub fn free_bytes(&self) -> u64 {
        let mut n = 0u64;
        for c in 2..self.clusters + 2 {
            if !self.bit(c) {
                n += 1;
            }
        }
        n * self.cluster_bytes() as u64
    }

    // ------------------------------------------------------------------- allocation / FAT

    fn bit(&self, c: u32) -> bool {
        let b = (c - 2) as usize;
        self.bitmap[b / 8] & (1 << (b % 8)) != 0
    }
    fn set_bit(&mut self, c: u32, v: bool) {
        let b = (c - 2) as usize;
        if v {
            self.bitmap[b / 8] |= 1 << (b % 8);
        } else {
            self.bitmap[b / 8] &= !(1 << (b % 8));
        }
        self.bitmap_dirty = true;
    }
    fn fat_get(&self, c: u32) -> u32 {
        le32(&self.fat, c as usize * 4)
    }
    fn fat_set(&mut self, c: u32, v: u32) {
        put32(&mut self.fat, c as usize * 4, v);
        self.fat_dirty = true;
    }
    fn valid_cluster(&self, c: u32) -> bool {
        c >= 2 && c < self.clusters + 2
    }
    fn cluster_off(&self, c: u32) -> u64 {
        self.base + ((self.heap_off + (((c - 2) as u64) << self.spc_shift)) << self.bps_shift)
    }

    /// Cluster list for a file: contiguous run or FAT chain, limited to the clusters `len` needs.
    fn run_clusters(&self, first: u32, nofat: bool, len: u64) -> Result<Vec<u32>> {
        let cb = self.cluster_bytes() as u64;
        let n = ((len + cb - 1) / cb) as u32;
        if n == 0 {
            return Ok(Vec::new());
        }
        if !self.valid_cluster(first) {
            return Err(FsError::Corrupt);
        }
        let mut out = Vec::with_capacity(n as usize);
        if nofat {
            if first as u64 + n as u64 > self.clusters as u64 + 2 {
                return Err(FsError::Corrupt);
            }
            out.extend(first..first + n);
        } else {
            let mut c = first;
            for _ in 0..n {
                if !self.valid_cluster(c) {
                    return Err(FsError::Corrupt);
                }
                out.push(c);
                c = self.fat_get(c);
            }
        }
        Ok(out)
    }

    /// Follows a FAT chain to its end (directories without a stored length).
    fn chain_to_end(&self, first: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut c = first;
        while self.valid_cluster(c) {
            if out.len() as u32 > self.clusters {
                return Err(FsError::Corrupt);
            }
            out.push(c);
            c = self.fat_get(c);
        }
        if c != FAT_EOC && c < 0xffff_fff8 {
            return Err(FsError::Corrupt);
        }
        Ok(out)
    }

    fn read_clusters(&mut self, cl: &[u32], len: usize) -> Result<Vec<u8>> {
        let cb = self.cluster_bytes() as usize;
        let mut v = vec![0u8; cl.len() * cb];
        for (i, &c) in cl.iter().enumerate() {
            let off = self.cluster_off(c);
            self.dev.read_at(off, &mut v[i * cb..(i + 1) * cb])?;
        }
        v.truncate(len);
        Ok(v)
    }

    fn write_cluster(&mut self, c: u32, buf: &[u8]) -> Result<()> {
        let off = self.cluster_off(c);
        self.dev.write_at(off, buf)
    }

    /// First-fit contiguous run of `n` free clusters.
    fn find_run(&self, n: u32) -> Option<u32> {
        let total = self.clusters;
        if n == 0 || n > total {
            return None;
        }
        let mut run = 0u32;
        let mut start = 0u32;
        for i in 0..total {
            let c = 2 + (self.hint - 2 + i) % total;
            if c == 2 {
                run = 0; // runs may not wrap
            }
            if !self.bit(c) {
                if run == 0 {
                    start = c;
                }
                run += 1;
                if run == n {
                    return Some(start);
                }
            } else {
                run = 0;
            }
        }
        None
    }

    fn alloc_any(&mut self) -> Result<u32> {
        let total = self.clusters;
        for i in 0..total {
            let c = 2 + (self.hint - 2 + i) % total;
            if !self.bit(c) {
                self.set_bit(c, true);
                self.hint = if c + 1 >= total + 2 { 2 } else { c + 1 };
                return Ok(c);
            }
        }
        Err(FsError::NoSpace)
    }

    fn zero_cluster(&mut self, c: u32) -> Result<()> {
        let z = vec![0u8; self.cluster_bytes() as usize];
        self.write_cluster(c, &z)
    }

    /// Grows a file's cluster list to `new_n` (new clusters are zeroed). Returns the updated
    /// (first, nofat, clusters). On failure everything allocated here is released.
    fn grow(&mut self, first: u32, nofat: bool, cl: &[u32], new_n: usize) -> Result<(u32, bool, Vec<u32>)> {
        let mut cl: Vec<u32> = cl.to_vec();
        if cl.len() >= new_n {
            return Ok((first, nofat, cl));
        }
        let extra = new_n - cl.len();
        let mut first = first;
        // Fresh file: prefer one contiguous run.
        if cl.is_empty() {
            if let Some(s) = self.find_run(extra as u32) {
                for c in s..s + extra as u32 {
                    self.set_bit(c, true);
                    self.zero_cluster(c)?;
                }
                self.hint = if s + extra as u32 >= self.clusters + 2 { 2 } else { s + extra as u32 };
                return Ok((s, true, (s..s + extra as u32).collect()));
            }
        } else if nofat {
            let last = *cl.last().unwrap();
            let free_after = (1..=extra as u32).all(|i| self.valid_cluster(last + i) && !self.bit(last + i));
            if free_after {
                for i in 1..=extra as u32 {
                    self.set_bit(last + i, true);
                    self.zero_cluster(last + i)?;
                    cl.push(last + i);
                }
                return Ok((first, true, cl));
            }
            // Convert to a FAT chain and continue below.
            for w in cl.windows(2) {
                self.fat_set(w[0], w[1]);
            }
            self.fat_set(*cl.last().unwrap(), FAT_EOC);
        }
        // FAT-chain growth.
        let mut added: Vec<u32> = Vec::new();
        for _ in 0..extra {
            match self.alloc_any() {
                Ok(c) => added.push(c),
                Err(e) => {
                    for &c in &added {
                        self.set_bit(c, false);
                    }
                    return Err(e);
                }
            }
        }
        for &c in &added {
            self.zero_cluster(c)?;
            self.fat_set(c, FAT_EOC);
        }
        if let Some(&last) = cl.last() {
            self.fat_set(last, added[0]);
        } else {
            first = added[0];
        }
        for w in added.windows(2) {
            self.fat_set(w[0], w[1]);
        }
        cl.extend_from_slice(&added);
        Ok((first, false, cl))
    }

    fn release(&mut self, cl: &[u32], nofat: bool) {
        for &c in cl {
            self.set_bit(c, false);
            if !nofat {
                self.fat_set(c, 0);
            }
        }
    }

    /// Writes FAT and bitmap back and flushes.
    pub fn sync(&mut self) -> Result<()> {
        if self.fat_dirty {
            let off = self.base + (self.fat_off << self.bps_shift);
            self.dev.write_at(off, &self.fat)?;
            self.fat_dirty = false;
        }
        if self.bitmap_dirty {
            let cb = self.cluster_bytes() as usize;
            let cl = self.bitmap_cl.clone();
            let bm = self.bitmap.clone();
            for (i, &c) in cl.iter().enumerate() {
                let start = i * cb;
                let end = core::cmp::min(start + cb, bm.len());
                let off = self.cluster_off(c);
                self.dev.write_at(off, &bm[start..end])?;
            }
            self.bitmap_dirty = false;
        }
        self.dev.flush()
    }

    // ------------------------------------------------------------------- directories

    fn load_dir(&mut self, r: &DirRef) -> Result<Dir> {
        let cl = if r.loc.is_none() {
            self.chain_to_end(r.first)?
        } else {
            self.run_clusters(r.first, r.nofat, r.size)?
        };
        let data = self.read_clusters(&cl, usize::MAX)?;
        Ok(Dir { r: r.clone(), cl, data })
    }

    fn store_dir(&mut self, d: &Dir) -> Result<()> {
        let cb = self.cluster_bytes() as usize;
        for (i, &c) in d.cl.iter().enumerate() {
            self.write_cluster(c, &d.data[i * cb..(i + 1) * cb])?;
        }
        Ok(())
    }

    fn upcase_units(&self, s: &str) -> Vec<u16> {
        s.encode_utf16().map(|u| self.upcase[u as usize]).collect()
    }

    fn parse_sets(&self, data: &[u8]) -> Vec<Set> {
        let mut out = Vec::new();
        let mut i = 0usize;
        let n = data.len() / 32;
        while i < n {
            let e = &data[i * 32..i * 32 + 32];
            if e[0] == 0 {
                break;
            }
            if e[0] != 0x85 {
                i += 1;
                continue;
            }
            let secondary = e[1] as usize;
            if secondary < 2 || i + 1 + secondary > n {
                i += 1;
                continue;
            }
            let raw = data[i * 32..(i + 1 + secondary) * 32].to_vec();
            let st = &raw[32..64];
            if st[0] != 0xc0 || le16(&raw, 2) != set_checksum(&raw) {
                i += 1 + secondary;
                continue;
            }
            let namelen = st[3] as usize;
            let mut units: Vec<u16> = Vec::new();
            let mut ok = true;
            for k in 0..secondary - 1 {
                let ne = &raw[64 + k * 32..96 + k * 32];
                if ne[0] != 0xc1 {
                    ok = false;
                    break;
                }
                for j in 0..15 {
                    units.push(le16(ne, 2 + j * 2));
                }
            }
            if !ok || namelen > units.len() || namelen == 0 {
                i += 1 + secondary;
                continue;
            }
            units.truncate(namelen);
            let name = core::char::decode_utf16(units.into_iter()).map(|r| r.unwrap_or('\u{fffd}')).collect();
            out.push(Set {
                idx: i,
                count: secondary + 1,
                attr: le16(&raw, 4),
                name,
                nofat: st[1] & 2 != 0,
                valid: le64(st, 8),
                len: le64(st, 24),
                first: le32(st, 20),
                mtime: le32(&raw, 12),
                raw,
            });
            i += 1 + secondary;
        }
        out
    }

    fn to_entry(s: &Set) -> DirEntry {
        DirEntry {
            name: s.name.clone(),
            kind: if s.attr & ATTR_DIR != 0 { FileKind::Dir } else { FileKind::File },
            size: if s.attr & ATTR_DIR != 0 { 0 } else { s.valid },
            attr: s.attr,
            first_cluster: s.first,
            mtime: dos_to_unix((s.mtime >> 16) as u16, s.mtime as u16),
        }
    }

    fn root_ref(&self) -> DirRef {
        DirRef { first: self.root, nofat: false, size: 0, loc: None }
    }

    fn dirref_of(&self, parent: &DirRef, s: &Set) -> DirRef {
        DirRef { first: s.first, nofat: s.nofat, size: s.len, loc: Some((Box::new(parent.clone()), s.idx)) }
    }

    /// Walks `path`; returns the directory holding the final component and its set (None = root).
    fn resolve(&mut self, path: &str) -> Result<(DirRef, Option<Set>)> {
        let comps = split_path(path);
        let mut dir = self.root_ref();
        let mut entry: Option<Set> = None;
        for (i, comp) in comps.iter().enumerate() {
            let d = self.load_dir(&dir)?;
            let want = self.upcase_units(comp);
            let found = self
                .parse_sets(&d.data)
                .into_iter()
                .find(|s| self.upcase_units(&s.name) == want)
                .ok_or(FsError::NotFound)?;
            if i + 1 < comps.len() {
                if found.attr & ATTR_DIR == 0 {
                    return Err(FsError::NotDir);
                }
                dir = self.dirref_of(&dir, &found);
            }
            entry = Some(found);
        }
        Ok((dir, entry))
    }

    /// Lists a directory.
    pub fn read_dir(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let (parent, e) = self.resolve(path)?;
        let dr = match &e {
            None => self.root_ref(),
            Some(s) if s.attr & ATTR_DIR != 0 => self.dirref_of(&parent, s),
            Some(_) => return Err(FsError::NotDir),
        };
        let d = self.load_dir(&dr)?;
        Ok(self.parse_sets(&d.data).iter().map(Self::to_entry).collect())
    }

    /// Metadata of a path.
    pub fn stat(&mut self, path: &str) -> Result<DirEntry> {
        match self.resolve(path)?.1 {
            Some(s) => Ok(Self::to_entry(&s)),
            None => Ok(DirEntry { name: String::new(), kind: FileKind::Dir, size: 0, attr: ATTR_DIR, first_cluster: self.root, mtime: 0 }),
        }
    }

    /// Reads from `offset`; returns bytes read (holes past the valid data length read as zeros).
    pub fn read_file(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let s = self.resolve(path)?.1.ok_or(FsError::IsDir)?;
        if s.attr & ATTR_DIR != 0 {
            return Err(FsError::IsDir);
        }
        let size = s.len;
        if offset >= size {
            return Ok(0);
        }
        let want = core::cmp::min(buf.len() as u64, size - offset) as usize;
        let cl = self.run_clusters(s.first, s.nofat, s.len)?;
        let cb = self.cluster_bytes() as u64;
        let mut done = 0usize;
        let mut tmp = vec![0u8; cb as usize];
        while done < want {
            let pos = offset + done as u64;
            let ci = (pos / cb) as usize;
            let co = (pos % cb) as usize;
            let n = core::cmp::min(cb as usize - co, want - done);
            if pos >= s.valid {
                for b in &mut buf[done..done + n] {
                    *b = 0;
                }
            } else {
                let off = self.cluster_off(cl[ci]);
                self.dev.read_at(off, &mut tmp)?;
                buf[done..done + n].copy_from_slice(&tmp[co..co + n]);
                let vd = s.valid.saturating_sub(pos) as usize;
                if vd < n {
                    for b in &mut buf[done + vd..done + n] {
                        *b = 0;
                    }
                }
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

    /// Offline consistency check: every allocated cluster must be reachable exactly once and every
    /// reachable cluster must be marked in the bitmap. Returns human-readable problems (empty = clean).
    pub fn check(&mut self) -> Result<Vec<String>> {
        let mut problems = Vec::new();
        let mut owner = vec![0u8; self.clusters as usize + 2];
        let mut stack = vec![self.root_ref()];
        let mut claim = |c: u32, what: &str, problems: &mut Vec<String>| {
            if (c as usize) < owner.len() {
                if owner[c as usize] != 0 {
                    problems.push(alloc::format!("cluster {} claimed twice ({})", c, what));
                }
                owner[c as usize] = 1;
            }
        };
        for &c in &self.bitmap_cl.clone() {
            claim(c, "bitmap", &mut problems);
        }
        while let Some(dr) = stack.pop() {
            let d = self.load_dir(&dr)?;
            for &c in &d.cl {
                claim(c, "directory", &mut problems);
            }
            let mut i = 0;
            while i + 32 <= d.data.len() && d.data[i] != 0 {
                if d.data[i] == 0x82 && dr.loc.is_none() {
                    let (uc, ul) = (le32(&d.data, i + 20), le64(&d.data, i + 24));
                    for c in self.run_clusters(uc, true, ul)? {
                        claim(c, "upcase", &mut problems);
                    }
                }
                i += 32;
            }
            for s in self.parse_sets(&d.data) {
                if s.attr & ATTR_DIR != 0 {
                    stack.push(self.dirref_of(&dr, &s));
                } else {
                    for c in self.run_clusters(s.first, s.nofat, s.len)? {
                        claim(c, &s.name, &mut problems);
                    }
                }
            }
        }
        for c in 2..self.clusters + 2 {
            let used = owner[c as usize] != 0;
            if used != self.bit(c) {
                problems.push(alloc::format!("cluster {}: bitmap={} reachable={}", c, self.bit(c), used));
            }
        }
        Ok(problems)
    }

    // ------------------------------------------------------------------- mutation

    fn check_rw(&self) -> Result<()> {
        if self.read_only {
            Err(FsError::ReadOnly)
        } else {
            Ok(())
        }
    }

    fn split_parent<'a>(&mut self, path: &'a str) -> Result<(DirRef, &'a str)> {
        let comps = split_path(path);
        let name = *comps.last().ok_or(FsError::InvalidName)?;
        let plen = path.trim_end_matches(|c| c == '/' || c == '\\').len() - name.len();
        let (pp, pe) = self.resolve(&path[..plen])?;
        let dir = match pe {
            None => self.root_ref(),
            Some(s) if s.attr & ATTR_DIR != 0 => self.dirref_of(&pp, &s),
            Some(_) => return Err(FsError::NotDir),
        };
        Ok((dir, name))
    }

    /// Rewrites the entry set at `idx` of directory `dir` with patched fields and a fresh checksum.
    fn patch_set(&mut self, dir: &DirRef, set: &Set, f: impl FnOnce(&mut [u8])) -> Result<()> {
        let mut d = self.load_dir(dir)?;
        let mut raw = set.raw.clone();
        f(&mut raw);
        let c = set_checksum(&raw);
        put16(&mut raw, 2, c);
        d.data[set.idx * 32..set.idx * 32 + raw.len()].copy_from_slice(&raw);
        self.store_dir(&d)
    }

    fn build_set(&self, name: &str, attr: u16, first: u32, nofat: bool, len: u64) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let nn = (units.len() + 14) / 15;
        let mut raw = vec![0u8; (2 + nn) * 32];
        raw[0] = 0x85;
        raw[1] = (1 + nn) as u8;
        put16(&mut raw, 4, attr);
        let ts = ts_from_unix(self.now);
        put32(&mut raw, 8, ts);
        put32(&mut raw, 12, ts);
        put32(&mut raw, 16, ts);
        raw[22] = 0x80; // UTC offset valid, +0
        raw[23] = 0x80;
        raw[24] = 0x80;
        raw[32] = 0xc0;
        raw[33] = 0x01 | if nofat { 0x02 } else { 0 };
        raw[35] = units.len() as u8;
        put16(&mut raw, 36, name_hash(&self.upcase_units(name)));
        put64(&mut raw, 40, len);
        put32(&mut raw, 52, first);
        put64(&mut raw, 56, len);
        for k in 0..nn {
            let e = &mut raw[64 + k * 32..96 + k * 32];
            e[0] = 0xc1;
            for j in 0..15 {
                let u = units.get(k * 15 + j).copied().unwrap_or(0);
                put16(e, 2 + j * 2, u);
            }
        }
        let c = set_checksum(&raw);
        put16(&mut raw, 2, c);
        raw
    }

    /// Grows a directory by one cluster and records the new size in its parent's entry set.
    fn grow_dir(&mut self, d: &mut Dir) -> Result<()> {
        let new_n = d.cl.len() + 1;
        if new_n as u64 * self.cluster_bytes() as u64 > 256 * 1024 * 1024 {
            return Err(FsError::NoSpace);
        }
        let (first, nofat, cl) = self.grow(d.r.first, d.r.nofat, &d.cl, new_n)?;
        let cb = self.cluster_bytes() as usize;
        d.data.resize(cl.len() * cb, 0);
        d.cl = cl;
        d.r.first = first;
        d.r.nofat = nofat;
        d.r.size = (d.cl.len() * cb) as u64;
        if let Some((parent, idx)) = d.r.loc.clone() {
            let pd = self.load_dir(&parent)?;
            let sets = self.parse_sets(&pd.data);
            let s = sets.into_iter().find(|s| s.idx == idx).ok_or(FsError::Corrupt)?;
            let size = d.r.size;
            self.patch_set(&parent, &s, |raw| {
                raw[33] = 0x01 | if nofat { 0x02 } else { 0 };
                put64(raw, 40, size);
                put32(raw, 52, first);
                put64(raw, 56, size);
            })?;
        } else {
            // Root: FAT chain only; `first` cannot change for a non-empty chain.
        }
        Ok(())
    }

    fn insert_set(&mut self, dir: &DirRef, raw: &[u8]) -> Result<()> {
        let mut d = self.load_dir(dir)?;
        let need = raw.len() / 32;
        let mut at = None;
        loop {
            let mut run = 0usize;
            for i in 0..d.data.len() / 32 {
                if d.data[i * 32] & 0x80 == 0 {
                    run += 1;
                    if run == need {
                        at = Some(i + 1 - run);
                        break;
                    }
                } else {
                    run = 0;
                }
            }
            if at.is_some() {
                break;
            }
            self.grow_dir(&mut d)?;
        }
        let at = at.unwrap();
        d.data[at * 32..at * 32 + raw.len()].copy_from_slice(raw);
        self.store_dir(&d)
    }

    fn kill_set(&mut self, dir: &DirRef, set: &Set) -> Result<()> {
        let mut d = self.load_dir(dir)?;
        for i in set.idx..set.idx + set.count {
            d.data[i * 32] &= 0x7f;
        }
        self.store_dir(&d)
    }

    fn exists_in(&mut self, dir: &DirRef, name: &str) -> Result<bool> {
        let d = self.load_dir(dir)?;
        let want = self.upcase_units(name);
        let sets = self.parse_sets(&d.data);
        Ok(sets.iter().any(|s| self.upcase_units(&s.name) == want))
    }

    /// Creates a directory.
    pub fn mkdir(&mut self, path: &str) -> Result<()> {
        self.check_rw()?;
        let (parent, name) = self.split_parent(path)?;
        validate_name(name)?;
        if self.exists_in(&parent, name)? {
            return Err(FsError::Exists);
        }
        let (first, nofat, cl) = self.grow(0, true, &[], 1)?;
        let cb = self.cluster_bytes() as u64;
        let raw = self.build_set(name, ATTR_DIR, first, nofat, cb);
        if let Err(e) = self.insert_set(&parent, &raw) {
            self.release(&cl, nofat);
            return Err(e);
        }
        Ok(())
    }

    /// Creates or replaces a file.
    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<()> {
        self.check_rw()?;
        match self.resolve(path) {
            Ok((_, Some(s))) => {
                if s.attr & ATTR_DIR != 0 {
                    return Err(FsError::IsDir);
                }
                self.truncate(path, 0)?;
            }
            Ok((_, None)) => return Err(FsError::IsDir),
            Err(FsError::NotFound) => {
                let (parent, name) = self.split_parent(path)?;
                validate_name(name)?;
                if self.exists_in(&parent, name)? {
                    return Err(FsError::Exists);
                }
                let raw = self.build_set(name, ATTR_ARCHIVE, 0, false, 0);
                self.insert_set(&parent, &raw)?;
            }
            Err(e) => return Err(e),
        }
        self.write_at(path, 0, data)
    }

    fn file_set(&mut self, path: &str) -> Result<(DirRef, Set)> {
        if split_path(path).is_empty() {
            return Err(FsError::IsDir);
        }
        let (parent, _) = self.split_parent(path)?;
        let (_, e) = self.resolve(path)?;
        Ok((parent, e.ok_or(FsError::IsDir)?))
    }

    /// Writes `data` at `offset`, growing the file (the gap is zero-filled).
    pub fn write_at(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        self.check_rw()?;
        let (parent, s) = self.file_set(path)?;
        if s.attr & ATTR_DIR != 0 {
            return Err(FsError::IsDir);
        }
        let end = offset.checked_add(data.len() as u64).ok_or(FsError::NoSpace)?;
        let cb = self.cluster_bytes() as u64;
        let cur = s.len;
        let new_len = core::cmp::max(cur, end);
        if new_len > self.clusters as u64 * cb {
            return Err(FsError::NoSpace);
        }
        let cl = self.run_clusters(s.first, s.nofat, cur)?;
        let new_n = ((new_len + cb - 1) / cb) as usize;
        let (first, nofat, cl) = self.grow(s.first, s.nofat, &cl, new_n)?;
        // Zero [cur, offset) within pre-existing clusters.
        if offset > cur {
            let mut pos = cur;
            let mut tmp = vec![0u8; cb as usize];
            while pos < offset {
                let ci = (pos / cb) as usize;
                let co = (pos % cb) as usize;
                let n = core::cmp::min(cb as usize - co, (offset - pos) as usize);
                let off = self.cluster_off(cl[ci]);
                self.dev.read_at(off, &mut tmp)?;
                for b in &mut tmp[co..co + n] {
                    *b = 0;
                }
                self.write_cluster(cl[ci], &tmp)?;
                pos += n as u64;
            }
        }
        let mut done = 0usize;
        let mut tmp = vec![0u8; cb as usize];
        while done < data.len() {
            let pos = offset + done as u64;
            let ci = (pos / cb) as usize;
            let co = (pos % cb) as usize;
            let n = core::cmp::min(cb as usize - co, data.len() - done);
            if co == 0 && n == cb as usize {
                self.write_cluster(cl[ci], &data[done..done + n])?;
            } else {
                let off = self.cluster_off(cl[ci]);
                self.dev.read_at(off, &mut tmp)?;
                tmp[co..co + n].copy_from_slice(&data[done..done + n]);
                self.write_cluster(cl[ci], &tmp)?;
            }
            done += n;
        }
        let ts = ts_from_unix(self.now);
        self.patch_set(&parent, &s, |raw| {
            raw[33] = 0x01 | if nofat { 0x02 } else { 0 };
            put64(raw, 40, new_len);
            put32(raw, 52, first);
            put64(raw, 56, new_len);
            put32(raw, 12, ts);
            raw[21] = 0;
        })
    }

    /// Sets the file size; growing zero-fills, shrinking releases clusters.
    pub fn truncate(&mut self, path: &str, size: u64) -> Result<()> {
        self.check_rw()?;
        let (parent, s) = self.file_set(path)?;
        if s.attr & ATTR_DIR != 0 {
            return Err(FsError::IsDir);
        }
        if size > s.len {
            return self.write_at(path, size, &[]);
        }
        let cb = self.cluster_bytes() as u64;
        let cl = self.run_clusters(s.first, s.nofat, s.len)?;
        let keep = ((size + cb - 1) / cb) as usize;
        if keep < cl.len() {
            self.release(&cl[keep..], s.nofat);
            if keep > 0 && !s.nofat {
                self.fat_set(cl[keep - 1], FAT_EOC);
            }
        }
        let first = if keep == 0 { 0 } else { s.first };
        let nofat = if keep == 0 { false } else { s.nofat };
        let ts = ts_from_unix(self.now);
        self.patch_set(&parent, &s, |raw| {
            raw[33] = 0x01 | if nofat { 0x02 } else { 0 };
            put64(raw, 40, size);
            put32(raw, 52, first);
            put64(raw, 56, size);
            put32(raw, 12, ts);
        })
    }

    /// Deletes a file or an empty directory.
    pub fn remove(&mut self, path: &str) -> Result<()> {
        self.check_rw()?;
        let (parent, s) = self.file_set(path)?;
        let cl = if s.attr & ATTR_DIR != 0 {
            let dr = self.dirref_of(&parent, &s);
            let d = self.load_dir(&dr)?;
            if !self.parse_sets(&d.data).is_empty() {
                return Err(FsError::NotEmpty);
            }
            d.cl
        } else {
            self.run_clusters(s.first, s.nofat, s.len)?
        };
        self.release(&cl, s.nofat);
        self.kill_set(&parent, &s)
    }

    /// Renames or moves within the volume (not over an existing target).
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        self.check_rw()?;
        let (src_parent, s) = self.file_set(from)?;
        let (dst_parent, name) = self.split_parent(to)?;
        validate_name(name)?;
        if self.exists_in(&dst_parent, name)? {
            return Err(FsError::Exists);
        }
        if s.attr & ATTR_DIR != 0 {
            // Reject moving a directory beneath itself.
            let mut cur = Some(dst_parent.clone());
            while let Some(c) = cur {
                if c.loc.is_some() && c.first == s.first {
                    return Err(FsError::InvalidName);
                }
                cur = c.loc.map(|(p, _)| *p);
            }
        }
        let mut raw = self.build_set(name, s.attr, s.first, s.nofat, s.len);
        put64(&mut raw, 40, s.valid);
        put32(&mut raw, 12, s.mtime);
        let c = set_checksum(&raw);
        put16(&mut raw, 2, c);
        self.insert_set(&dst_parent, &raw)?;
        // The source set index is unchanged unless it lives in the directory that just grew; re-find.
        let (sp, found) = self.file_set(from)?;
        self.kill_set(&sp, &found)?;
        let _ = src_parent;
        Ok(())
    }
}
