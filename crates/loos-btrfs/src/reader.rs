//! Read-only Btrfs logical reader.
//!
//! This module deliberately owns no device and performs no writes.  A caller
//! supplies a byte-addressed [`ReadAt`] implementation (or [`SliceReader`]) and
//! the reader builds a bounded logical map from the filesystem's system and
//! chunk trees.  Metadata blocks and data sectors are checked before any bytes
//! are returned to the caller.

extern crate alloc;

use alloc::{vec, vec::Vec};

use super::{
    crc32c_raw, verify, BtrfsKey, BtrfsLeafItem, BtrfsSuperblock, BtrfsTreeBlock,
    ChecksumAlgorithm, ChunkItem, ParseError, BTRFS_MAGIC, BTRFS_SUPERBLOCK_OFFSET,
    BTRFS_SUPERBLOCK_SIZE,
};

const SUPER_BYTENR: usize = 48;
const SUPER_MAGIC: usize = 64;
const SUPER_NUM_DEVICES: usize = 136;
const SUPER_INCOMPAT_FLAGS: usize = 188;

const HEADER_FSID: usize = 32;
const HEADER_BYTENR: usize = 48;

const ROOT_TREE_OBJECTID: u64 = 1;
const CHUNK_TREE_OBJECTID: u64 = 3;
const FS_TREE_OBJECTID: u64 = 5;
const CSUM_TREE_OBJECTID: u64 = 7;
const EXTENT_CSUM_OBJECTID: u64 = u64::MAX - 9;

const INODE_ITEM_KEY: u8 = 1;
const DIR_ITEM_KEY: u8 = 84;
const EXTENT_DATA_KEY: u8 = 108;
const EXTENT_CSUM_KEY: u8 = 128;
const ROOT_ITEM_KEY: u8 = 132;
const CHUNK_ITEM_KEY: u8 = 228;

const EXTENT_INLINE: u8 = 0;
const EXTENT_REG: u8 = 1;
const EXTENT_PREALLOC: u8 = 2;

const BLOCK_GROUP_TYPE_MASK: u64 = 0x7;
const BLOCK_GROUP_DUP: u64 = 1 << 5;
const BLOCK_GROUP_PROFILE_MASK: u64 = 0x7f8;
const BLOCK_GROUP_SUPPORTED_MASK: u64 = BLOCK_GROUP_TYPE_MASK | BLOCK_GROUP_PROFILE_MASK;

const INCOMPAT_MIXED_BACKREF: u64 = 1 << 0;
const INCOMPAT_DEFAULT_SUBVOL: u64 = 1 << 1;
const INCOMPAT_MIXED_GROUPS: u64 = 1 << 2;
const INCOMPAT_BIG_METADATA: u64 = 1 << 5;
const INCOMPAT_EXTENDED_IREF: u64 = 1 << 6;
const INCOMPAT_SKINNY_METADATA: u64 = 1 << 8;
const INCOMPAT_NO_HOLES: u64 = 1 << 9;
const INCOMPAT_METADATA_UUID: u64 = 1 << 10;
const INCOMPAT_SUPPORTED: u64 = INCOMPAT_MIXED_BACKREF
    | INCOMPAT_DEFAULT_SUBVOL
    | INCOMPAT_MIXED_GROUPS
    | INCOMPAT_BIG_METADATA
    | INCOMPAT_EXTENDED_IREF
    | INCOMPAT_SKINNY_METADATA
    | INCOMPAT_NO_HOLES
    | INCOMPAT_METADATA_UUID;

const MAX_TREE_BLOCKS: usize = 16_384;
const MAX_CHUNKS: usize = 16_384;
const MAX_NAME: usize = 255;

/// A byte-addressed read source.
pub trait ReadAt {
    /// Read exactly `out.len()` bytes beginning at `offset`.
    fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<(), ReadError>;

    /// Optional source length.  The reader still bounds every read against
    /// the Btrfs superblock's declared device size.
    fn len(&self) -> Option<u64> {
        None
    }
}

/// Errors returned by a byte-addressed source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    /// The requested byte range is outside the source.
    OutOfBounds { offset: u64, length: usize },
    /// The source could not complete the request.
    Backend,
}

/// A zero-copy read source over a complete image.
#[derive(Clone, Copy, Debug)]
pub struct SliceReader<'a> {
    bytes: &'a [u8],
}

impl<'a> SliceReader<'a> {
    /// Borrow an image as a byte-addressed source.
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl ReadAt for SliceReader<'_> {
    fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<(), ReadError> {
        let start = usize::try_from(offset).map_err(|_| ReadError::OutOfBounds {
            offset,
            length: out.len(),
        })?;
        let end = start.checked_add(out.len()).ok_or(ReadError::OutOfBounds {
            offset,
            length: out.len(),
        })?;
        if end > self.bytes.len() {
            return Err(ReadError::OutOfBounds {
                offset,
                length: out.len(),
            });
        }
        out.copy_from_slice(&self.bytes[start..end]);
        Ok(())
    }

    fn len(&self) -> Option<u64> {
        Some(self.bytes.len() as u64)
    }
}

/// File metadata returned by [`BtrfsReader::stat_path`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileStat {
    /// Inode object id.
    pub inode: u64,
    /// File size in bytes.
    pub size: u64,
    /// POSIX mode, including the file type bits.
    pub mode: u32,
    /// Btrfs inode flags.
    pub flags: u64,
    /// Owner uid.
    pub uid: u32,
    /// Owner gid.
    pub gid: u32,
    /// Link count.
    pub nlink: u32,
}

/// A directory entry with byte-exact name storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirEntry {
    /// Child inode object id.
    pub inode: u64,
    /// Btrfs directory type value.
    pub file_type: u8,
    /// Name bytes, without a NUL terminator.
    pub name: Vec<u8>,
}

/// A callback used by [`BtrfsReader::list_dir_into`].
pub trait DirEntrySink {
    /// Consume one directory entry.
    fn push(&mut self, entry: DirEntry) -> Result<(), ReaderError>;
}

/// Reader failures, including malformed on-disk structures and unsupported
/// Btrfs features.
#[derive(Debug, Eq, PartialEq)]
pub enum ReaderError {
    /// A source read failed.
    Read(ReadError),
    /// An existing on-disk parser rejected bytes.
    Parse(ParseError),
    /// The superblock or a tree block has a bad checksum.
    Checksum,
    /// A recognized checksum algorithm has no matching csum item.
    MissingChecksum { logical: u64 },
    /// A metadata mirror was present but all candidates were invalid.
    MirrorUnavailable { logical: u64 },
    /// The image uses a format feature this reader does not implement.
    Unsupported(&'static str),
    /// The image is malformed.
    Corrupt(&'static str),
    /// A path or requested range is invalid.
    InvalidInput(&'static str),
    /// A lookup found no matching object.
    NotFound,
    /// A path component requires a directory.
    NotDirectory,
    /// A requested file is a directory.
    IsDirectory,
    /// A bounded traversal exceeded its safety limit.
    Limit,
}

impl From<ReadError> for ReaderError {
    fn from(error: ReadError) -> Self {
        Self::Read(error)
    }
}

impl From<ParseError> for ReaderError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Stripe {
    devid: u64,
    physical: u64,
}

#[derive(Clone, Debug)]
struct ChunkMap {
    logical: u64,
    length: u64,
    flags: u64,
    stripes: Vec<Stripe>,
}

#[derive(Clone, Copy, Debug)]
struct RootInfo {
    bytenr: u64,
    root_dirid: u64,
    level: u8,
}

#[derive(Clone, Copy, Debug)]
struct InodeInfo {
    stat: FileStat,
}

/// A mounted, read-only Btrfs view.
pub struct BtrfsReader<R> {
    source: R,
    metadata_uuid: [u8; 16],
    total_bytes: u64,
    csum: ChecksumAlgorithm,
    sectorsize: u32,
    nodesize: u32,
    root_tree: u64,
    root_tree_level: u8,
    chunk_tree: u64,
    chunk_tree_level: u8,
    fs_tree: u64,
    fs_tree_level: u8,
    fs_root_dirid: u64,
    subvolume_id: u64,
    csum_tree: Option<(u64, u8)>,
    chunks: Vec<ChunkMap>,
}

impl<R: ReadAt> BtrfsReader<R> {
    /// Mount the top-level filesystem tree (subvolume object id 5).
    pub fn open(source: R) -> Result<Self, ReaderError> {
        Self::open_subvolume(source, FS_TREE_OBJECTID)
    }

    /// Mount a filesystem-tree root item by subvolume object id.
    pub fn open_subvolume(source: R, subvolume_id: u64) -> Result<Self, ReaderError> {
        if subvolume_id == 0 {
            return Err(ReaderError::InvalidInput("subvolume id"));
        }
        let mut super_bytes = vec![0u8; BTRFS_SUPERBLOCK_SIZE];
        source.read_at(BTRFS_SUPERBLOCK_OFFSET, &mut super_bytes)?;
        let parsed = BtrfsSuperblock::parse(&super_bytes)?;
        if parsed.csum_type > 3 {
            return Err(ReaderError::Unsupported("checksum algorithm"));
        }
        let csum = ChecksumAlgorithm::from_btrfs_type(parsed.csum_type)
            .ok_or(ReaderError::Unsupported("checksum algorithm"))?;
        verify_block_checksum(csum, &super_bytes)?;
        if le64(&super_bytes, SUPER_MAGIC)? != BTRFS_MAGIC
            || le64(&super_bytes, SUPER_BYTENR)? != BTRFS_SUPERBLOCK_OFFSET
        {
            return Err(ReaderError::Corrupt("superblock identity"));
        }
        if le64(&super_bytes, SUPER_NUM_DEVICES)? != 1 {
            return Err(ReaderError::Unsupported("multidevice filesystem"));
        }
        let incompat = le64(&super_bytes, SUPER_INCOMPAT_FLAGS)?;
        if incompat & !INCOMPAT_SUPPORTED != 0 {
            return Err(ReaderError::Unsupported("incompatible format flag"));
        }
        if incompat & ((1 << 3) | (1 << 4) | (1 << 7) | (1 << 11) | (1 << 12) | (1 << 13)) != 0 {
            return Err(ReaderError::Unsupported(
                "compression, parity, zoned, or extent-tree-v2",
            ));
        }
        let sectorsize = parsed.sectorsize;
        let nodesize = parsed.nodesize;
        if sectorsize > 65_536 || nodesize > 65_536 {
            return Err(ReaderError::Unsupported("block size"));
        }
        if let Some(length) = source.len() {
            if parsed.total_bytes > length {
                return Err(ReaderError::Corrupt("declared device exceeds source"));
            }
        }

        // BtrfsSuperblock borrows the temporary superblock; copy its fields
        // into a stable owned representation before the temporary is dropped.
        let metadata_uuid = parsed.effective_metadata_uuid();
        let root_tree = parsed.root;
        let root_tree_level = parsed.root_level;
        let chunk_tree = parsed.chunk_root;
        let chunk_tree_level = parsed.chunk_root_level;
        let total_bytes = parsed.total_bytes;
        let system_chunks = collect_system_chunks(&parsed)?;
        let mut reader = Self {
            source,
            metadata_uuid,
            total_bytes,
            csum,
            sectorsize,
            nodesize,
            root_tree,
            root_tree_level,
            chunk_tree,
            chunk_tree_level,
            fs_tree: 0,
            fs_tree_level: 0,
            fs_root_dirid: 0,
            subvolume_id,
            csum_tree: None,
            chunks: system_chunks,
        };
        for chunk in &reader.chunks {
            if chunk
                .logical
                .checked_add(chunk.length)
                .map_or(true, |e| e > total_bytes)
            {
                return Err(ReaderError::Corrupt("system chunk outside device"));
            }
            for stripe in &chunk.stripes {
                if stripe
                    .physical
                    .checked_add(chunk.length)
                    .map_or(true, |e| e > total_bytes)
                {
                    return Err(ReaderError::Corrupt("system stripe outside device"));
                }
            }
        }
        validate_chunk_set(&reader.chunks)?;
        reader.collect_chunk_tree()?;
        let fs_root = reader.find_root(ROOT_TREE_OBJECTID, subvolume_id)?;
        reader.fs_tree = fs_root.bytenr;
        reader.fs_tree_level = fs_root.level;
        reader.fs_root_dirid = fs_root.root_dirid;
        if reader.fs_root_dirid == 0 {
            return Err(ReaderError::Corrupt("root directory id"));
        }
        match reader.find_root(ROOT_TREE_OBJECTID, CSUM_TREE_OBJECTID) {
            Ok(csum_root) => reader.csum_tree = Some((csum_root.bytenr, csum_root.level)),
            Err(ReaderError::NotFound) => {}
            Err(error) => return Err(error),
        }
        Ok(reader)
    }

    /// Return the mounted subvolume object id.
    pub const fn subvolume_id(&self) -> u64 {
        self.subvolume_id
    }

    /// Look up a name in a directory inode.
    pub fn lookup(&self, directory: u64, name: &[u8]) -> Result<DirEntry, ReaderError> {
        if name.is_empty() || name.len() > MAX_NAME || name.iter().any(|&b| b == b'/') {
            return Err(ReaderError::InvalidInput("directory name"));
        }
        if !is_dir(self.find_inode(directory)?.stat.mode) {
            return Err(ReaderError::NotDirectory);
        }
        let hash = crc32c_raw(!1, name) as u64;
        let mut found = None;
        let exact = key_range(directory, DIR_ITEM_KEY, hash, directory, DIR_ITEM_KEY, hash);
        self.walk_tree(
            self.fs_tree,
            self.fs_tree_level,
            self.subvolume_id,
            exact,
            &mut |item| {
                if let Some(entry) = parse_dir_item(item, name)? {
                    found = Some(entry);
                    return Ok(true);
                }
                Ok(false)
            },
        )?;
        if found.is_none() {
            let range = key_range(
                directory,
                DIR_ITEM_KEY,
                0,
                directory,
                DIR_ITEM_KEY,
                u64::MAX,
            );
            self.walk_tree(
                self.fs_tree,
                self.fs_tree_level,
                self.subvolume_id,
                range,
                &mut |item| {
                    if let Some(entry) = parse_dir_item(item, name)? {
                        found = Some(entry);
                        return Ok(true);
                    }
                    Ok(false)
                },
            )?;
        }
        found.ok_or(ReaderError::NotFound)
    }

    /// Enumerate a directory inode in DIR_ITEM hash order.
    pub fn list_dir(&self, directory: u64) -> Result<Vec<DirEntry>, ReaderError> {
        if !is_dir(self.find_inode(directory)?.stat.mode) {
            return Err(ReaderError::NotDirectory);
        }
        let mut entries = Vec::new();
        self.list_dir_into(directory, VecSink(&mut entries))?;
        Ok(entries)
    }

    /// Enumerate a directory inode without requiring a result allocation.
    pub fn list_dir_into<S: DirEntrySink>(
        &self,
        directory: u64,
        mut sink: S,
    ) -> Result<(), ReaderError> {
        if !is_dir(self.find_inode(directory)?.stat.mode) {
            return Err(ReaderError::NotDirectory);
        }
        let range = key_range(
            directory,
            DIR_ITEM_KEY,
            0,
            directory,
            DIR_ITEM_KEY,
            u64::MAX,
        );
        self.walk_tree(
            self.fs_tree,
            self.fs_tree_level,
            self.subvolume_id,
            range,
            &mut |item| {
                parse_dir_item_all(item, &mut sink)?;
                Ok(false)
            },
        )
    }

    /// Stat an absolute path using byte-exact UTF-8-independent components.
    pub fn stat_path(&self, path: &[u8]) -> Result<FileStat, ReaderError> {
        let (_, inode) = self.resolve_path(path)?;
        Ok(inode.stat)
    }

    /// Read up to `length` bytes from an absolute regular-file path.
    pub fn read_file(
        &self,
        path: &[u8],
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, ReaderError> {
        let (_, inode) = self.resolve_path(path)?;
        if is_dir(inode.stat.mode) {
            return Err(ReaderError::IsDirectory);
        }
        if !is_regular(inode.stat.mode) {
            return Err(ReaderError::Unsupported("special file"));
        }
        if offset >= inode.stat.size || length == 0 {
            return Ok(Vec::new());
        }
        let available = inode.stat.size - offset;
        let wanted = core::cmp::min(available, length as u64);
        let wanted = usize::try_from(wanted).map_err(|_| ReaderError::Limit)?;
        let mut output = vec![0u8; wanted];
        let end = offset
            .checked_add(wanted as u64)
            .ok_or(ReaderError::Limit)?;
        let range = key_range(
            inode.stat.inode,
            EXTENT_DATA_KEY,
            0,
            inode.stat.inode,
            EXTENT_DATA_KEY,
            u64::MAX,
        );
        self.walk_tree(
            self.fs_tree,
            self.fs_tree_level,
            self.subvolume_id,
            range,
            &mut |item| self.copy_extent(item, offset, end, &mut output),
        )?;
        Ok(output)
    }

    fn resolve_path(&self, path: &[u8]) -> Result<(u64, InodeInfo), ReaderError> {
        if path.first().copied() != Some(b'/') {
            return Err(ReaderError::InvalidInput("absolute path"));
        }
        let mut inode = self.fs_root_dirid;
        let mut info = self.find_inode(inode)?;
        let mut cursor = 0usize;
        while cursor < path.len() {
            while cursor < path.len() && path[cursor] == b'/' {
                cursor += 1;
            }
            if cursor == path.len() {
                break;
            }
            let start = cursor;
            while cursor < path.len() && path[cursor] != b'/' {
                cursor += 1;
            }
            let component = &path[start..cursor];
            if component.is_empty() || component.len() > MAX_NAME {
                return Err(ReaderError::InvalidInput("path component"));
            }
            if !is_dir(info.stat.mode) {
                return Err(ReaderError::NotDirectory);
            }
            let entry = self.lookup(inode, component)?;
            inode = entry.inode;
            info = self.find_inode(inode)?;
        }
        Ok((inode, info))
    }

    fn find_inode(&self, inode: u64) -> Result<InodeInfo, ReaderError> {
        let range = key_range(inode, INODE_ITEM_KEY, 0, inode, INODE_ITEM_KEY, 0);
        let mut found = None;
        self.walk_tree(
            self.fs_tree,
            self.fs_tree_level,
            self.subvolume_id,
            range,
            &mut |item| {
                if item.data.len() < 160 {
                    return Err(ReaderError::Corrupt("inode item length"));
                }
                found = Some(InodeInfo {
                    stat: FileStat {
                        inode,
                        size: le64(item.data, 16)?,
                        mode: le32(item.data, 52)?,
                        flags: le64(item.data, 64)?,
                        uid: le32(item.data, 44)?,
                        gid: le32(item.data, 48)?,
                        nlink: le32(item.data, 40)?,
                    },
                });
                Ok(true)
            },
        )?;
        found.ok_or(ReaderError::NotFound)
    }

    fn copy_extent(
        &self,
        item: BtrfsLeafItem<'_>,
        request_start: u64,
        request_end: u64,
        output: &mut [u8],
    ) -> Result<bool, ReaderError> {
        if item.key.key_type != EXTENT_DATA_KEY {
            return Ok(false);
        }
        if item.data.len() < 21 {
            return Err(ReaderError::Corrupt("extent item length"));
        }
        let extent_type = item.data[20];
        let extent_length = match extent_type {
            EXTENT_INLINE => {
                let ram = le64(item.data, 8)?;
                if ram > (item.data.len() - 21) as u64 {
                    return Err(ReaderError::Corrupt("inline extent length"));
                }
                ram
            }
            EXTENT_REG | EXTENT_PREALLOC => {
                if item.data.len() < 53 {
                    return Err(ReaderError::Corrupt("regular extent length"));
                }
                if item.data[16] != 0 || item.data[17] != 0 || item.data[18] != 0 {
                    return Err(ReaderError::Unsupported("compressed or encrypted extent"));
                }
                le64(item.data, 45)?
            }
            _ => return Err(ReaderError::Unsupported("extent type")),
        };
        let extent_end = item
            .key
            .offset
            .checked_add(extent_length)
            .ok_or(ReaderError::Corrupt("extent range"))?;
        let start = core::cmp::max(item.key.offset, request_start);
        let end = core::cmp::min(extent_end, request_end);
        if start >= end {
            return Ok(false);
        }
        let output_start =
            usize::try_from(start - request_start).map_err(|_| ReaderError::Limit)?;
        let copy_len = usize::try_from(end - start).map_err(|_| ReaderError::Limit)?;
        if output_start > output.len() || copy_len > output.len() - output_start {
            return Err(ReaderError::Limit);
        }
        match extent_type {
            EXTENT_INLINE => {
                let relative =
                    usize::try_from(start - item.key.offset).map_err(|_| ReaderError::Limit)?;
                output[output_start..output_start + copy_len]
                    .copy_from_slice(&item.data[21 + relative..21 + relative + copy_len]);
            }
            EXTENT_PREALLOC => {}
            EXTENT_REG => {
                let disk_bytenr = le64(item.data, 21)?;
                let disk_num_bytes = le64(item.data, 29)?;
                let extent_offset = le64(item.data, 37)?;
                if extent_offset > disk_num_bytes || extent_length > disk_num_bytes - extent_offset
                {
                    return Err(ReaderError::Corrupt("regular extent backing range"));
                }
                if disk_bytenr != 0 {
                    let logical = disk_bytenr
                        .checked_add(extent_offset)
                        .and_then(|v| v.checked_add(start - item.key.offset))
                        .ok_or(ReaderError::Limit)?;
                    self.read_data_checked(
                        logical,
                        &mut output[output_start..output_start + copy_len],
                    )?;
                }
            }
            _ => unreachable!(),
        }
        Ok(false)
    }

    fn collect_chunk_tree(&mut self) -> Result<(), ReaderError> {
        let range = key_range(0, 0, 0, u64::MAX, u8::MAX, u64::MAX);
        let chunk_tree = self.chunk_tree;
        let level = self.chunk_tree_level;
        let owner = CHUNK_TREE_OBJECTID;
        let mut additions = Vec::new();
        self.walk_tree(chunk_tree, level, owner, range, &mut |item| {
            if item.key.key_type != CHUNK_ITEM_KEY {
                return Ok(false);
            }
            let chunk = ChunkItem::parse(item.data)?;
            additions.push(self.chunk_from_item(item.key.offset, chunk)?);
            Ok(false)
        })?;
        for chunk in additions {
            self.insert_chunk(chunk)?;
        }
        Ok(())
    }

    fn chunk_from_item(&self, logical: u64, item: ChunkItem<'_>) -> Result<ChunkMap, ReaderError> {
        if item.length == 0
            || item.stripe_count == 0
            || item.chunk_type & !BLOCK_GROUP_SUPPORTED_MASK != 0
        {
            return Err(ReaderError::Corrupt("chunk flags or length"));
        }
        let profile = item.chunk_type & BLOCK_GROUP_PROFILE_MASK;
        if profile != 0 && profile != BLOCK_GROUP_DUP {
            return Err(ReaderError::Unsupported("striped or parity chunk profile"));
        }
        if profile == 0 && item.stripe_count != 1 {
            return Err(ReaderError::Unsupported("multidevice SINGLE chunk"));
        }
        if profile == BLOCK_GROUP_DUP && item.stripe_count < 2 {
            return Err(ReaderError::Corrupt("DUP stripe count"));
        }
        let mut stripes = Vec::new();
        let mut first_devid = None;
        for i in 0..usize::from(item.stripe_count) {
            let stripe = item.stripe(i)?;
            if let Some(devid) = first_devid {
                if stripe.devid != devid {
                    return Err(ReaderError::Unsupported("multidevice filesystem"));
                }
            } else {
                first_devid = Some(stripe.devid);
            }
            stripes.push(Stripe {
                devid: stripe.devid,
                physical: stripe.offset,
            });
        }
        Ok(ChunkMap {
            logical,
            length: item.length,
            flags: item.chunk_type,
            stripes,
        })
    }

    fn insert_chunk(&mut self, chunk: ChunkMap) -> Result<(), ReaderError> {
        if self.chunks.len() >= MAX_CHUNKS {
            return Err(ReaderError::Limit);
        }
        let end = chunk
            .logical
            .checked_add(chunk.length)
            .ok_or(ReaderError::Corrupt("chunk range"))?;
        if end > self.total_bytes {
            return Err(ReaderError::Corrupt("chunk outside device"));
        }
        for stripe in &chunk.stripes {
            if stripe
                .physical
                .checked_add(chunk.length)
                .map_or(true, |stripe_end| stripe_end > self.total_bytes)
            {
                return Err(ReaderError::Corrupt("stripe outside device"));
            }
        }
        for existing in &self.chunks {
            let existing_end = existing
                .logical
                .checked_add(existing.length)
                .ok_or(ReaderError::Corrupt("chunk range"))?;
            if chunk.logical < existing_end && existing.logical < end {
                if chunk.logical == existing.logical
                    && chunk.length == existing.length
                    && chunk.flags == existing.flags
                {
                    if chunk.stripes == existing.stripes {
                        return Ok(());
                    }
                    return Err(ReaderError::Corrupt("conflicting chunk mirrors"));
                }
                return Err(ReaderError::Corrupt("overlapping chunks"));
            }
        }
        self.chunks.push(chunk);
        self.chunks.sort_by_key(|c| c.logical);
        Ok(())
    }

    fn find_root(&self, _root_tree_objectid: u64, objectid: u64) -> Result<RootInfo, ReaderError> {
        let range = key_range(
            objectid,
            ROOT_ITEM_KEY,
            0,
            objectid,
            ROOT_ITEM_KEY,
            u64::MAX,
        );
        let mut found = None;
        self.walk_tree(
            self.root_tree,
            self.root_tree_level,
            ROOT_TREE_OBJECTID,
            range,
            &mut |item| {
                if item.data.len() <= 238 || item.data.len() < 184 {
                    return Err(ReaderError::Corrupt("root item length"));
                }
                found = Some(RootInfo {
                    root_dirid: le64(item.data, 168)?,
                    bytenr: le64(item.data, 176)?,
                    level: item.data[238],
                });
                Ok(true)
            },
        )?;
        let root = found.ok_or(ReaderError::NotFound)?;
        if root.bytenr == 0 || root.level > 8 {
            return Err(ReaderError::Corrupt("root item pointer"));
        }
        Ok(root)
    }

    fn walk_tree<F: FnMut(BtrfsLeafItem<'_>) -> Result<bool, ReaderError>>(
        &self,
        logical: u64,
        level: u8,
        owner: u64,
        range: KeyRange,
        callback: &mut F,
    ) -> Result<(), ReaderError> {
        let mut visited = 0usize;
        self.walk_tree_inner(logical, level, owner, range, callback, 0, &mut visited)
    }

    fn walk_tree_inner<F: FnMut(BtrfsLeafItem<'_>) -> Result<bool, ReaderError>>(
        &self,
        logical: u64,
        expected_level: u8,
        owner: u64,
        range: KeyRange,
        callback: &mut F,
        depth: usize,
        visited: &mut usize,
    ) -> Result<(), ReaderError> {
        if depth > 8 || *visited >= MAX_TREE_BLOCKS {
            return Err(ReaderError::Limit);
        }
        *visited += 1;
        let block = self.read_metadata_block(logical)?;
        let tree = BtrfsTreeBlock::parse(&block)?;
        if tree.header.level != expected_level
            || tree.header.owner != owner
            || tree.header.fsid != self.metadata_uuid
        {
            return Err(ReaderError::Corrupt("tree block identity"));
        }
        if tree.header.level == 0 {
            let mut previous = None;
            for index in 0..tree.header.item_count as usize {
                let item = tree.leaf_item(index)?;
                if let Some(key) = previous {
                    if compare_btrfs_keys(key, item.key) >= 0 {
                        return Err(ReaderError::Corrupt("leaf key order"));
                    }
                }
                previous = Some(item.key);
                if compare_btrfs_to_key(item.key, range.lower) < 0 {
                    continue;
                }
                if compare_btrfs_to_key(item.key, range.upper) > 0 {
                    break;
                }
                if callback(item)? {
                    break;
                }
            }
            return Ok(());
        }
        let mut previous = None;
        for index in 0..tree.header.item_count as usize {
            let ptr = tree.node_ptr(index)?;
            if let Some(key) = previous {
                if compare_btrfs_keys(key, ptr.key) >= 0 {
                    return Err(ReaderError::Corrupt("node key order"));
                }
            }
            previous = Some(ptr.key);
            if compare_btrfs_to_key(ptr.key, range.upper) > 0 {
                break;
            }
            let next = if index + 1 < tree.header.item_count as usize {
                Some(tree.node_ptr(index + 1)?.key)
            } else {
                None
            };
            if next.map_or(true, |key| compare_btrfs_to_key(key, range.lower) >= 0) {
                self.walk_tree_inner(
                    ptr.blockptr,
                    expected_level - 1,
                    owner,
                    range,
                    callback,
                    depth + 1,
                    visited,
                )?;
            }
        }
        Ok(())
    }

    fn read_metadata_block(&self, logical: u64) -> Result<Vec<u8>, ReaderError> {
        let candidates = self.map_candidates(logical, self.nodesize as usize)?;
        let mut block = vec![0u8; self.nodesize as usize];
        for (physical, _) in candidates {
            if self.source.read_at(physical, &mut block).is_err() {
                continue;
            }
            if verify_block_checksum(self.csum, &block).is_err()
                || le64(&block, HEADER_BYTENR)? != logical
                || block[HEADER_FSID..HEADER_FSID + 16] != self.metadata_uuid
            {
                continue;
            }
            return Ok(block);
        }
        Err(ReaderError::MirrorUnavailable { logical })
    }

    fn map_candidates(&self, logical: u64, length: usize) -> Result<Vec<(u64, u64)>, ReaderError> {
        let length = length as u64;
        for chunk in &self.chunks {
            let delta = logical.checked_sub(chunk.logical);
            if let Some(delta) = delta {
                if delta < chunk.length && length <= chunk.length - delta {
                    let mut out = Vec::new();
                    for stripe in &chunk.stripes {
                        let physical = stripe
                            .physical
                            .checked_add(delta)
                            .ok_or(ReaderError::Limit)?;
                        out.push((physical, stripe.devid));
                    }
                    return Ok(out);
                }
            }
        }
        Err(ReaderError::Corrupt("logical address is unmapped"))
    }

    fn read_data_checked(&self, logical: u64, out: &mut [u8]) -> Result<(), ReaderError> {
        if out.is_empty() {
            return Ok(());
        }
        let sector = self.sectorsize as usize;
        let start_sector = logical / self.sectorsize as u64;
        let end_logical = logical
            .checked_add(out.len() as u64)
            .ok_or(ReaderError::Limit)?;
        let end_sector = (end_logical - 1) / self.sectorsize as u64;
        let mut sector_bytes = vec![0u8; sector];
        let mut copied = 0usize;
        for sector_number in start_sector..=end_sector {
            let sector_logical = sector_number
                .checked_mul(self.sectorsize as u64)
                .ok_or(ReaderError::Limit)?;
            let candidates = self.map_candidates(sector_logical, sector)?;
            let mut valid = false;
            for (physical, _) in candidates {
                if self.source.read_at(physical, &mut sector_bytes).is_err() {
                    continue;
                }
                if self.verify_data_sector(sector_logical, &sector_bytes)? {
                    valid = true;
                    break;
                }
            }
            if !valid {
                return Err(ReaderError::MirrorUnavailable {
                    logical: sector_logical,
                });
            }
            let copy_start = if logical > sector_logical {
                (logical - sector_logical) as usize
            } else {
                0
            };
            let copy_end = core::cmp::min(sector, (end_logical - sector_logical) as usize);
            let amount = copy_end.saturating_sub(copy_start);
            if amount != 0 {
                out[copied..copied + amount].copy_from_slice(&sector_bytes[copy_start..copy_end]);
                copied += amount;
            }
        }
        Ok(())
    }

    fn verify_data_sector(&self, logical: u64, data: &[u8]) -> Result<bool, ReaderError> {
        let tree = self
            .csum_tree
            .ok_or(ReaderError::MissingChecksum { logical })?;
        let range = key_range(
            EXTENT_CSUM_OBJECTID,
            EXTENT_CSUM_KEY,
            0,
            EXTENT_CSUM_OBJECTID,
            EXTENT_CSUM_KEY,
            u64::MAX,
        );
        let mut result = None;
        self.walk_tree(tree.0, tree.1, CSUM_TREE_OBJECTID, range, &mut |item| {
            if item.key.key_type != EXTENT_CSUM_KEY || logical < item.key.offset {
                return Ok(false);
            }
            let delta = logical - item.key.offset;
            let index = delta / self.sectorsize as u64;
            let digest_len = self.csum.digest_len();
            let start = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_mul(digest_len));
            if let Some(start) = start {
                if start + digest_len <= item.data.len() && delta % self.sectorsize as u64 == 0 {
                    result = Some(verify(
                        self.csum,
                        data,
                        &item.data[start..start + digest_len],
                    ));
                    return Ok(true);
                }
            }
            Ok(false)
        })?;
        result.ok_or(ReaderError::MissingChecksum { logical })
    }
}

struct VecSink<'a>(&'a mut Vec<DirEntry>);
impl DirEntrySink for VecSink<'_> {
    fn push(&mut self, entry: DirEntry) -> Result<(), ReaderError> {
        self.0.push(entry);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct KeyRange {
    lower: Key,
    upper: Key,
}

#[derive(Clone, Copy)]
struct Key {
    object_id: u64,
    key_type: u8,
    offset: u64,
}

fn key_range(
    lo_obj: u64,
    lo_type: u8,
    lo_off: u64,
    hi_obj: u64,
    hi_type: u8,
    hi_off: u64,
) -> KeyRange {
    KeyRange {
        lower: Key {
            object_id: lo_obj,
            key_type: lo_type,
            offset: lo_off,
        },
        upper: Key {
            object_id: hi_obj,
            key_type: hi_type,
            offset: hi_off,
        },
    }
}

fn compare_btrfs_keys(a: BtrfsKey, b: BtrfsKey) -> i8 {
    if a.object_id < b.object_id {
        -1
    } else if a.object_id > b.object_id {
        1
    } else if a.key_type < b.key_type {
        -1
    } else if a.key_type > b.key_type {
        1
    } else if a.offset < b.offset {
        -1
    } else if a.offset > b.offset {
        1
    } else {
        0
    }
}

fn compare_btrfs_to_key(a: BtrfsKey, b: Key) -> i8 {
    if a.object_id < b.object_id {
        -1
    } else if a.object_id > b.object_id {
        1
    } else if a.key_type < b.key_type {
        -1
    } else if a.key_type > b.key_type {
        1
    } else if a.offset < b.offset {
        -1
    } else if a.offset > b.offset {
        1
    } else {
        0
    }
}

fn parse_dir_item(item: BtrfsLeafItem<'_>, wanted: &[u8]) -> Result<Option<DirEntry>, ReaderError> {
    let mut cursor = 0usize;
    while cursor < item.data.len() {
        let entry = parse_one_dir_item(item.data, &mut cursor)?;
        if entry.name == wanted {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

fn parse_dir_item_all<S: DirEntrySink>(
    item: BtrfsLeafItem<'_>,
    sink: &mut S,
) -> Result<(), ReaderError> {
    let mut cursor = 0usize;
    while cursor < item.data.len() {
        let entry = parse_one_dir_item(item.data, &mut cursor)?;
        sink.push(entry)?;
    }
    Ok(())
}

fn parse_one_dir_item(data: &[u8], cursor: &mut usize) -> Result<DirEntry, ReaderError> {
    if data.len() - *cursor < 30 {
        return Err(ReaderError::Corrupt("directory item header"));
    }
    let base = *cursor;
    let data_len = le16(data, base + 25)? as usize;
    let name_len = le16(data, base + 27)? as usize;
    if name_len > MAX_NAME {
        return Err(ReaderError::Corrupt("directory name length"));
    }
    let total = 30usize
        .checked_add(data_len)
        .and_then(|v| v.checked_add(name_len))
        .ok_or(ReaderError::Limit)?;
    if total > data.len() - base {
        return Err(ReaderError::Corrupt("directory item length"));
    }
    let name_start = base + 30 + data_len;
    let entry = DirEntry {
        inode: le64(data, base)?,
        file_type: data[base + 29],
        name: data[name_start..name_start + name_len].to_vec(),
    };
    *cursor += total;
    Ok(entry)
}

fn verify_block_checksum(algorithm: ChecksumAlgorithm, block: &[u8]) -> Result<(), ReaderError> {
    if block.len() <= 32 || !verify(algorithm, &block[32..], &block[..algorithm.digest_len()]) {
        return Err(ReaderError::Checksum);
    }
    Ok(())
}

fn is_dir(mode: u32) -> bool {
    mode & 0o170000 == 0o040000
}
fn is_regular(mode: u32) -> bool {
    mode & 0o170000 == 0o100000
}

fn le16(bytes: &[u8], offset: usize) -> Result<u16, ReaderError> {
    let end = offset.checked_add(2).ok_or(ReaderError::Limit)?;
    if end > bytes.len() {
        return Err(ReaderError::Corrupt("short field"));
    }
    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}
fn le32(bytes: &[u8], offset: usize) -> Result<u32, ReaderError> {
    let end = offset.checked_add(4).ok_or(ReaderError::Limit)?;
    if end > bytes.len() {
        return Err(ReaderError::Corrupt("short field"));
    }
    Ok(u32::from_le_bytes(bytes[offset..end].try_into().unwrap()))
}
fn le64(bytes: &[u8], offset: usize) -> Result<u64, ReaderError> {
    let end = offset.checked_add(8).ok_or(ReaderError::Limit)?;
    if end > bytes.len() {
        return Err(ReaderError::Corrupt("short field"));
    }
    Ok(u64::from_le_bytes(bytes[offset..end].try_into().unwrap()))
}

fn collect_system_chunks(superblock: &BtrfsSuperblock<'_>) -> Result<Vec<ChunkMap>, ReaderError> {
    let mut chunks = Vec::new();
    let mut array = superblock.system_chunks();
    while let Some(chunk) = array.next()? {
        chunks.push(chunk_to_map(chunk.key.offset, chunk.item)?);
    }
    Ok(chunks)
}

fn chunk_to_map(logical: u64, item: ChunkItem<'_>) -> Result<ChunkMap, ReaderError> {
    if item.length == 0
        || item.stripe_count == 0
        || item.chunk_type & !BLOCK_GROUP_SUPPORTED_MASK != 0
    {
        return Err(ReaderError::Corrupt("system chunk flags or length"));
    }
    let profile = item.chunk_type & BLOCK_GROUP_PROFILE_MASK;
    if profile != 0 && profile != BLOCK_GROUP_DUP {
        return Err(ReaderError::Unsupported("striped or parity chunk profile"));
    }
    if profile == 0 && item.stripe_count != 1 {
        return Err(ReaderError::Unsupported("multidevice SINGLE chunk"));
    }
    if profile == BLOCK_GROUP_DUP && item.stripe_count < 2 {
        return Err(ReaderError::Corrupt("DUP stripe count"));
    }
    let mut stripes = Vec::new();
    let mut devid = None;
    for i in 0..usize::from(item.stripe_count) {
        let stripe = item.stripe(i)?;
        if devid.map_or(false, |v| v != stripe.devid) {
            return Err(ReaderError::Unsupported("multidevice filesystem"));
        }
        devid = Some(stripe.devid);
        stripes.push(Stripe {
            devid: stripe.devid,
            physical: stripe.offset,
        });
    }
    Ok(ChunkMap {
        logical,
        length: item.length,
        flags: item.chunk_type,
        stripes,
    })
}

fn validate_chunk_set(chunks: &[ChunkMap]) -> Result<(), ReaderError> {
    for (index, chunk) in chunks.iter().enumerate() {
        let end = chunk
            .logical
            .checked_add(chunk.length)
            .ok_or(ReaderError::Corrupt("chunk range"))?;
        for other in &chunks[index + 1..] {
            let other_end = other
                .logical
                .checked_add(other.length)
                .ok_or(ReaderError::Corrupt("chunk range"))?;
            if chunk.logical < other_end && other.logical < end {
                return Err(ReaderError::Corrupt("overlapping chunks"));
            }
        }
    }
    Ok(())
}
