# LoOS — filesystem: btrfs (not NTFS)

The native, read/write filesystem of LoOS is **btrfs**. NTFS is not the system
filesystem. This is a deliberate choice: btrfs gives checksums, snapshots,
subvolumes, compression, and online growth, and avoids NTFS's proprietary,
patent-encumbered corner cases.

## What this means

- **System volume (root)** is btrfs. Bootloader loads the kernel from btrfs;
  the kernel mounts a btrfs subvolume as `/`.
- **FAT32** remains for the EFI System Partition and small boot tooling.
- **NTFS is not used as the system FS.** For convenience, legacy NTFS volumes
  may be mounted **read-only** (so users can read old disks), implemented later;
  never required.

## Introducing btrfs to a Windows-compatible OS

Windows apps generally don't care about the filesystem — they call the Win32
file APIs, which we implement over btrfs. Only a few NTFS-specific features need
a compatibility layer:

| Feature apps might expect | How we provide it on btrfs |
|---|---|
| Paths, case-insensitivity | btrfs is case-sensitive; present a case-insensitive lookup layer in the VFS for Win32 callers (and preserve case). |
| Alternate Data Streams (`file:stream`) | btrfs has no ADS; store named streams as adjacent xattrs or a hidden sidecar subvolume. Document the mapping. |
| Reparse points / junctions / symlinks | map to btrfs subvolumes/symlinks + our reparse metadata via xattrs. |
| ACLs / security descriptors | map NT security descriptors to POSIX ACLs + xattrs; keep the Win32 model at the API layer. |
| File IDs / `FILE_ID_BOTH_DIR_INFO` | synthesize from btrfs inode numbers. |
| Compression (`NTFS compression`) | use btrfs `zstd` transparent compression; expose as "compress this file". |
| Encryption (EFS/BitLocker) | use btrfs/LUKS full-volume encryption; BitLocker is not implemented. |
| Snapshots / previous versions | native btrfs snapshots + send/receive — better than VSS. |
| Sparse files, preallocation | btrfs supports both natively. |

**Design rule:** keep the **Win32 file API** stable (that's the compatibility
contract) and implement *below* it on btrfs. Apps see normal Windows semantics;
the disk underneath is btrfs.

## Bootloader requirement

The loader must be able to read btrfs well enough to find and load the kernel
(superblock → root tree → default subvolume → kernel path). Options:

1. A minimal btrfs reader in the loader (read-only, no compression-at-boot, or
   zstd support if the kernel image is compressed).
2. A small FAT32 boot partition that holds kernel + initramfs; the running
   system's root is btrfs. (Simplest, recommended first.)

## Implementation: borrow, don't reinvent

A filesystem is exactly where [SOURCE-POLICY.md](SOURCE-POLICY.md)'s "borrow
from open source" applies. btrfs is a Linux filesystem with a complete, mature
implementation. The realistic path:

- Port/adapt the btrfs on-disk logic from the **Linux kernel** (GPL-2.0) or from
  a userspace btrfs library, behind our VFS interface. **This makes the LoOS
  filesystem component GPL** — record it in `THIRD-PARTY.md`.
- Alternatively implement a clean btrfs core from the on-disk format spec
  ([btrfs docs / wiki](https://btrfs.readthedocs.io/)) if a non-GPL license is
  required for the whole OS. This is a large effort; decide the OS license
  first.

> **License decision needed.** If LoOS is GPL, borrow the Linux btrfs code.
> If LoOS must be permissive (MIT/Apache), the btrfs driver is either
> written from the spec or isolated as a separate GPL component the user links.
> **[you]** pick the project-wide license; it changes this whole section.

## Milestones

- B0: read-only btrfs mount (root subvolume, uncompressed) + list/read files.
- B1: write path (allocate, COW, checksum, flush).
- B2: subvolumes + snapshots + send/receive.
- B3: compression (zstd), xattr mapping, case-insensitive Win32 layer, ADS
  sidecar, ACL/security-descriptor mapping.
- B4: boot-from-btrfs (loader reader or FAT boot partition).
- B5: optional read-only NTFS for legacy disks.

`astra` owns this. See [ARCHITECTURE.md](ARCHITECTURE.md).
