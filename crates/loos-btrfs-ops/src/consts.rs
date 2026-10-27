//! On-disk constants (objectids, key types, flags) from the btrfs format.

// ---- reserved object ids
pub const ROOT_TREE_OBJECTID: u64 = 1;
pub const EXTENT_TREE_OBJECTID: u64 = 2;
pub const CHUNK_TREE_OBJECTID: u64 = 3;
pub const DEV_TREE_OBJECTID: u64 = 4;
pub const FS_TREE_OBJECTID: u64 = 5;
pub const ROOT_TREE_DIR_OBJECTID: u64 = 6;
pub const CSUM_TREE_OBJECTID: u64 = 7;
pub const QUOTA_TREE_OBJECTID: u64 = 8;
pub const UUID_TREE_OBJECTID: u64 = 9;
pub const FREE_SPACE_TREE_OBJECTID: u64 = 10;
pub const BLOCK_GROUP_TREE_OBJECTID: u64 = 11;
pub const RAID_STRIPE_TREE_OBJECTID: u64 = 12;
pub const DEV_STATS_OBJECTID: u64 = 0;
pub const BALANCE_OBJECTID: u64 = (-4i64) as u64;
pub const ORPHAN_OBJECTID: u64 = (-5i64) as u64;
pub const TREE_LOG_OBJECTID: u64 = (-6i64) as u64;
pub const TREE_LOG_FIXUP_OBJECTID: u64 = (-7i64) as u64;
pub const TREE_RELOC_OBJECTID: u64 = (-8i64) as u64;
pub const DATA_RELOC_TREE_OBJECTID: u64 = (-9i64) as u64;
pub const EXTENT_CSUM_OBJECTID: u64 = (-10i64) as u64;
pub const FREE_SPACE_OBJECTID: u64 = (-11i64) as u64;
pub const FREE_INO_OBJECTID: u64 = (-12i64) as u64;
pub const MULTIPLE_OBJECTIDS: u64 = (-255i64) as u64;
pub const FIRST_FREE_OBJECTID: u64 = 256;
pub const LAST_FREE_OBJECTID: u64 = (-256i64) as u64;
pub const DEV_ITEMS_OBJECTID: u64 = 1;
pub const FIRST_CHUNK_TREE_OBJECTID: u64 = 256;

// ---- key types
pub const INODE_ITEM_KEY: u8 = 1;
pub const INODE_REF_KEY: u8 = 12;
pub const INODE_EXTREF_KEY: u8 = 13;
pub const XATTR_ITEM_KEY: u8 = 24;
pub const VERITY_DESC_ITEM_KEY: u8 = 36;
pub const VERITY_MERKLE_ITEM_KEY: u8 = 37;
pub const ORPHAN_ITEM_KEY: u8 = 48;
pub const DIR_LOG_ITEM_KEY: u8 = 60;
pub const DIR_LOG_INDEX_KEY: u8 = 72;
pub const DIR_ITEM_KEY: u8 = 84;
pub const DIR_INDEX_KEY: u8 = 96;
pub const EXTENT_DATA_KEY: u8 = 108;
pub const EXTENT_CSUM_KEY: u8 = 128;
pub const ROOT_ITEM_KEY: u8 = 132;
pub const ROOT_BACKREF_KEY: u8 = 144;
pub const ROOT_REF_KEY: u8 = 156;
pub const EXTENT_ITEM_KEY: u8 = 168;
pub const METADATA_ITEM_KEY: u8 = 169;
pub const EXTENT_OWNER_REF_KEY: u8 = 172;
pub const TREE_BLOCK_REF_KEY: u8 = 176;
pub const EXTENT_DATA_REF_KEY: u8 = 178;
pub const SHARED_BLOCK_REF_KEY: u8 = 182;
pub const SHARED_DATA_REF_KEY: u8 = 184;
pub const BLOCK_GROUP_ITEM_KEY: u8 = 192;
pub const FREE_SPACE_INFO_KEY: u8 = 198;
pub const FREE_SPACE_EXTENT_KEY: u8 = 199;
pub const FREE_SPACE_BITMAP_KEY: u8 = 200;
pub const DEV_EXTENT_KEY: u8 = 204;
pub const DEV_ITEM_KEY: u8 = 216;
pub const CHUNK_ITEM_KEY: u8 = 228;
pub const RAID_STRIPE_KEY: u8 = 230;
pub const QGROUP_STATUS_KEY: u8 = 240;
pub const QGROUP_INFO_KEY: u8 = 242;
pub const QGROUP_LIMIT_KEY: u8 = 244;
pub const QGROUP_RELATION_KEY: u8 = 246;
pub const BALANCE_ITEM_KEY: u8 = 248;
pub const DEV_STATS_KEY: u8 = 249;
pub const DEV_REPLACE_KEY: u8 = 250;
pub const UUID_KEY_SUBVOL: u8 = 251;
pub const UUID_KEY_RECEIVED_SUBVOL: u8 = 252;
pub const STRING_ITEM_KEY: u8 = 253;

// ---- block group / chunk type flags
pub const BLOCK_GROUP_DATA: u64 = 1 << 0;
pub const BLOCK_GROUP_SYSTEM: u64 = 1 << 1;
pub const BLOCK_GROUP_METADATA: u64 = 1 << 2;
pub const BLOCK_GROUP_RAID0: u64 = 1 << 3;
pub const BLOCK_GROUP_RAID1: u64 = 1 << 4;
pub const BLOCK_GROUP_DUP: u64 = 1 << 5;
pub const BLOCK_GROUP_RAID10: u64 = 1 << 6;
pub const BLOCK_GROUP_RAID5: u64 = 1 << 7;
pub const BLOCK_GROUP_RAID6: u64 = 1 << 8;
pub const BLOCK_GROUP_RAID1C3: u64 = 1 << 9;
pub const BLOCK_GROUP_RAID1C4: u64 = 1 << 10;
pub const BLOCK_GROUP_PROFILE_MASK: u64 = BLOCK_GROUP_RAID0
    | BLOCK_GROUP_RAID1
    | BLOCK_GROUP_RAID1C3
    | BLOCK_GROUP_RAID1C4
    | BLOCK_GROUP_RAID5
    | BLOCK_GROUP_RAID6
    | BLOCK_GROUP_DUP
    | BLOCK_GROUP_RAID10;
pub const BLOCK_GROUP_TYPE_MASK: u64 = BLOCK_GROUP_DATA | BLOCK_GROUP_SYSTEM | BLOCK_GROUP_METADATA;

// ---- file extent items
pub const FILE_EXTENT_INLINE: u8 = 0;
pub const FILE_EXTENT_REG: u8 = 1;
pub const FILE_EXTENT_PREALLOC: u8 = 2;
pub const COMPRESS_NONE: u8 = 0;
pub const COMPRESS_ZLIB: u8 = 1;
pub const COMPRESS_LZO: u8 = 2;
pub const COMPRESS_ZSTD: u8 = 3;

// ---- inode flags
pub const INODE_NODATASUM: u64 = 1 << 0;
pub const INODE_NODATACOW: u64 = 1 << 1;
pub const INODE_READONLY: u64 = 1 << 2;
pub const INODE_NOCOMPRESS: u64 = 1 << 3;
pub const INODE_PREALLOC: u64 = 1 << 4;
pub const INODE_SYNC: u64 = 1 << 5;
pub const INODE_IMMUTABLE: u64 = 1 << 6;
pub const INODE_APPEND: u64 = 1 << 7;
pub const INODE_NODUMP: u64 = 1 << 8;
pub const INODE_NOATIME: u64 = 1 << 9;
pub const INODE_DIRSYNC: u64 = 1 << 10;
pub const INODE_COMPRESS: u64 = 1 << 11;

// ---- dir item file types
pub const FT_UNKNOWN: u8 = 0;
pub const FT_REG_FILE: u8 = 1;
pub const FT_DIR: u8 = 2;
pub const FT_CHRDEV: u8 = 3;
pub const FT_BLKDEV: u8 = 4;
pub const FT_FIFO: u8 = 5;
pub const FT_SOCK: u8 = 6;
pub const FT_SYMLINK: u8 = 7;
pub const FT_XATTR: u8 = 8;

// ---- root item flags
pub const ROOT_SUBVOL_RDONLY: u64 = 1 << 0;
pub const ROOT_SUBVOL_DEAD: u64 = 1 << 48;

// ---- superblock feature bits
pub const COMPAT_RO_FREE_SPACE_TREE: u64 = 1 << 0;
pub const COMPAT_RO_FREE_SPACE_TREE_VALID: u64 = 1 << 1;
pub const COMPAT_RO_VERITY: u64 = 1 << 2;
pub const COMPAT_RO_BLOCK_GROUP_TREE: u64 = 1 << 3;
pub const INCOMPAT_MIXED_BACKREF: u64 = 1 << 0;
pub const INCOMPAT_DEFAULT_SUBVOL: u64 = 1 << 1;
pub const INCOMPAT_MIXED_GROUPS: u64 = 1 << 2;
pub const INCOMPAT_COMPRESS_LZO: u64 = 1 << 3;
pub const INCOMPAT_COMPRESS_ZSTD: u64 = 1 << 4;
pub const INCOMPAT_BIG_METADATA: u64 = 1 << 5;
pub const INCOMPAT_EXTENDED_IREF: u64 = 1 << 6;
pub const INCOMPAT_RAID56: u64 = 1 << 7;
pub const INCOMPAT_SKINNY_METADATA: u64 = 1 << 8;
pub const INCOMPAT_NO_HOLES: u64 = 1 << 9;
pub const INCOMPAT_METADATA_UUID: u64 = 1 << 10;
pub const INCOMPAT_RAID1C34: u64 = 1 << 11;
pub const INCOMPAT_ZONED: u64 = 1 << 12;
pub const INCOMPAT_EXTENT_TREE_V2: u64 = 1 << 13;
pub const INCOMPAT_RAID_STRIPE_TREE: u64 = 1 << 14;
pub const INCOMPAT_SIMPLE_QUOTA: u64 = 1 << 16;
pub const INCOMPAT_KNOWN: u64 = INCOMPAT_MIXED_BACKREF
    | INCOMPAT_DEFAULT_SUBVOL
    | INCOMPAT_MIXED_GROUPS
    | INCOMPAT_COMPRESS_LZO
    | INCOMPAT_COMPRESS_ZSTD
    | INCOMPAT_BIG_METADATA
    | INCOMPAT_EXTENDED_IREF
    | INCOMPAT_RAID56
    | INCOMPAT_SKINNY_METADATA
    | INCOMPAT_NO_HOLES
    | INCOMPAT_METADATA_UUID
    | INCOMPAT_RAID1C34
    | INCOMPAT_SIMPLE_QUOTA;

pub const STRIPE_LEN: u64 = 64 * 1024;
pub const SUPER_MIRROR_OFFSETS: [u64; 3] = [64 << 10, 64 << 20, 256 << 30];
pub const SUPER_INFO_SIZE: usize = 4096;
pub const HEADER_SIZE: usize = 101;
pub const ITEM_SIZE: usize = 25;
pub const KEY_PTR_SIZE: usize = 33;
