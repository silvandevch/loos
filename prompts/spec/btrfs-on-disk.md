# LoOS btrfs read-path contract

This is the interface fact sheet for the first native filesystem increment. It
describes the structures consumed by `src/fs/btrfs/btrfs.c`; it is not a copy of
an upstream header.

## Mount sequence

1. Read the 4096-byte primary superblock at logical/physical offset `65536`.
2. Validate the `_BHRfS_M` magic, the self-reported bytenr, sector/node sizes,
   and (when checksum verification is enabled) the CRC32C over bytes 32–4095.
3. Decode the variable-length system chunk array. Its entries are a disk key
   followed by a chunk item and its stripe records. The first stripe is usable
   for single, duplicate, and mirrored profiles; striped/parity profiles are
   rejected until their mapping rules are implemented.
4. Use the system map to read the chunk tree named by the superblock. Chunk
   items in that tree extend the logical-to-physical map to metadata and data
   chunks.
5. Read the root tree named by the superblock. The `ROOT_ITEM` for object id 5
   (`BTRFS_FS_TREE_OBJECTID`) gives the top-level filesystem tree block, level,
   and root-directory inode id. Selecting a configured non-top-level default
   subvolume is a later mount-policy increment.

Tree block headers are 101 bytes. Leaves contain 25-byte item records; nodes
contain 33-byte key pointers. Keys are packed as little-endian
`(objectid:u64, type:u8, offset:u64)`. A leaf item offset is relative to the
first byte after the 101-byte header. Metadata checksums cover the block after
its first 32-byte checksum field.

## File lookup

- `INODE_ITEM` key type `1` stores mode at byte 52 and file size at byte 16.
- `DIR_ITEM` key type `84` stores one or more entries. Each entry is a 30-byte
  header followed by data and name bytes; the location key's object id is the
  child inode. Names are byte-exact in this layer. The name hash uses CRC32C
  with seed `~1`, but the reader falls back to a directory-range scan if a
  legacy hash does not find the requested name.
- Directory enumeration in this increment walks `DIR_ITEM` keys, so its order
  is hash order. `DIR_INDEX` insertion-order iteration is deferred to the VFS
  readdir contract.
- `EXTENT_DATA` key type `108` is keyed by file offset. Inline extents use the
  first 21 bytes of the item as a fixed header and store the payload after it;
  `ram_bytes` is the inline length. Regular extents map the requested range
  through `disk_bytenr + offset`; holes and preallocation read as zeroes.
- Compression, encryption, RAID0/10/5/6, and RAID1C3/1C4 are explicit
  `ENOTSUP` results. The reader never returns encoded or incorrectly mapped
  bytes as if they were plain file data.

## Current boundary

The mount and file reader are read-only and top-level-root only. They verify
superblock and metadata-tree CRC32C checksums. Data checksum-tree verification,
subvolume selection, compressed extents, and full multi-stripe mapping are
separate increments; callers can still refuse a filesystem with an unsupported
metadata checksum type by leaving verification enabled.

The B1 transaction primitive accepts a caller-owned allocator and write/flush
callbacks. It writes a replacement metadata block at a fresh aligned address,
updates its bytenr and CRC32C, flushes that block, then stages and flushes a
eligible superblock mirrors, then the primary-superblock root pointer. A failed
or aborted transaction never publishes the root. It deliberately does not claim
full file-write support yet:
directory/file-item mutation, extent accounting, mirrored superblocks, and free
space-tree updates remain required before general writes are enabled.

The writer also exposes a bounded fixed-size leaf-item replacement operation. It
validates the existing item, preserves sorted key order, writes a fresh leaf
block with a new checksum, and leaves root publication to the transaction
caller. Variable-size insert/delete and extent/free-space accounting remain
outside this operation.

Facts were checked against the public Linux/btrfs-progs on-disk structure
definitions, especially `include/uapi/linux/btrfs_tree.h`; no upstream code is
copied into LoOS.
