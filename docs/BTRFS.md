# LoOS — filesystem: btrfs, and only btrfs

**btrfs is the only filesystem you choose.** The installer offers no other
option for root, home, or data: every LoOS volume is btrfs. NTFS is *not* used
as a system filesystem, and there is no ext4/exFAT/FAT choice for data — btrfs
replaces all of them. This is deliberate: btrfs gives checksums, copy-on-write,
snapshots, subvolumes, compression, RAID and online growth in one filesystem,
with none of NTFS's proprietary, patent-encumbered corner cases.

The only non-btrfs storage in a LoOS system is unavoidable and hidden from the
user's choice:

- **FAT32** — the EFI System Partition (UEFI requirement) and tiny boot tooling.
- **NTFS, read-only** — an *optional* mount so users can read old Windows disks.
  Never writable, never a system FS. (See `btrfs-ntfsro` / `ntfs`.)

## The goal: the full btrfs feature set, 100% stable

LoOS implements **all** of btrfs, not a subset, and it must be **100% stable**:
no data loss, no corruption, clean crash consistency, and `btrfs check` clean
after every operation, including power-loss and fault injection. Every feature
below is a task; stability is a task of its own and a gate on all of them.

| Feature | What it means | Task |
|---|---|---|
| **Copy-on-write** | never overwrite live data; new extents + new tree root | `btrfsw`, `btrfs-cow` |
| **Checksums** | every data + metadata block checksummed (crc32c, xxhash64/xxh3, sha256, blake2), verified on read, repaired from a good mirror | `btrfs-csum` |
| **Snapshots** | read-only and writable snapshots, atomic, instant | `btrfs-snap` |
| **Subvolumes** | nested subvolumes, default subvolume, `readonly`/`nodatacow` flags | `btrfs-subvol` |
| **Send / receive** | incremental, streamed, deduplicated backup/restore | `btrfs-sendreceive` |
| **Compression** | zstd, zlib, lzo, lz4, per-file and mount-level, with heuristics | `btrfs-compress` |
| **RAID** | raid0, raid1, raid1c3, raid1c4, raid10, raid5, raid6 (write-hole-correct) | `btrfs-raid` |
| **Devices** | multi-device, add/remove/replace, seed devices | `btrfs-device` |
| **Balance** | rebalance data/metadata/system chunks across devices | `btrfs-balance` |
| **Scrub** | verify all checksums, auto-repair from mirrors | `btrfs-scrub` |
| **Free-space tree** | space cache v2 / free-space tree, fast mount | `btrfs-freestree` |
| **Quotas** | qgroups, subvolume quotas, per-user accounting | `btrfs-qgroup` |
| **Defrag** | online + autodefrag, compression-aware | `btrfs-defrag` |
| **Dedupe** | block/ref deduplication | `btrfs-dedupe` |
| **Reflink** | `FICLONE`/clone_range, `cp --reflink` — instant CoW copies | `btrfs-reflink` |
| **Preallocation** | `fallocate` modes, sparse files, inline extents | `btrfs-prealloc` |
| **xattrs / ACLs / caps** | full POSIX xattr, ACL and capability support | `btrfs-xattr` |
| **Trim** | discard/trim, SSD + zoned-device support | `btrfs-trim` |
| **Swap** | btrfs swapfile support (CoW-safe) | `btrfs-swap` |
| **Online resize** | grow/shrink while mounted | `btrfs-resize` |
| **ioctl surface** | the full btrfs ioctl set apps/tools expect | `btrfs-ioctl` |
| **Tools** | `check`, `restore`, `rescue` (btrfs-progs equivalent) | `btrfs-check` |
| **Boot** | boot the kernel from btrfs | `btrfs-boot` |
| **Stability** | fuzz + power-loss + soak + crash consistency, `check`-clean | `btrfs-stab` |

## Windows-compatible behaviour over btrfs

Windows apps call the Win32 file APIs, which we implement *below* on btrfs. Only
a few NTFS-specific behaviours need a mapping layer:

| Apps expect | How LoOS provides it on btrfs |
|---|---|
| Paths, case-insensitivity | case-insensitive lookup layer in the VFS for Win32 callers (case preserved) |
| Alternate Data Streams (`file:stream`) | sidecar subvolume / xattrs (documented mapping) |
| Reparse points / junctions / symlinks | btrfs subvolumes + symlinks + reparse metadata in xattrs |
| ACLs / security descriptors | NT security descriptors ↔ POSIX ACLs + xattrs, Win32 model at the API layer |
| File IDs / `FILE_ID_BOTH_DIR_INFO` | synthesized from btrfs inode numbers |
| NTFS compression | btrfs zstd transparent compression |
| Encryption (EFS/BitLocker) | btrfs-on-LUKS full-volume encryption (BitLocker not implemented) |
| Previous versions | native btrfs snapshots + send/receive |

**Design rule:** keep the **Win32 file API** stable (the compatibility contract)
and implement everything *below* it on btrfs.

## Implementation: borrow, don't reinvent

btrfs is a Linux filesystem with a complete, mature implementation; this is
exactly where [SOURCE-POLICY.md](SOURCE-POLICY.md)'s "borrow from open source"
applies.

- Port/adapt the btrfs on-disk logic from the **Linux kernel** (GPL-2.0) or a
  userspace btrfs library, behind our VFS. **This makes the LoOS filesystem
  component GPL** — record it in `THIRD-PARTY.md`.
- Or implement a clean core from the on-disk format spec
  ([btrfs.readthedocs.io](https://btrfs.readthedocs.io/)) if a permissive
  license is required. Large effort; decide the OS license first.

> **License decision needed** — see the note in [LANGUAGE.md](LANGUAGE.md).
> **[you]** pick the project-wide license; it shapes this whole section.

## Current state (honest)

Read-only: `src/fs/btrfs/` validates the primary superblock and metadata CRC32C,
bootstraps chunk maps, resolves the root tree, verifies CRC32C data extents,
and reads inline/regular uncompressed extents; the VFS adapter provides
lookup/stat/readdir/open/read. The Rust `loos-btrfs` parser now provides
allocation-free Btrfs digest encodings for CRC32C, XXH64, SHA-256, and
BLAKE2b-256, plus a bounded read-only image reader with checksum verification,
SINGLE/DUP mapping, mirror fallback, sparse reads, and subvolume selection.
The C mounted reader still rejects non-CRC32C volumes until that digest ABI is
wired into the kernel reader.
Write: the first B1 primitives (`loos_btrfs_write_transaction_*`) do
allocate→COW→checksum→flush with a new root published last. A bounded leaf-item
replacement primitive now COWs fixed-size inode, directory, extent, or checksum
items while validating key ordering; it provides the mutation building block but
does **not** yet update dir/file relationships, extent accounting, or free-space
metadata. It is not a complete filesystem write path, and snapshot/clone
publication is intentionally not exposed as a successful operation. Striped/parity chunks,
compressed/encrypted extents, subvolumes, and non-CRC32C mounted checksum
algorithms are not done. Everything else in the table above is **planned**.

The host-only `loos_btrfs_tools` library provides a deliberately limited
integrity slice: `loos_btrfs_integrity_check` validates CRC32C primary and
standard mirror superblocks, the chunk mappings accepted by the reader, and the
chunk/root/filesystem metadata paths that the reader can reach. Its rescue
helper can restore only the primary superblock from a valid mirror. The Rust
`loos-btrfs-ops` crate additionally has tested, read-only/planning slices for
`check`, `restore`, `rescue`, scrub/parity recovery, ioctl codecs, xattrs/ACLs,
trim, resize, dedupe, defrag, and swapfile mapping. These are not equivalent
to a complete mounted Btrfs implementation: full COW mutation, snapshots,
send/receive, compression integration, and multi-device write support remain
open.

Snapshot metadata planning is explicit in `loos-btrfs-ops::snapshot`: it
validates source/destination identity, generation monotonicity, UUID uniqueness,
and root geometry, then prepares read-only or writable root-item state with
parent UUID linkage. Device mutation and atomic root-tree publication still
belong to the mounted COW writer.

`loos-btrfs-ops::subvolume` centralizes nested-subvolume validation: free object
id range, nonzero parent/directory identity, bounded names without slash/NUL,
and nonzero UUIDs are checked before metadata allocation. It is shared policy;
the actual root-tree, parent `DIR_ITEM`, and `ROOT_REF`/`ROOT_BACKREF` writes
remain transaction work.

## Milestones

- **B0** read-only mount (root subvolume, uncompressed) + list/read — *mostly done*
- **B1** complete read-write: dir/file items, extent + free-space accounting, CoW, checksums, flush
- **B2** subvolumes + snapshots + send/receive
- **B3** compression, xattr/ACL mapping, case-insensitive Win32 layer, ADS sidecar
- **B4** RAID profiles + multi-device + balance + scrub
- **B5** quotas, defrag, dedupe, reflink, prealloc, trim, swap, resize, ioctl
- **B6** tools (`check`/`restore`/`rescue`) + boot-from-btrfs — *tool slices done; boot open*
- **B7** optional read-only NTFS for legacy disks — *done as compatibility reader*
- **S** the stability program, gating every milestone above

`astra` owns the on-disk core; `sol` takes tools and the Win32 mapping layer.
See [ARCHITECTURE.md](ARCHITECTURE.md).

## Tasks

`btrfsr`, `btrfsw`, `btrfs-cow`, `btrfs-csum`, `btrfs-snap`, `btrfs-subvol`,
`btrfs-sendreceive`, `btrfs-compress`, `btrfs-raid`, `btrfs-device`,
`btrfs-balance`, `btrfs-scrub`, `btrfs-freestree`, `btrfs-qgroup`,
`btrfs-defrag`, `btrfs-dedupe`, `btrfs-reflink`, `btrfs-prealloc`,
`btrfs-xattr`, `btrfs-trim`, `btrfs-swap`, `btrfs-resize`, `btrfs-ioctl`,
`btrfs-check`, `btrfs-boot`, `btrfs-stab`, `ntfs` (read-only), `fatfs` (ESP).

## Quota groups and balance planning (`loos-btrfs-ops`, task #127)
- `qgroup` – reads the quota tree (status/info/limit/relation), `violations()` lists exceeded limits, `would_exceed(qgroup, extra)` checks a write against the group and all ancestors (cycle-safe). Subvolume/snapshot listing is `Fs::subvolumes()`.
- `balance` – `read_usage()` reads block-group usage; `plan()` selects groups by type/usage%/device/profile-conversion, emptiest first, with `bytes_to_move`; `feasible()` checks unallocated space. Relocation itself belongs to the write path (not yet implemented). Test: `cargo +1.97.1 test -p loos-btrfs-ops --features std --test qgroup_balance`.
