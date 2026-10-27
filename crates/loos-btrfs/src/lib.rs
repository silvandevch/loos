//! Safe, allocation-free Btrfs on-disk primitives.
//!
//! The crate deliberately stops at format parsing.  It does not own a block
//! device, mount state, or the existing C filesystem ABI, so it can be used by
//! a kernel, an offline checker, or a host-side test without pulling in `std`.
#![no_std]

pub mod checksum;
pub mod digests;
pub mod ondisk;
pub mod reader;
pub mod validate;

pub use checksum::{crc32c, crc32c_raw, verify_crc32c_block, write_crc32c_block, ChecksumError};
pub use digests::{digest, verify, ChecksumAlgorithm, Digest};
pub use ondisk::{
    BtrfsHeader, BtrfsKey, BtrfsLeafItem, BtrfsNodePtr, BtrfsSuperblock, BtrfsTreeBlock,
    ChunkItem, ChunkStripe, ParseError, SystemChunkArray, BTRFS_MAGIC, BTRFS_SUPERBLOCK_OFFSET,
    BTRFS_SUPERBLOCK_SIZE, BTRFS_SYSTEM_CHUNK_ARRAY_OFFSET,
};
pub use reader::{
    BtrfsReader, DirEntry, DirEntrySink, FileStat, ReadAt, ReadError, ReaderError, SliceReader,
};
pub use validate::{validate_image, ImageValidation, ImageValidationError, BTRFS_IMAGE_MIN_SIZE};
