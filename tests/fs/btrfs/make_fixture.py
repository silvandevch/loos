#!/usr/bin/env python3
"""Build a small deterministic, checksummed btrfs image for the C tests.

This is a format fixture, not a second filesystem implementation.  It covers
the read-only path's required structures: the superblock, system and chunk
maps, root items, inode/dir items, an inline extent, and a regular extent.
"""

from __future__ import annotations

import pathlib
import struct
import sys

NODE_SIZE = 4096
IMAGE_SIZE = 0x60000
SUPER = 0x10000
CHUNK_TREE = 0x12000
ROOT_TREE = 0x13000
FS_TREE = 0x14000
DATA_LOGICAL = 0x40000

MAGIC = 0x4D5F53665248425F
FSID = bytes(range(16))

ROOT_TREE_OBJECTID = 1
CHUNK_TREE_OBJECTID = 3
FS_TREE_OBJECTID = 5
INDEPENDENT_SUBVOLUME_OBJECTID = 256
CSUM_TREE_OBJECTID = 7
EXTENT_CSUM_OBJECTID = 0xFFFFFFFFFFFFFFF6
INODE_ITEM_KEY = 1
DIR_ITEM_KEY = 84
EXTENT_DATA_KEY = 108
ROOT_ITEM_KEY = 132
CHUNK_ITEM_KEY = 228
FILE_EXTENT_INLINE = 0
FILE_EXTENT_REG = 1
INODE_NODATASUM = 1


def put16(buffer: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<H", buffer, offset, value)


def put32(buffer: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<I", buffer, offset, value)


def put64(buffer: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<Q", buffer, offset, value)


def crc32c_raw(seed: int, data: bytes | bytearray) -> int:
    crc = seed & 0xFFFFFFFF
    for value in data:
        crc ^= value
        for _ in range(8):
            mask = -(crc & 1) & 0xFFFFFFFF
            crc = ((crc >> 1) ^ (0x82F63B78 & mask)) & 0xFFFFFFFF
    return crc


def crc32c(seed: int, data: bytes | bytearray) -> int:
    return (~crc32c_raw(~seed, data)) & 0xFFFFFFFF


def disk_key(objectid: int, item_type: int, offset: int) -> bytes:
    return struct.pack("<QBQ", objectid, item_type, offset)


def name_hash(name: bytes) -> int:
    return crc32c_raw(0xFFFFFFFE, name)


def chunk_item(length: int, physical: int, devid: int = 1) -> bytes:
    # struct btrfs_chunk through the first stripe, followed by its UUID.
    header = struct.pack(
        "<QQQQIIIHH",
        length,
        CHUNK_TREE_OBJECTID,
        0x1000,
        1,  # SINGLE profile: no RAID striping
        0x1000,
        0x1000,
        0x1000,
        1,
        0,
    )
    stripe = struct.pack("<QQ", devid, physical) + bytes(16)
    return header + stripe


def tree_header(block: bytearray, bytenr: int, owner: int, level: int, count: int) -> None:
    block[32:48] = FSID
    put64(block, 48, bytenr)
    put64(block, 80, 1)
    put64(block, 88, owner)
    put32(block, 96, count)
    block[100] = level


def seal_tree(block: bytearray) -> None:
    put32(block, 0, crc32c(0, block[32:]))


def leaf(bytenr: int, owner: int, entries: list[tuple[bytes, bytes]]) -> bytearray:
    block = bytearray(NODE_SIZE)
    tree_header(block, bytenr, owner, 0, len(entries))
    data_cursor = NODE_SIZE
    for index, (key, data) in enumerate(entries):
        data_cursor -= len(data)
        block[data_cursor : data_cursor + len(data)] = data
        item_offset = 101 + index * 25
        block[item_offset : item_offset + 17] = key
        put32(block, item_offset + 17, data_cursor - 101)
        put32(block, item_offset + 21, len(data))
    seal_tree(block)
    return block


def inode(size: int, mode: int, flags: int = 0) -> bytes:
    data = bytearray(160)
    put64(data, 0, 1)
    put64(data, 8, 1)
    put64(data, 16, size)
    put32(data, 52, mode)
    put64(data, 64, flags)
    return bytes(data)


def root_item(fs_tree: int, root_dirid: int = 256, flags: int = 0) -> bytes:
    data = bytearray(439)
    put64(data, 168, root_dirid)
    put64(data, 176, fs_tree)
    put64(data, 208, flags)
    data[238] = 0
    return bytes(data)


def dir_item(name: bytes, child_inode: int, file_type: int) -> bytes:
    location = disk_key(child_inode, INODE_ITEM_KEY, 0)
    header = location + struct.pack("<QHHB", 1, 0, len(name), file_type)
    return header + name


def inline_extent(payload: bytes) -> bytes:
    data = bytearray(21 + len(payload))
    put64(data, 8, len(payload))  # ram_bytes is the inline file length
    data[20] = FILE_EXTENT_INLINE
    data[21:] = payload
    return bytes(data)


def regular_extent(logical: int, length: int) -> bytes:
    data = bytearray(53)
    put64(data, 8, length)
    data[20] = FILE_EXTENT_REG
    put64(data, 21, logical)
    put64(data, 29, length)
    put64(data, 37, 0)
    put64(data, 45, length)
    return bytes(data)


def make_image() -> bytearray:
    image = bytearray(IMAGE_SIZE)
    entries = [
        (
            disk_key(256, CHUNK_ITEM_KEY, 0x10000),
            chunk_item(0x30000, 0x10000),
        ),
        (
            disk_key(256, CHUNK_ITEM_KEY, DATA_LOGICAL),
            chunk_item(0x10000, DATA_LOGICAL),
        ),
    ]
    image[CHUNK_TREE : CHUNK_TREE + NODE_SIZE] = leaf(
        CHUNK_TREE, CHUNK_TREE_OBJECTID, entries
    )

    root_entries = [
        (disk_key(FS_TREE_OBJECTID, ROOT_ITEM_KEY, 0), root_item(FS_TREE)),
        (disk_key(CSUM_TREE_OBJECTID, ROOT_ITEM_KEY, 0), root_item(0x15000)),
        (disk_key(INDEPENDENT_SUBVOLUME_OBJECTID, ROOT_ITEM_KEY, 0),
         root_item(0x16000, flags=1)),
    ]
    image[ROOT_TREE : ROOT_TREE + NODE_SIZE] = leaf(
        ROOT_TREE, ROOT_TREE_OBJECTID, root_entries
    )

    hello = b"hello from LoOS\n"
    nested = b"nested from LoOS\n"
    regular = bytes((index * 13 + 7) & 0xFF for index in range(0x2000))
    csum_entries = [
        (
            disk_key(EXTENT_CSUM_OBJECTID, 128, DATA_LOGICAL),
            struct.pack("<II", crc32c(0, regular[:4096]),
                        crc32c(0, regular[4096:])),
        )
    ]
    image[0x15000 : 0x15000 + NODE_SIZE] = leaf(
        0x15000, CSUM_TREE_OBJECTID, csum_entries
    )
    entries = [
        (disk_key(256, INODE_ITEM_KEY, 0), inode(0, 0o40755)),
        (
            disk_key(256, DIR_ITEM_KEY, name_hash(b"hello.txt")),
            dir_item(b"hello.txt", 257, 1),
        ),
        (
            disk_key(256, DIR_ITEM_KEY, name_hash(b"regular.bin")),
            dir_item(b"regular.bin", 258, 1),
        ),
        (
            disk_key(256, DIR_ITEM_KEY, name_hash(b"nodatasum.bin")),
            dir_item(b"nodatasum.bin", 261, 1),
        ),
        (
            disk_key(256, DIR_ITEM_KEY, name_hash(b"subdir")),
            dir_item(b"subdir", 259, 2),
        ),
        (disk_key(257, INODE_ITEM_KEY, 0), inode(len(hello), 0o100644)),
        (
            disk_key(257, EXTENT_DATA_KEY, 0),
            inline_extent(hello),
        ),
        (disk_key(258, INODE_ITEM_KEY, 0), inode(len(regular), 0o100644)),
        (
            disk_key(258, EXTENT_DATA_KEY, 0),
            regular_extent(DATA_LOGICAL, len(regular)),
        ),
        (disk_key(261, INODE_ITEM_KEY, 0),
         inode(len(regular), 0o100644, INODE_NODATASUM)),
        (disk_key(261, EXTENT_DATA_KEY, 0),
         regular_extent(DATA_LOGICAL, len(regular))),
        (disk_key(259, INODE_ITEM_KEY, 0), inode(0, 0o40755)),
        (
            disk_key(259, DIR_ITEM_KEY, name_hash(b"nested.txt")),
            dir_item(b"nested.txt", 260, 1),
        ),
        (disk_key(260, INODE_ITEM_KEY, 0), inode(len(nested), 0o100644)),
        (disk_key(260, EXTENT_DATA_KEY, 0), inline_extent(nested)),
    ]
    entries.sort(key=lambda item: (struct.unpack_from("<Q", item[0])[0], item[0][8], struct.unpack_from("<Q", item[0], 9)[0]))
    image[FS_TREE : FS_TREE + NODE_SIZE] = leaf(FS_TREE, FS_TREE_OBJECTID, entries)
    image[0x16000 : 0x16000 + NODE_SIZE] = leaf(
        0x16000, INDEPENDENT_SUBVOLUME_OBJECTID, entries
    )
    image[DATA_LOGICAL : DATA_LOGICAL + len(regular)] = regular

    superblock = bytearray(NODE_SIZE)
    superblock[32:48] = FSID
    put64(superblock, 48, SUPER)
    put64(superblock, 64, MAGIC)
    put64(superblock, 72, 1)
    put64(superblock, 80, ROOT_TREE)
    put64(superblock, 88, CHUNK_TREE)
    put64(superblock, 112, IMAGE_SIZE)
    put64(superblock, 128, 6)
    put32(superblock, 144, 4096)
    put32(superblock, 148, NODE_SIZE)
    put32(superblock, 160, 17 + len(chunk_item(0x30000, 0x10000)))
    put64(superblock, 164, 1)
    put64(superblock, 172, 0)
    put64(superblock, 180, 0)
    put64(superblock, 188, 0)
    put16(superblock, 196, 0)
    superblock[198] = 0
    superblock[199] = 0
    # The metadata UUID is at offset 571 in the packed on-disk superblock.
    superblock[571:587] = FSID
    array = bytearray()
    array += disk_key(256, CHUNK_ITEM_KEY, 0x10000)
    array += chunk_item(0x30000, 0x10000)
    superblock[811 : 811 + len(array)] = array
    put32(superblock, 0, crc32c(0, superblock[32:]))
    image[SUPER : SUPER + NODE_SIZE] = superblock
    return image


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} OUTPUT", file=sys.stderr)
        return 2
    output = pathlib.Path(sys.argv[1])
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(make_image())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
