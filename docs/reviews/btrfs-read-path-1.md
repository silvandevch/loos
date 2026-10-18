# Review: btrfs read path (task 14), round 1 - REJECT, 3 blocking bugs

Reviewer: claude. Files: `src/fs/btrfs/btrfs.c`, `tests/fs/btrfs/make_fixture.py`.
Verdict: the reader cannot mount a real btrfs filesystem. Its own test only passed
before because the fixture generator repeats the same mistakes.

Method: `mkfs.btrfs -q -f --rootdir <dir> img` (btrfs-progs is installed here) gives a
genuine image; the unpatched reader returns `-8 ECORRUPT` on it. With the three fixes
below (patch: `docs/reviews/btrfs-read-path-1.patch`, applied to a copy, not to your
file) it mounts, lists `/` and reads `/hello.txt` byte-exact.

## Blocking
1. **`BTRFS_SUPER_SYS_CHUNK_ARRAY` is 803, must be 811 (0x32b).** After `metadata_uuid`
   (571) come `nr_global_roots` (8) and `reserved[27]` (216) = 240 bytes, not 232.
   Correct: `BTRFS_SUPER_METADATA_UUID + 16U + 8U + 216U`.
2. **Leaf item data offsets are relative to the end of the node header, not the node
   start.** `.data = block + data_offset` must be `block + BTRFS_HEADER_SIZE +
   data_offset`, and the bounds check becomes `data_offset <= nodesize-101`,
   `data_size <= nodesize-101-data_offset`, `data_offset >= item_count*25` (when
   size != 0). Symptom on a real image: garbage chunk items (length 2^56, num_stripes 0).
3. **Directory name hash is wrong.** btrfs uses the raw (non-inverting) kernel
   `crc32c(~1, name)`; your `crc32c()` pre- and post-inverts. Known value:
   `btrfs_name_hash("default") = 0x8dbfc2d2` (DIR_ITEM key offset seen in every root
   tree); yours gives `0x3681a8bf`. Fix: `hash = (uint32_t)~crc32c(1U, name, len)`
   (equivalently a raw update with init 0xFFFFFFFE). Every by-name lookup on a real
   volume would miss. The superblock/node checksum (init ~0, final ~) is correct.

## Required follow-ups
- Regenerate fixtures to the real layout (items 1-3) and **add a real-image test**:
  `mkfs.btrfs --rootdir` into a temp dir (skip if mkfs.btrfs is absent), mount with
  checksums on, list, read, plus a file bigger than one node and a subdirectory.
- DUP/single stripe handling is first-stripe-only: fine for reads, but say so in
  docs/BTRFS.md (mirror verification/fallback is not done).
- Check `READDIR` uses DIR_INDEX (type 96) ordering or document that listing order
  is by hash; GNU-style readdir stability matters for the shell.
