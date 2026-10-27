//! A btrfs volume: member devices, superblock, chunk map and verified logical reads.
use crate::consts::*;
use crate::dev::{Dev, DevError};
use crate::error::{Error, Result};
use crate::raid::{self, ChunkMap, Copy, Mapped, Stripe};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use loos_btrfs::{digest, verify as verify_digest, ChecksumAlgorithm};

pub(crate) fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
pub(crate) fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
pub(crate) fn le64(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}
pub(crate) fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
pub(crate) fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
pub(crate) fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// A tree key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    /// Object id.
    pub objectid: u64,
    /// Item type.
    pub ty: u8,
    /// Type-specific offset.
    pub offset: u64,
}

impl Key {
    /// Builds a key.
    pub const fn new(objectid: u64, ty: u8, offset: u64) -> Key {
        Key { objectid, ty, offset }
    }
    /// Smallest key with this object id.
    pub const fn min(objectid: u64) -> Key {
        Key { objectid, ty: 0, offset: 0 }
    }
    /// Largest key with this object id.
    pub const fn max(objectid: u64) -> Key {
        Key { objectid, ty: 255, offset: u64::MAX }
    }
    /// Parses 17 serialized bytes.
    pub fn parse(b: &[u8]) -> Key {
        Key { objectid: le64(b, 0), ty: b[8], offset: le64(b, 9) }
    }
    /// Serializes to 17 bytes.
    pub fn write(&self, b: &mut [u8]) {
        put64(b, 0, self.objectid);
        b[8] = self.ty;
        put64(b, 9, self.offset);
    }
}

/// An owned tree item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Key.
    pub key: Key,
    /// Payload.
    pub data: Vec<u8>,
}

/// What is wrong (if anything) with a tree block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockStatus {
    /// Checksum matches.
    pub csum_ok: bool,
    /// Header fsid matches the metadata UUID.
    pub fsid_ok: bool,
    /// Header bytenr matches the address it was read from.
    pub bytenr_ok: bool,
    /// Level is the expected one.
    pub level_ok: bool,
    /// Generation is not newer than the superblock's.
    pub generation_ok: bool,
}

impl BlockStatus {
    /// True when every check passed.
    pub fn ok(&self) -> bool {
        self.csum_ok && self.fsid_ok && self.bytenr_ok && self.level_ok && self.generation_ok
    }
}

/// Root of a tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeRef {
    /// Logical address of the root block.
    pub bytenr: u64,
    /// Level of the root block (0 = leaf).
    pub level: u8,
}

/// Device item stored in the superblock and the chunk tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevItem {
    /// Device id.
    pub devid: u64,
    /// Size in bytes.
    pub total_bytes: u64,
    /// Bytes allocated to chunks.
    pub bytes_used: u64,
    /// Sector size.
    pub sector_size: u32,
    /// Generation.
    pub generation: u64,
    /// Device UUID.
    pub uuid: [u8; 16],
    /// Filesystem UUID.
    pub fsid: [u8; 16],
}

impl DevItem {
    /// Parses a 98-byte `btrfs_dev_item`.
    pub fn parse(b: &[u8]) -> Result<DevItem> {
        if b.len() < 98 {
            return Err(Error::Corrupt("dev item size"));
        }
        let mut uuid = [0u8; 16];
        let mut fsid = [0u8; 16];
        uuid.copy_from_slice(&b[66..82]);
        fsid.copy_from_slice(&b[82..98]);
        Ok(DevItem { devid: le64(b, 0), total_bytes: le64(b, 8), bytes_used: le64(b, 16), sector_size: le32(b, 32), generation: le64(b, 44), uuid, fsid })
    }
}

/// Parsed superblock.
#[derive(Clone, Debug)]
pub struct Super {
    /// Filesystem UUID.
    pub fsid: [u8; 16],
    /// Metadata UUID (== fsid unless `METADATA_UUID`).
    pub metadata_uuid: [u8; 16],
    /// Superblock flags.
    pub flags: u64,
    /// Generation.
    pub generation: u64,
    /// Root tree.
    pub root: TreeRef,
    /// Chunk tree.
    pub chunk_root: TreeRef,
    /// Log tree (0 = none).
    pub log_root: u64,
    /// Total bytes over all devices.
    pub total_bytes: u64,
    /// Bytes used.
    pub bytes_used: u64,
    /// Root directory objectid (256).
    pub root_dir_objectid: u64,
    /// Number of member devices.
    pub num_devices: u64,
    /// Sector size.
    pub sectorsize: u32,
    /// Node size.
    pub nodesize: u32,
    /// Compat RO feature flags.
    pub compat_ro: u64,
    /// Incompat feature flags.
    pub incompat: u64,
    /// Checksum type.
    pub csum_type: u16,
    /// This device's item.
    pub dev_item: DevItem,
    /// Label.
    pub label: String,
    /// System chunk array.
    pub sys_chunks: Vec<u8>,
    /// Chunk root generation.
    pub chunk_root_generation: u64,
}

impl Super {
    /// Parses a 4096-byte superblock; verifies magic, bytenr and checksum.
    pub fn parse(raw: &[u8], expect_bytenr: u64) -> Result<Super> {
        if raw.len() < SUPER_INFO_SIZE || le64(raw, 64) != loos_btrfs::BTRFS_MAGIC {
            return Err(Error::NotBtrfs);
        }
        if le64(raw, 48) != expect_bytenr {
            return Err(Error::NotBtrfs);
        }
        let csum_type = le16(raw, 196);
        let alg = ChecksumAlgorithm::from_btrfs_type(csum_type).ok_or(Error::Unsupported("checksum type"))?;
        if !verify_digest(alg, &raw[32..SUPER_INFO_SIZE], &raw[..alg.digest_len()]) {
            return Err(Error::Csum);
        }
        let sectorsize = le32(raw, 144);
        let nodesize = le32(raw, 148);
        if !sectorsize.is_power_of_two() || sectorsize < 512 || !nodesize.is_power_of_two() || nodesize < sectorsize || nodesize > 65536 {
            return Err(Error::Corrupt("block sizes"));
        }
        let sys_size = le32(raw, 160) as usize;
        if sys_size > 2048 {
            return Err(Error::Corrupt("sys_chunk_array_size"));
        }
        let mut fsid = [0u8; 16];
        fsid.copy_from_slice(&raw[32..48]);
        let incompat = le64(raw, 188);
        let mut muuid = [0u8; 16];
        muuid.copy_from_slice(&raw[571..587]);
        let metadata_uuid = if incompat & INCOMPAT_METADATA_UUID != 0 && muuid != [0u8; 16] { muuid } else { fsid };
        let label_raw = &raw[299..299 + 256];
        let end = label_raw.iter().position(|&c| c == 0).unwrap_or(256);
        Ok(Super {
            fsid,
            metadata_uuid,
            flags: le64(raw, 56),
            generation: le64(raw, 72),
            root: TreeRef { bytenr: le64(raw, 80), level: raw[198] },
            chunk_root: TreeRef { bytenr: le64(raw, 88), level: raw[199] },
            log_root: le64(raw, 96),
            total_bytes: le64(raw, 112),
            bytes_used: le64(raw, 120),
            root_dir_objectid: le64(raw, 128),
            num_devices: le64(raw, 136),
            sectorsize,
            nodesize,
            compat_ro: le64(raw, 180),
            incompat,
            csum_type,
            dev_item: DevItem::parse(&raw[201..299])?,
            label: String::from_utf8_lossy(&label_raw[..end]).into_owned(),
            sys_chunks: raw[811..811 + sys_size].to_vec(),
            chunk_root_generation: le64(raw, 164),
        })
    }
}

/// Parses one chunk item (`btrfs_chunk`) into a [`ChunkMap`].
pub fn parse_chunk(logical: u64, b: &[u8]) -> Result<ChunkMap> {
    if b.len() < 48 {
        return Err(Error::Corrupt("chunk item size"));
    }
    let n = le16(b, 44) as usize;
    if b.len() < 48 + n * 32 {
        return Err(Error::Corrupt("chunk stripes"));
    }
    let stripes = (0..n).map(|i| Stripe { devid: le64(b, 48 + i * 32), physical: le64(b, 56 + i * 32) }).collect();
    let c = ChunkMap { logical, length: le64(b, 0), flags: le64(b, 24), stripe_len: le64(b, 16), stripes, sub_stripes: le16(b, 46) };
    c.validate().map_err(|_| Error::Corrupt("chunk geometry"))?;
    Ok(c)
}

/// A mounted set of btrfs devices.
pub struct Volume<D: Dev> {
    devs: BTreeMap<u64, D>,
    /// Superblock with the highest generation.
    pub sb: Super,
    alg: ChecksumAlgorithm,
    chunks: Vec<ChunkMap>,
    /// Devices described by the chunk tree.
    pub dev_items: BTreeMap<u64, DevItem>,
}

/// Bytes `[off, off+len)` of a freshly read device for superblock probing.
fn read_super_from<D: Dev>(d: &mut D) -> Option<Super> {
    let mut best: Option<Super> = None;
    for &off in SUPER_MIRROR_OFFSETS.iter() {
        if off + SUPER_INFO_SIZE as u64 > d.len() {
            continue;
        }
        let mut raw = vec![0u8; SUPER_INFO_SIZE];
        if d.read_at(off, &mut raw).is_err() {
            continue;
        }
        if let Ok(s) = Super::parse(&raw, off) {
            if best.as_ref().map_or(true, |b| s.generation > b.generation) {
                best = Some(s);
            }
        }
    }
    best
}

impl<D: Dev> Volume<D> {
    /// Opens the volume from its member devices (any order; missing members tolerated, reads that
    /// need them fail unless parity/mirrors can rebuild the data).
    pub fn open(devs: Vec<D>) -> Result<Volume<D>> {
        let mut map: BTreeMap<u64, D> = BTreeMap::new();
        let mut best: Option<Super> = None;
        for mut d in devs {
            let s = match read_super_from(&mut d) {
                Some(s) => s,
                None => continue,
            };
            if let Some(b) = &best {
                if b.fsid != s.fsid {
                    return Err(Error::Invalid("devices belong to different filesystems"));
                }
            }
            if best.as_ref().map_or(true, |b| s.generation > b.generation) {
                best = Some(s.clone());
            }
            map.insert(s.dev_item.devid, d);
        }
        let sb = best.ok_or(Error::NotBtrfs)?;
        let alg = ChecksumAlgorithm::from_btrfs_type(sb.csum_type).ok_or(Error::Unsupported("checksum type"))?;
        if sb.incompat & !INCOMPAT_KNOWN != 0 {
            return Err(Error::Unsupported("unknown incompat feature"));
        }
        let mut v = Volume { devs: map, sb, alg, chunks: Vec::new(), dev_items: BTreeMap::new() };
        v.load_sys_chunks()?;
        v.load_chunk_tree()?;
        Ok(v)
    }

    /// Builds a volume from already-known parts (devices keyed by devid, chunk map, geometry),
    /// bypassing superblock discovery. For tests and for tools that construct volumes in memory.
    #[doc(hidden)]
    pub fn synthetic(devs: BTreeMap<u64, D>, chunks: Vec<ChunkMap>, nodesize: u32, sectorsize: u32, alg: ChecksumAlgorithm) -> Volume<D> {
        let mut sb = Super {
            fsid: [0; 16],
            metadata_uuid: [0; 16],
            flags: 0,
            generation: 1,
            root: TreeRef { bytenr: 0, level: 0 },
            chunk_root: TreeRef { bytenr: 0, level: 0 },
            log_root: 0,
            total_bytes: 0,
            bytes_used: 0,
            root_dir_objectid: FIRST_FREE_OBJECTID,
            num_devices: devs.len() as u64,
            sectorsize,
            nodesize,
            compat_ro: 0,
            incompat: 0,
            csum_type: alg as u16,
            dev_item: DevItem { devid: 1, total_bytes: 0, bytes_used: 0, sector_size: sectorsize, generation: 1, uuid: [0; 16], fsid: [0; 16] },
            label: String::new(),
            sys_chunks: Vec::new(),
            chunk_root_generation: 1,
        };
        sb.total_bytes = devs.values().map(|d| d.len()).sum();
        Volume { devs, sb, alg, chunks, dev_items: BTreeMap::new() }
    }

    fn load_sys_chunks(&mut self) -> Result<()> {
        let a = self.sb.sys_chunks.clone();
        let mut i = 0;
        while i < a.len() {
            if i + 17 + 48 > a.len() {
                return Err(Error::Corrupt("sys chunk array"));
            }
            let key = Key::parse(&a[i..]);
            if key.ty != CHUNK_ITEM_KEY {
                return Err(Error::Corrupt("sys chunk key"));
            }
            let n = le16(&a[i + 17..], 44) as usize;
            let len = 48 + n * 32;
            if i + 17 + len > a.len() {
                return Err(Error::Corrupt("sys chunk stripes"));
            }
            let c = parse_chunk(key.offset, &a[i + 17..i + 17 + len])?;
            self.insert_chunk(c);
            i += 17 + len;
        }
        Ok(())
    }

    fn insert_chunk(&mut self, c: ChunkMap) {
        match self.chunks.binary_search_by(|x| x.logical.cmp(&c.logical)) {
            Ok(i) => self.chunks[i] = c,
            Err(i) => self.chunks.insert(i, c),
        }
    }

    fn load_chunk_tree(&mut self) -> Result<()> {
        let root = self.sb.chunk_root;
        let items = self.tree_items(root, Key::min(0), Key::max(u64::MAX))?;
        let mut fresh: Vec<ChunkMap> = Vec::new();
        for it in items {
            match it.key.ty {
                CHUNK_ITEM_KEY => fresh.push(parse_chunk(it.key.offset, &it.data)?),
                DEV_ITEM_KEY => {
                    let d = DevItem::parse(&it.data)?;
                    self.dev_items.insert(d.devid, d);
                }
                _ => {}
            }
        }
        if fresh.is_empty() {
            return Err(Error::Corrupt("empty chunk tree"));
        }
        self.chunks = fresh;
        self.chunks.sort_by_key(|c| c.logical);
        for w in self.chunks.windows(2) {
            if w[0].logical + w[0].length > w[1].logical {
                return Err(Error::Corrupt("overlapping chunks"));
            }
        }
        Ok(())
    }

    /// Checksum algorithm in use.
    pub fn alg(&self) -> ChecksumAlgorithm {
        self.alg
    }
    /// Bytes per stored checksum.
    pub fn csum_size(&self) -> usize {
        self.alg.digest_len()
    }
    /// Node size.
    pub fn nodesize(&self) -> u32 {
        self.sb.nodesize
    }
    /// Sector size.
    pub fn sectorsize(&self) -> u32 {
        self.sb.sectorsize
    }
    /// All chunks, ordered by logical address.
    pub fn chunks(&self) -> &[ChunkMap] {
        &self.chunks
    }
    /// Device ids present.
    pub fn devids(&self) -> Vec<u64> {
        self.devs.keys().copied().collect()
    }
    /// Device ids referenced by chunks but not present.
    pub fn missing_devices(&self) -> Vec<u64> {
        let mut m: Vec<u64> = Vec::new();
        for c in &self.chunks {
            for s in &c.stripes {
                if !self.devs.contains_key(&s.devid) && !m.contains(&s.devid) {
                    m.push(s.devid);
                }
            }
        }
        m
    }
    /// The chunk containing a logical address.
    pub fn chunk_for(&self, logical: u64) -> Option<&ChunkMap> {
        let i = match self.chunks.binary_search_by(|c| c.logical.cmp(&logical)) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        let c = &self.chunks[i];
        if logical < c.logical + c.length {
            Some(c)
        } else {
            None
        }
    }
    /// Number of independently readable copies of a logical address (RAID5/6 count the rebuild path).
    pub fn num_copies(&self, logical: u64) -> usize {
        match self.chunk_for(logical).and_then(|c| c.profile()) {
            Some(raid::Profile::Single) | Some(raid::Profile::Raid0) => 1,
            Some(raid::Profile::Dup) | Some(raid::Profile::Raid1) => 2,
            Some(raid::Profile::Raid10) => self.chunk_for(logical).map_or(1, |c| c.sub_stripes as usize),
            Some(raid::Profile::Raid1c3) => 3,
            Some(raid::Profile::Raid1c4) => 4,
            Some(raid::Profile::Raid5) | Some(raid::Profile::Raid6) => 2,
            None => 0,
        }
    }
    /// Detaches a device, simulating its loss (degraded operation); returns it.
    #[doc(hidden)]
    pub fn drop_device(&mut self, devid: u64) -> Option<D> {
        self.devs.remove(&devid)
    }
    /// Gives a device handle (for tools that need raw access).
    pub fn dev_mut(&mut self, devid: u64) -> Option<&mut D> {
        self.devs.get_mut(&devid)
    }
    /// Flushes every device.
    pub fn flush(&mut self) -> Result<()> {
        for d in self.devs.values_mut() {
            d.flush().map_err(Error::from)?;
        }
        Ok(())
    }

    // ----------------------------------------------------------------------- raw reads

    fn read_copy_at(&mut self, c: Copy, buf: &mut [u8]) -> Result<()> {
        let d = self.devs.get_mut(&c.devid).ok_or(Error::MissingDevice(c.devid))?;
        d.read_at(c.physical, buf).map_err(|e: DevError| e.into())
    }

    /// Reads one mapped range from copy `mirror` (RAID5/6: 0 = data stripe, 1 = rebuilt from parity).
    pub fn read_mapped(&mut self, m: &Mapped, buf: &mut [u8], mirror: usize) -> Result<()> {
        if let Some(p) = &m.parity {
            if mirror == 0 {
                match self.read_copy_at(m.copies[0], buf) {
                    Ok(()) => return Ok(()),
                    Err(_) => {}
                }
            }
            return self.rebuild_raid56(p, buf);
        }
        let c = *m.copies.get(mirror).ok_or(Error::Invalid("mirror index"))?;
        self.read_copy_at(c, buf)
    }

    fn rebuild_raid56(&mut self, p: &raid::ParityInfo, buf: &mut [u8]) -> Result<()> {
        let total = p.stripes.len();
        let nr_data = p.nr_data;
        let len = buf.len();
        let mut stripes: Vec<Option<Vec<u8>>> = Vec::with_capacity(total);
        for (i, s) in p.stripes.iter().enumerate() {
            if i == p.data_index {
                stripes.push(None); // the stripe we are rebuilding
                continue;
            }
            let mut v = vec![0u8; len];
            let phys = s.physical + p.offset_in_stripe;
            match self.read_copy_at(Copy { devid: s.devid, physical: phys }, &mut v) {
                Ok(()) => stripes.push(Some(v)),
                Err(_) => stripes.push(None),
            }
        }
        raid::reconstruct(&mut stripes, nr_data).map_err(|_| Error::Io)?;
        buf.copy_from_slice(stripes[p.data_index].as_ref().unwrap());
        Ok(())
    }

    /// Reads `buf.len()` bytes at `logical`, trying copies in turn until `verify(segment_logical,
    /// segment_bytes)` accepts each mapped segment. Returns the number of segments that needed a
    /// fallback copy (a hint for scrub/repair).
    pub fn read_logical_verified(&mut self, logical: u64, buf: &mut [u8], verify: &mut dyn FnMut(u64, &[u8]) -> bool) -> Result<u32> {
        let mut pos = 0usize;
        let mut fallbacks = 0u32;
        while pos < buf.len() {
            let lg = logical + pos as u64;
            let chunk = self.chunk_for(lg).ok_or(Error::Corrupt("logical address not mapped"))?;
            let m = chunk.map(lg).map_err(|_| Error::Corrupt("chunk mapping"))?;
            let n = core::cmp::min(m.len as usize, buf.len() - pos);
            let copies = if m.parity.is_some() { 2 } else { m.copies.len() };
            let mut ok = false;
            let mut last_err = Error::Io;
            for mirror in 0..copies {
                let seg = &mut buf[pos..pos + n];
                match self.read_mapped(&m, seg, mirror) {
                    Ok(()) => {
                        if verify(lg, seg) {
                            ok = true;
                            if mirror > 0 {
                                fallbacks += 1;
                            }
                            break;
                        }
                        last_err = Error::Csum;
                    }
                    Err(e) => last_err = e,
                }
            }
            if !ok {
                return Err(last_err);
            }
            pos += n;
        }
        Ok(fallbacks)
    }

    /// Reads without verification (first readable copy).
    pub fn read_logical(&mut self, logical: u64, buf: &mut [u8]) -> Result<()> {
        self.read_logical_verified(logical, buf, &mut |_, _| true).map(|_| ())
    }

    /// Overwrites one copy of a mapped range in place (scrub repair).
    pub fn write_copy(&mut self, c: Copy, data: &[u8]) -> Result<()> {
        let d = self.devs.get_mut(&c.devid).ok_or(Error::MissingDevice(c.devid))?;
        d.write_at(c.physical, data).map_err(Error::from)
    }

    // ----------------------------------------------------------------------- tree blocks

    /// Verifies a tree block's checksum (and optionally its header identity).
    pub fn check_block_csum(&self, raw: &[u8]) -> bool {
        raw.len() > 32 && verify_digest(self.alg, &raw[32..], &raw[..self.alg.digest_len()])
    }

    /// Reads and verifies a tree block (checksum, fsid, bytenr, level), using any good mirror.
    pub fn read_tree_block(&mut self, bytenr: u64, expect_level: Option<u8>) -> Result<Vec<u8>> {
        let ns = self.sb.nodesize as usize;
        let fsid = self.sb.metadata_uuid;
        let alg = self.alg;
        let gen_limit = self.sb.generation.saturating_add(1);
        let mut raw = vec![0u8; ns];
        let mut verify = |_lg: u64, b: &[u8]| -> bool {
            if !verify_digest(alg, &b[32..], &b[..alg.digest_len()]) {
                return false;
            }
            if b[32..48] != fsid || le64(b, 48) != bytenr || le64(b, 80) > gen_limit {
                return false;
            }
            match expect_level {
                Some(l) => b[100] == l,
                None => b[100] <= 8,
            }
        };
        self.read_logical_verified(bytenr, &mut raw, &mut verify).map_err(|e| if e == Error::Csum { Error::Corrupt("tree block failed verification") } else { e })?;
        Ok(raw)
    }

    /// Reads a tree block without failing on verification problems: returns the best copy (first fully
    /// valid one, else the first readable one) together with what is wrong with it.
    pub fn read_tree_block_raw(&mut self, bytenr: u64, expect_level: Option<u8>) -> Result<(Vec<u8>, BlockStatus)> {
        let ns = self.sb.nodesize as usize;
        let chunk = self.chunk_for(bytenr).ok_or(Error::Corrupt("tree block not mapped"))?;
        let m = chunk.map(bytenr).map_err(|_| Error::Corrupt("chunk mapping"))?;
        if (m.len as usize) < ns {
            return Err(Error::Corrupt("tree block straddles a stripe"));
        }
        let copies = if m.parity.is_some() { 2 } else { m.copies.len() };
        let mut first: Option<(Vec<u8>, BlockStatus)> = None;
        let mut last_err = Error::Io;
        for mirror in 0..copies {
            let mut raw = vec![0u8; ns];
            match self.read_mapped(&m, &mut raw, mirror) {
                Ok(()) => {
                    let st = self.block_status(&raw, bytenr, expect_level);
                    if st.ok() {
                        return Ok((raw, st));
                    }
                    if first.is_none() {
                        first = Some((raw, st));
                    }
                }
                Err(e) => last_err = e,
            }
        }
        first.ok_or(last_err)
    }

    /// Verification flags of a tree block image.
    pub fn block_status(&self, raw: &[u8], bytenr: u64, expect_level: Option<u8>) -> BlockStatus {
        BlockStatus {
            csum_ok: self.check_block_csum(raw),
            fsid_ok: raw[32..48] == self.sb.metadata_uuid,
            bytenr_ok: le64(raw, 48) == bytenr,
            level_ok: expect_level.map_or(raw[100] <= 8, |l| raw[100] == l),
            generation_ok: le64(raw, 80) <= self.sb.generation.saturating_add(1),
        }
    }

    /// Rewrites one tree block in place on **every** copy after applying `f`, recomputing its
    /// checksum. Offline-tool primitive (resize, tune, rescue); it is *not* copy-on-write, so the
    /// filesystem must be unmounted.
    pub fn patch_tree_block(&mut self, bytenr: u64, f: &mut dyn FnMut(&mut [u8])) -> Result<()> {
        let mut raw = self.read_tree_block(bytenr, None)?;
        f(&mut raw);
        let d = digest(self.alg, &raw[32..]);
        raw[..32].fill(0);
        raw[..d.as_bytes().len()].copy_from_slice(d.as_bytes());
        let chunk = self.chunk_for(bytenr).ok_or(Error::Corrupt("tree block not mapped"))?;
        let m = chunk.map(bytenr).map_err(|_| Error::Corrupt("chunk mapping"))?;
        for c in &m.copies {
            self.write_copy(*c, &raw)?;
        }
        Ok(())
    }

    /// Finds the leaf and item slot holding `key` in a tree: `(leaf bytenr, data offset in the block, size)`.
    pub fn locate_item(&mut self, root: TreeRef, key: Key) -> Result<Option<(u64, usize, usize)>> {
        let mut bytenr = root.bytenr;
        let mut level = root.level;
        loop {
            let raw = self.read_tree_block(bytenr, Some(level))?;
            if level == 0 {
                let n = le32(&raw, 96) as usize;
                for i in 0..n {
                    let o = HEADER_SIZE + i * ITEM_SIZE;
                    if Key::parse(&raw[o..]) == key {
                        return Ok(Some((bytenr, HEADER_SIZE + le32(&raw, o + 17) as usize, le32(&raw, o + 21) as usize)));
                    }
                }
                return Ok(None);
            }
            let ptrs = self.node_ptrs(&raw)?;
            let pick = match ptrs.binary_search_by(|p| p.0.cmp(&key)) {
                Ok(i) => i,
                Err(0) => return Ok(None),
                Err(i) => i - 1,
            };
            bytenr = ptrs[pick].1;
            level -= 1;
        }
    }

    /// Decodes a leaf into owned items; validates offsets.
    pub fn leaf_items(&self, raw: &[u8]) -> Result<Vec<Item>> {
        if raw[100] != 0 {
            return Err(Error::Corrupt("expected a leaf"));
        }
        let n = le32(raw, 96) as usize;
        if HEADER_SIZE + n * ITEM_SIZE > raw.len() {
            return Err(Error::Corrupt("leaf item count"));
        }
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let o = HEADER_SIZE + i * ITEM_SIZE;
            let off = le32(raw, o + 17) as usize;
            let size = le32(raw, o + 21) as usize;
            let s = HEADER_SIZE.checked_add(off).ok_or(Error::Corrupt("item offset"))?;
            let e = s.checked_add(size).ok_or(Error::Corrupt("item size"))?;
            if e > raw.len() || s < HEADER_SIZE + n * ITEM_SIZE {
                return Err(Error::Corrupt("item payload bounds"));
            }
            out.push(Item { key: Key::parse(&raw[o..]), data: raw[s..e].to_vec() });
        }
        Ok(out)
    }

    /// Child pointers of an internal node.
    pub fn node_ptrs(&self, raw: &[u8]) -> Result<Vec<(Key, u64, u64)>> {
        if raw[100] == 0 {
            return Err(Error::Corrupt("expected a node"));
        }
        let n = le32(raw, 96) as usize;
        if HEADER_SIZE + n * KEY_PTR_SIZE > raw.len() {
            return Err(Error::Corrupt("node pointer count"));
        }
        Ok((0..n)
            .map(|i| {
                let o = HEADER_SIZE + i * KEY_PTR_SIZE;
                (Key::parse(&raw[o..]), le64(raw, o + 17), le64(raw, o + 25))
            })
            .collect())
    }

    /// Visits items with `lo <= key <= hi` in order; `f` returns `false` to stop early.
    pub fn walk_tree(&mut self, root: TreeRef, lo: Key, hi: Key, f: &mut dyn FnMut(&Key, &[u8]) -> bool) -> Result<()> {
        self.walk(root.bytenr, root.level, lo, hi, &mut |_g, k, d| f(k, d)).map(|_| ())
    }

    /// Like [`Volume::walk_tree`] but also passes the generation of the leaf holding each item
    /// (what `BTRFS_IOC_TREE_SEARCH` reports as `transid`).
    pub fn walk_tree_gen(&mut self, root: TreeRef, lo: Key, hi: Key, f: &mut dyn FnMut(u64, &Key, &[u8]) -> bool) -> Result<()> {
        self.walk(root.bytenr, root.level, lo, hi, f).map(|_| ())
    }

    fn walk(&mut self, bytenr: u64, level: u8, lo: Key, hi: Key, f: &mut dyn FnMut(u64, &Key, &[u8]) -> bool) -> Result<bool> {
        let raw = self.read_tree_block(bytenr, Some(level))?;
        if level == 0 {
            let generation = le64(&raw, 80);
            for it in self.leaf_items(&raw)? {
                if it.key < lo {
                    continue;
                }
                if it.key > hi {
                    return Ok(false);
                }
                if !f(generation, &it.key, &it.data) {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        let ptrs = self.node_ptrs(&raw)?;
        // last child whose first key <= lo (or the first child)
        let start = match ptrs.binary_search_by(|p| p.0.cmp(&lo)) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        for (k, ptr, _gen) in ptrs.into_iter().skip(start) {
            if k > hi {
                break;
            }
            if !self.walk(ptr, level - 1, lo, hi, f)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// All items in `[lo, hi]`.
    pub fn tree_items(&mut self, root: TreeRef, lo: Key, hi: Key) -> Result<Vec<Item>> {
        let mut out = Vec::new();
        self.walk_tree(root, lo, hi, &mut |k, d| {
            out.push(Item { key: *k, data: d.to_vec() });
            true
        })?;
        Ok(out)
    }

    /// Exact-key lookup.
    pub fn tree_lookup(&mut self, root: TreeRef, key: Key) -> Result<Option<Vec<u8>>> {
        let mut found = None;
        self.walk_tree(root, key, key, &mut |_, d| {
            found = Some(d.to_vec());
            false
        })?;
        Ok(found)
    }

    /// First item with key >= `key` and key <= `hi`.
    pub fn tree_first_ge(&mut self, root: TreeRef, key: Key, hi: Key) -> Result<Option<Item>> {
        let mut found = None;
        self.walk_tree(root, key, hi, &mut |k, d| {
            found = Some(Item { key: *k, data: d.to_vec() });
            false
        })?;
        Ok(found)
    }

    /// Last item with key <= `key` (predecessor search; `None` if every key is larger).
    pub fn tree_floor(&mut self, root: TreeRef, key: Key) -> Result<Option<Item>> {
        self.floor(root.bytenr, root.level, key)
    }

    fn floor(&mut self, bytenr: u64, level: u8, key: Key) -> Result<Option<Item>> {
        let raw = self.read_tree_block(bytenr, Some(level))?;
        if level == 0 {
            let items = self.leaf_items(&raw)?;
            let idx = match items.binary_search_by(|i| i.key.cmp(&key)) {
                Ok(i) => i + 1,
                Err(i) => i,
            };
            return Ok(if idx == 0 { None } else { Some(items[idx - 1].clone()) });
        }
        let ptrs = self.node_ptrs(&raw)?;
        let start = match ptrs.binary_search_by(|p| p.0.cmp(&key)) {
            Ok(i) => i,
            Err(0) => return Ok(None),
            Err(i) => i - 1,
        };
        for i in (0..=start).rev() {
            if let Some(it) = self.floor(ptrs[i].1, level - 1, key)? {
                return Ok(Some(it));
            }
        }
        Ok(None)
    }

    // ----------------------------------------------------------------------- roots

    /// Looks up a tree root (`ROOT_ITEM`) in the root tree; returns the newest.
    pub fn tree_root(&mut self, objectid: u64) -> Result<TreeRef> {
        match objectid {
            ROOT_TREE_OBJECTID => return Ok(self.sb.root),
            CHUNK_TREE_OBJECTID => return Ok(self.sb.chunk_root),
            _ => {}
        }
        let root = self.sb.root;
        let it = self.tree_first_ge(root, Key::new(objectid, ROOT_ITEM_KEY, 0), Key::new(objectid, ROOT_ITEM_KEY, u64::MAX))?.ok_or(Error::NotFound)?;
        let ri = RootItem::parse(&it.data)?;
        Ok(TreeRef { bytenr: ri.bytenr, level: ri.level })
    }

    /// Reads the root item of a subvolume/tree.
    pub fn root_item(&mut self, objectid: u64) -> Result<RootItem> {
        let root = self.sb.root;
        let it = self.tree_first_ge(root, Key::new(objectid, ROOT_ITEM_KEY, 0), Key::new(objectid, ROOT_ITEM_KEY, u64::MAX))?.ok_or(Error::NotFound)?;
        RootItem::parse(&it.data)
    }

    /// Superblock offsets checked on a device of this size.
    pub fn super_offsets(dev_len: u64) -> Vec<u64> {
        SUPER_MIRROR_OFFSETS.iter().copied().filter(|&o| o + SUPER_INFO_SIZE as u64 <= dev_len).collect()
    }
}

/// `btrfs_root_item` fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootItem {
    /// Generation.
    pub generation: u64,
    /// Root directory objectid inside the tree.
    pub root_dirid: u64,
    /// Root block.
    pub bytenr: u64,
    /// Space limit (bytes).
    pub byte_limit: u64,
    /// Bytes used by the tree.
    pub bytes_used: u64,
    /// Last snapshot generation.
    pub last_snapshot: u64,
    /// `ROOT_SUBVOL_*` flags.
    pub flags: u64,
    /// Reference count.
    pub refs: u32,
    /// Level of the root block.
    pub level: u8,
    /// UUID.
    pub uuid: [u8; 16],
    /// Parent UUID (snapshots).
    pub parent_uuid: [u8; 16],
    /// Received UUID.
    pub received_uuid: [u8; 16],
    /// Change transid.
    pub ctransid: u64,
    /// Creation transid.
    pub otransid: u64,
    /// Send transid.
    pub stransid: u64,
    /// Receive transid.
    pub rtransid: u64,
}

impl RootItem {
    /// Parses a (possibly pre-v2) root item.
    pub fn parse(b: &[u8]) -> Result<RootItem> {
        // inode_item(160) generation@160 root_dirid@168 bytenr@176 byte_limit@184 bytes_used@192
        // last_snapshot@200 flags@208 refs@216 drop_progress(17)@220 drop_level@237 level@238
        if b.len() < 239 {
            return Err(Error::Corrupt("root item size"));
        }
        let mut r = RootItem {
            generation: le64(b, 160),
            root_dirid: le64(b, 168),
            bytenr: le64(b, 176),
            byte_limit: le64(b, 184),
            bytes_used: le64(b, 192),
            last_snapshot: le64(b, 200),
            flags: le64(b, 208),
            refs: le32(b, 216),
            level: b[238],
            uuid: [0; 16],
            parent_uuid: [0; 16],
            received_uuid: [0; 16],
            ctransid: 0,
            otransid: 0,
            stransid: 0,
            rtransid: 0,
        };
        // v2 extension: generation_v2@239 uuid@247 parent@263 received@279 ctransid@295 otransid@303 stransid@311 rtransid@319
        if b.len() >= 439 && le64(b, 239) == r.generation {
            r.uuid.copy_from_slice(&b[247..263]);
            r.parent_uuid.copy_from_slice(&b[263..279]);
            r.received_uuid.copy_from_slice(&b[279..295]);
            r.ctransid = le64(b, 295);
            r.otransid = le64(b, 303);
            r.stransid = le64(b, 311);
            r.rtransid = le64(b, 319);
        }
        Ok(r)
    }
}

/// Digest helper for tests/tools: checksum of `data` with the volume's algorithm.
pub fn csum_of(alg: ChecksumAlgorithm, data: &[u8]) -> Vec<u8> {
    digest(alg, data).as_bytes().to_vec()
}
