//! Bounds-checked views of common Btrfs little-endian disk structures.

use crate::checksum::{verify_crc32c_block, ChecksumError, BTRFS_CSUM_SIZE};

/// Primary Btrfs superblock byte offset.
pub const BTRFS_SUPERBLOCK_OFFSET: u64 = 65_536;
/// On-disk superblock size.
pub const BTRFS_SUPERBLOCK_SIZE: usize = 4_096;
/// Btrfs superblock magic, stored as a little-endian u64.
pub const BTRFS_MAGIC: u64 = 0x4d5f_5366_5248_425f;
/// Common metadata header size.
pub const BTRFS_HEADER_SIZE: usize = 101;
/// Serialized key size.
pub const BTRFS_KEY_SIZE: usize = 17;
/// Serialized leaf item size.
pub const BTRFS_ITEM_SIZE: usize = 25;
/// Serialized internal-node pointer size.
pub const BTRFS_NODE_PTR_SIZE: usize = 33;
/// Maximum tree level accepted by the on-disk format.
pub const BTRFS_MAX_TREE_LEVEL: u8 = 8;
/// Maximum system chunk-array payload in a superblock.
pub const BTRFS_SYSTEM_CHUNK_ARRAY_SIZE: usize = 2_048;
/// Offset of the system chunk array within a serialized superblock.
pub const BTRFS_SYSTEM_CHUNK_ARRAY_OFFSET: usize = 811;

const SUPER_FSID: usize = 32;
const SUPER_BYTENR: usize = 48;
const SUPER_MAGIC: usize = 64;
const SUPER_GENERATION: usize = 72;
const SUPER_ROOT: usize = 80;
const SUPER_CHUNK_ROOT: usize = 88;
const SUPER_TOTAL_BYTES: usize = 112;
const SUPER_ROOT_DIR_OBJECTID: usize = 128;
const SUPER_SECTORSIZE: usize = 144;
const SUPER_NODESIZE: usize = 148;
const SUPER_SYS_CHUNK_ARRAY_SIZE: usize = 160;
const SUPER_CSUM_TYPE: usize = 196;
const SUPER_ROOT_LEVEL: usize = 198;
const SUPER_CHUNK_ROOT_LEVEL: usize = 199;
const SUPER_LABEL: usize = 299;
const SUPER_METADATA_UUID: usize = SUPER_LABEL + 256 + 16;
const SUPER_SYS_CHUNK_ARRAY: usize = BTRFS_SYSTEM_CHUNK_ARRAY_OFFSET;

const HEADER_FSID: usize = 32;
const HEADER_BYTENR: usize = 48;
const HEADER_GENERATION: usize = 80;
const HEADER_OWNER: usize = 88;
const HEADER_NRITEMS: usize = 96;
const HEADER_LEVEL: usize = 100;

const CHUNK_ITEM_HEADER_SIZE: usize = 48;
const CHUNK_STRIPE_SIZE: usize = 32;
const CHUNK_ITEM_KEY_TYPE: u8 = 228;

/// A parse failure caused by malformed or truncated on-disk bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// A requested field extends beyond the supplied byte slice.
    Truncated { needed: usize, available: usize },
    /// A checked offset or length calculation overflowed.
    ArithmeticOverflow,
    /// A field has a value that cannot be valid for its format position.
    InvalidValue { field: &'static str },
    /// The byte sequence is not a Btrfs structure of the requested kind.
    InvalidMagic,
    /// The checksum type is not implemented by this foundation crate.
    UnsupportedChecksum(u16),
    /// A checksum was present but did not match.
    Checksum(ChecksumError),
}

fn need(available: usize, offset: usize, length: usize) -> Result<(), ParseError> {
    let end = offset
        .checked_add(length)
        .ok_or(ParseError::ArithmeticOverflow)?;
    if end <= available {
        Ok(())
    } else {
        Err(ParseError::Truncated {
            needed: end,
            available,
        })
    }
}

fn is_power_of_two(value: u32) -> bool {
    value != 0 && value & (value - 1) == 0
}

fn le16(bytes: &[u8], offset: usize) -> Result<u16, ParseError> {
    need(bytes.len(), offset, 2)?;
    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn le32(bytes: &[u8], offset: usize) -> Result<u32, ParseError> {
    need(bytes.len(), offset, 4)?;
    Ok(u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

fn le64(bytes: &[u8], offset: usize) -> Result<u64, ParseError> {
    need(bytes.len(), offset, 8)?;
    Ok(u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ]))
}

fn copy_array<const N: usize>(input: &[u8], offset: usize) -> Result<[u8; N], ParseError> {
    need(input.len(), offset, N)?;
    let mut output = [0; N];
    output.copy_from_slice(&input[offset..offset + N]);
    Ok(output)
}

/// A Btrfs object/type/offset key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BtrfsKey {
    /// Object identifier.
    pub object_id: u64,
    /// Item type.
    pub key_type: u8,
    /// Type-specific offset or identifier.
    pub offset: u64,
}

impl BtrfsKey {
    /// Parse one serialized key.
    pub fn parse(input: &[u8]) -> Result<Self, ParseError> {
        Ok(Self {
            object_id: le64(input, 0)?,
            key_type: *input.get(8).ok_or(ParseError::Truncated {
                needed: 9,
                available: input.len(),
            })?,
            offset: le64(input, 9)?,
        })
    }
}

/// The common header at the beginning of every tree block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BtrfsHeader {
    /// Stored checksum bytes.
    pub checksum: [u8; BTRFS_CSUM_SIZE],
    /// Filesystem metadata UUID.
    pub fsid: [u8; 16],
    /// Logical address of this block.
    pub bytenr: u64,
    /// Transaction generation that created this block.
    pub generation: u64,
    /// Owning tree object identifier.
    pub owner: u64,
    /// Number of items or node pointers.
    pub item_count: u32,
    /// Tree level, where zero is a leaf.
    pub level: u8,
}

impl BtrfsHeader {
    /// Parse a common tree-block header.
    pub fn parse(input: &[u8]) -> Result<Self, ParseError> {
        Ok(Self {
            checksum: copy_array(input, 0)?,
            fsid: copy_array(input, HEADER_FSID)?,
            bytenr: le64(input, HEADER_BYTENR)?,
            generation: le64(input, HEADER_GENERATION)?,
            owner: le64(input, HEADER_OWNER)?,
            item_count: le32(input, HEADER_NRITEMS)?,
            level: *input.get(HEADER_LEVEL).ok_or(ParseError::Truncated {
                needed: BTRFS_HEADER_SIZE,
                available: input.len(),
            })?,
        })
    }
}

/// A parsed leaf item descriptor and its bounded payload view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BtrfsLeafItem<'a> {
    /// Key associated with the item.
    pub key: BtrfsKey,
    /// Item payload bytes.
    pub data: &'a [u8],
}

/// A parsed internal-node pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BtrfsNodePtr {
    /// First key covered by the child block.
    pub key: BtrfsKey,
    /// Child logical address.
    pub blockptr: u64,
    /// Child generation.
    pub generation: u64,
}

/// A validated view over one complete tree block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BtrfsTreeBlock<'a> {
    bytes: &'a [u8],
    /// Common tree-block header.
    pub header: BtrfsHeader,
    entries_end: usize,
}

impl<'a> BtrfsTreeBlock<'a> {
    /// Parse and validate the header and item table bounds.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let header = BtrfsHeader::parse(bytes)?;
        if header.level > BTRFS_MAX_TREE_LEVEL {
            return Err(ParseError::InvalidValue { field: "level" });
        }
        let entry_size = if header.level == 0 {
            BTRFS_ITEM_SIZE
        } else {
            BTRFS_NODE_PTR_SIZE
        };
        let count = usize::try_from(header.item_count)
            .map_err(|_| ParseError::ArithmeticOverflow)?;
        let entries_len = count
            .checked_mul(entry_size)
            .ok_or(ParseError::ArithmeticOverflow)?;
        let entries_end = BTRFS_HEADER_SIZE
            .checked_add(entries_len)
            .ok_or(ParseError::ArithmeticOverflow)?;
        need(bytes.len(), 0, entries_end)?;
        Ok(Self {
            bytes,
            header,
            entries_end,
        })
    }

    /// Return the leaf item at `index`, including a payload slice validated
    /// against both the block and the item table.
    pub fn leaf_item(&self, index: usize) -> Result<BtrfsLeafItem<'a>, ParseError> {
        if self.header.level != 0 {
            return Err(ParseError::InvalidValue { field: "leaf level" });
        }
        if index >= self.header.item_count as usize {
            return Err(ParseError::InvalidValue { field: "item index" });
        }
        let offset = BTRFS_HEADER_SIZE
            .checked_add(
                index
                    .checked_mul(BTRFS_ITEM_SIZE)
                    .ok_or(ParseError::ArithmeticOverflow)?,
            )
            .ok_or(ParseError::ArithmeticOverflow)?;
        let key = BtrfsKey::parse(&self.bytes[offset..])?;
        let data_offset = le32(self.bytes, offset + 17)? as usize;
        let data_len = le32(self.bytes, offset + 21)? as usize;
        let data_start = BTRFS_HEADER_SIZE
            .checked_add(data_offset)
            .ok_or(ParseError::ArithmeticOverflow)?;
        let data_end = data_start
            .checked_add(data_len)
            .ok_or(ParseError::ArithmeticOverflow)?;
        if data_end > self.bytes.len()
            || (data_len != 0 && data_offset < self.entries_end - BTRFS_HEADER_SIZE)
        {
            return Err(ParseError::InvalidValue { field: "item payload" });
        }
        Ok(BtrfsLeafItem {
            key,
            data: &self.bytes[data_start..data_end],
        })
    }

    /// Return the internal-node pointer at `index`.
    pub fn node_ptr(&self, index: usize) -> Result<BtrfsNodePtr, ParseError> {
        if self.header.level == 0 {
            return Err(ParseError::InvalidValue { field: "node level" });
        }
        if index >= self.header.item_count as usize {
            return Err(ParseError::InvalidValue { field: "pointer index" });
        }
        let offset = BTRFS_HEADER_SIZE
            .checked_add(
                index
                    .checked_mul(BTRFS_NODE_PTR_SIZE)
                    .ok_or(ParseError::ArithmeticOverflow)?,
            )
            .ok_or(ParseError::ArithmeticOverflow)?;
        Ok(BtrfsNodePtr {
            key: BtrfsKey::parse(&self.bytes[offset..])?,
            blockptr: le64(self.bytes, offset + 17)?,
            generation: le64(self.bytes, offset + 25)?,
        })
    }
}

/// Parsed Btrfs superblock metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BtrfsSuperblock<'a> {
    /// Filesystem UUID.
    pub fsid: [u8; 16],
    /// Physical location recorded in the superblock.
    pub bytenr: u64,
    /// Current generation.
    pub generation: u64,
    /// Root tree logical address.
    pub root: u64,
    /// Chunk tree logical address.
    pub chunk_root: u64,
    /// Device bytes described by the filesystem.
    pub total_bytes: u64,
    /// Top-level filesystem root directory object id.
    pub root_dir_objectid: u64,
    /// Sector size.
    pub sectorsize: u32,
    /// Tree-node size.
    pub nodesize: u32,
    /// Checksum type identifier.
    pub csum_type: u16,
    /// Root tree level.
    pub root_level: u8,
    /// Chunk tree level.
    pub chunk_root_level: u8,
    /// Human-readable label bytes, including any trailing zeroes.
    pub label: [u8; 256],
    /// Metadata UUID, as stored on disk.
    pub metadata_uuid: [u8; 16],
    system_chunks: &'a [u8],
}

impl<'a> BtrfsSuperblock<'a> {
    /// Parse and validate a complete Btrfs superblock.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        need(bytes.len(), 0, BTRFS_SUPERBLOCK_SIZE)?;
        let magic = le64(bytes, SUPER_MAGIC)?;
        if magic != BTRFS_MAGIC {
            return Err(ParseError::InvalidMagic);
        }
        let bytenr = le64(bytes, SUPER_BYTENR)?;
        if bytenr != BTRFS_SUPERBLOCK_OFFSET {
            return Err(ParseError::InvalidValue { field: "bytenr" });
        }
        let sectorsize = le32(bytes, SUPER_SECTORSIZE)?;
        let nodesize = le32(bytes, SUPER_NODESIZE)?;
        if !is_power_of_two(sectorsize)
            || !is_power_of_two(nodesize)
            || sectorsize < 4_096
            || nodesize < sectorsize
            || nodesize > 65_536
        {
            return Err(ParseError::InvalidValue { field: "block size" });
        }
        let root = le64(bytes, SUPER_ROOT)?;
        let chunk_root = le64(bytes, SUPER_CHUNK_ROOT)?;
        let root_level = *bytes.get(SUPER_ROOT_LEVEL).ok_or(ParseError::Truncated {
            needed: SUPER_ROOT_LEVEL + 1,
            available: bytes.len(),
        })?;
        let chunk_root_level = *bytes
            .get(SUPER_CHUNK_ROOT_LEVEL)
            .ok_or(ParseError::Truncated {
                needed: SUPER_CHUNK_ROOT_LEVEL + 1,
                available: bytes.len(),
            })?;
        if root == 0
            || chunk_root == 0
            || root_level > BTRFS_MAX_TREE_LEVEL
            || chunk_root_level > BTRFS_MAX_TREE_LEVEL
        {
            return Err(ParseError::InvalidValue { field: "tree root" });
        }
        let array_size = le32(bytes, SUPER_SYS_CHUNK_ARRAY_SIZE)? as usize;
        if array_size == 0 || array_size > BTRFS_SYSTEM_CHUNK_ARRAY_SIZE {
            return Err(ParseError::InvalidValue {
                field: "system chunk array size",
            });
        }
        need(bytes.len(), SUPER_SYS_CHUNK_ARRAY, array_size)?;

        Ok(Self {
            fsid: copy_array::<16>(bytes, SUPER_FSID)?,
            bytenr,
            generation: le64(bytes, SUPER_GENERATION)?,
            root,
            chunk_root,
            total_bytes: le64(bytes, SUPER_TOTAL_BYTES)?,
            root_dir_objectid: le64(bytes, SUPER_ROOT_DIR_OBJECTID)?,
            sectorsize,
            nodesize,
            csum_type: le16(bytes, SUPER_CSUM_TYPE)?,
            root_level,
            chunk_root_level,
            label: copy_array::<256>(bytes, SUPER_LABEL)?,
            metadata_uuid: copy_array::<16>(bytes, SUPER_METADATA_UUID)?,
            system_chunks: &bytes[SUPER_SYS_CHUNK_ARRAY..SUPER_SYS_CHUNK_ARRAY + array_size],
        })
    }

    /// Verify this superblock when it declares CRC32C checksums.
    pub fn verify_checksum(&self, bytes: &[u8]) -> Result<(), ParseError> {
        if self.csum_type != 0 {
            return Err(ParseError::UnsupportedChecksum(self.csum_type));
        }
        verify_crc32c_block(bytes).map_err(ParseError::Checksum)
    }

    /// Return the stored system chunk array as a bounded parser.
    pub fn system_chunks(&self) -> SystemChunkArray<'a> {
        SystemChunkArray::new(self.system_chunks)
    }

    /// Return the metadata UUID, falling back to the filesystem UUID when the
    /// optional field is all zeroes as older filesystems do.
    pub fn effective_metadata_uuid(&self) -> [u8; 16] {
        if self.metadata_uuid.iter().all(|&byte| byte == 0) {
            self.fsid
        } else {
            self.metadata_uuid
        }
    }
}

/// A chunk stripe descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkStripe {
    /// Device identifier.
    pub devid: u64,
    /// Physical device byte address.
    pub offset: u64,
    /// Device UUID.
    pub device_uuid: [u8; 16],
}

/// A variable-sized chunk item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkItem<'a> {
    bytes: &'a [u8],
    /// Chunk logical length.
    pub length: u64,
    /// Block-group profile/type flags.
    pub chunk_type: u64,
    /// Number of stripe records.
    pub stripe_count: u16,
    /// Number of sub-stripes used by RAID10.
    pub sub_stripes: u16,
}

impl<'a> ChunkItem<'a> {
    /// Parse a chunk item payload.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        need(bytes.len(), 0, CHUNK_ITEM_HEADER_SIZE)?;
        let stripe_count = le16(bytes, 44)?;
        let sub_stripes = le16(bytes, 46)?;
        let stripe_bytes = usize::from(stripe_count)
            .checked_mul(CHUNK_STRIPE_SIZE)
            .ok_or(ParseError::ArithmeticOverflow)?;
        let needed = CHUNK_ITEM_HEADER_SIZE
            .checked_add(stripe_bytes)
            .ok_or(ParseError::ArithmeticOverflow)?;
        need(bytes.len(), 0, needed)?;
        Ok(Self {
            bytes: &bytes[..needed],
            length: le64(bytes, 0)?,
            chunk_type: le64(bytes, 24)?,
            stripe_count,
            sub_stripes,
        })
    }

    /// Return a stripe descriptor by index.
    pub fn stripe(&self, index: usize) -> Result<ChunkStripe, ParseError> {
        if index >= usize::from(self.stripe_count) {
            return Err(ParseError::InvalidValue { field: "stripe index" });
        }
        let offset = CHUNK_ITEM_HEADER_SIZE
            .checked_add(
                index
                    .checked_mul(CHUNK_STRIPE_SIZE)
                    .ok_or(ParseError::ArithmeticOverflow)?,
            )
            .ok_or(ParseError::ArithmeticOverflow)?;
        Ok(ChunkStripe {
            devid: le64(self.bytes, offset)?,
            offset: le64(self.bytes, offset + 8)?,
            device_uuid: copy_array(self.bytes, offset + 16)?,
        })
    }
}

/// One parsed entry from a superblock system chunk array.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemChunk<'a> {
    /// Key identifying the chunk's logical start.
    pub key: BtrfsKey,
    /// Variable-size chunk payload.
    pub item: ChunkItem<'a>,
}

/// Allocation-free cursor over a superblock's system chunk array.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemChunkArray<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> SystemChunkArray<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    /// Parse the next chunk, returning `Ok(None)` at the end of the array.
    pub fn next(&mut self) -> Result<Option<SystemChunk<'a>>, ParseError> {
        if self.cursor == self.bytes.len() {
            return Ok(None);
        }
        let key = BtrfsKey::parse(&self.bytes[self.cursor..])?;
        if key.key_type != CHUNK_ITEM_KEY_TYPE {
            return Err(ParseError::InvalidValue { field: "chunk key type" });
        }
        let payload_start = self
            .cursor
            .checked_add(BTRFS_KEY_SIZE)
            .ok_or(ParseError::ArithmeticOverflow)?;
        let item = ChunkItem::parse(&self.bytes[payload_start..])?;
        let item_end = payload_start
            .checked_add(item.bytes.len())
            .ok_or(ParseError::ArithmeticOverflow)?;
        if item_end > self.bytes.len() {
            return Err(ParseError::Truncated {
                needed: item_end,
                available: self.bytes.len(),
            });
        }
        self.cursor = item_end;
        Ok(Some(SystemChunk { key, item }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::write_crc32c_block;

    fn put16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn superblock_parses_and_verifies() {
        let mut bytes = [0u8; BTRFS_SUPERBLOCK_SIZE];
        bytes[SUPER_FSID..SUPER_FSID + 16].copy_from_slice(&[1; 16]);
        put64(&mut bytes, SUPER_BYTENR, BTRFS_SUPERBLOCK_OFFSET);
        put64(&mut bytes, SUPER_MAGIC, BTRFS_MAGIC);
        put64(&mut bytes, SUPER_ROOT, 0x1000);
        put64(&mut bytes, SUPER_CHUNK_ROOT, 0x2000);
        put64(&mut bytes, SUPER_TOTAL_BYTES, 128 << 20);
        put64(&mut bytes, SUPER_ROOT_DIR_OBJECTID, 256);
        put32(&mut bytes, SUPER_SECTORSIZE, 4096);
        put32(&mut bytes, SUPER_NODESIZE, 16_384);
        put32(&mut bytes, SUPER_SYS_CHUNK_ARRAY_SIZE, 97);
        bytes[SUPER_SYS_CHUNK_ARRAY + 17 + 44..SUPER_SYS_CHUNK_ARRAY + 17 + 46]
            .copy_from_slice(&1u16.to_le_bytes());
        bytes[SUPER_SYS_CHUNK_ARRAY + 17 + 48 + 8..SUPER_SYS_CHUNK_ARRAY + 17 + 48 + 16]
            .copy_from_slice(&0x4000u64.to_le_bytes());
        bytes[SUPER_SYS_CHUNK_ARRAY + 17 + 48..SUPER_SYS_CHUNK_ARRAY + 17 + 48 + 8]
            .copy_from_slice(&1u64.to_le_bytes());
        put64(&mut bytes, SUPER_SYS_CHUNK_ARRAY + 9, 0x100);
        bytes[SUPER_SYS_CHUNK_ARRAY + 8] = CHUNK_ITEM_KEY_TYPE;
        put16(&mut bytes, SUPER_SYS_CHUNK_ARRAY + 17 + 44, 1);
        write_crc32c_block(&mut bytes).unwrap();

        let superblock = BtrfsSuperblock::parse(&bytes).unwrap();
        superblock.verify_checksum(&bytes).unwrap();
        assert_eq!(superblock.effective_metadata_uuid(), [1; 16]);
        let mut chunks = superblock.system_chunks();
        let chunk = chunks.next().unwrap().unwrap();
        assert_eq!(chunk.key.offset, 0x100);
        assert_eq!(chunk.item.stripe_count, 1);
        assert_eq!(chunk.item.stripe(0).unwrap().offset, 0x4000);
        assert!(chunks.next().unwrap().is_none());
    }

    #[test]
    fn tree_leaf_rejects_payload_overlapping_entries() {
        let mut bytes = [0u8; 256];
        bytes[HEADER_LEVEL] = 0;
        put32(&mut bytes, HEADER_NRITEMS, 1);
        let item = BTRFS_HEADER_SIZE;
        bytes[item + 8] = 1;
        put32(&mut bytes, item + 17, 0);
        put32(&mut bytes, item + 21, 1);
        let tree = BtrfsTreeBlock::parse(&bytes).unwrap();
        assert_eq!(
            tree.leaf_item(0),
            Err(ParseError::InvalidValue { field: "item payload" })
        );
    }

    #[test]
    fn tree_node_and_key_are_little_endian() {
        let mut bytes = [0u8; 256];
        bytes[HEADER_LEVEL] = 1;
        put32(&mut bytes, HEADER_NRITEMS, 1);
        let entry = BTRFS_HEADER_SIZE;
        put64(&mut bytes, entry, 7);
        bytes[entry + 8] = 228;
        put64(&mut bytes, entry + 9, 99);
        put64(&mut bytes, entry + 17, 0xdead_beef);
        put64(&mut bytes, entry + 25, 42);
        let ptr = BtrfsTreeBlock::parse(&bytes).unwrap().node_ptr(0).unwrap();
        assert_eq!(ptr.key, BtrfsKey { object_id: 7, key_type: 228, offset: 99 });
        assert_eq!(ptr.blockptr, 0xdead_beef);
        assert_eq!(ptr.generation, 42);
    }

    #[test]
    fn malformed_chunk_count_is_rejected_without_panic() {
        let mut bytes = [0u8; 48];
        put16(&mut bytes, 44, u16::MAX);
        assert!(matches!(
            ChunkItem::parse(&bytes),
            Err(ParseError::Truncated { .. })
        ));
    }
}
