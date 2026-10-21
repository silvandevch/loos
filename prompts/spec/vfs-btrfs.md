# LoOS btrfs VFS contract

The first VFS layer is an explicit adapter over `loos_btrfs_mount`; it does not
reparse paths for every operation or expose internal tree buffers.

- `loos_btrfs_lookup(mount, directory_inode, name, entry, stat)` resolves one
  byte-exact component and returns both the directory entry and the child
  metadata. A non-directory parent returns `ENOTDIR`.
- `loos_btrfs_stat_path` resolves a path and returns inode, size, mode, uid,
  gid, nlink, and inode flags.
- `loos_btrfs_readdir` enumerates `DIR_ITEM` entries with a resumable ordinal
  cookie. The current order is the on-disk hash order; the caller stores the
  returned cookie and passes it back to resume. `DIR_INDEX` ordering is a later
  compatibility increment.
- `loos_btrfs_open_inode` pins the mount pointer and snapshots the inode
  metadata into a caller-owned file object. `loos_btrfs_read_inode` reads by
  inode without a second path lookup and returns holes as zeroes.
- Xattr mutation is rejected with `EROFS` while mounts are read-only. Xattr
  decoding is not silently faked: unsupported xattr reads return `ENOTSUP` until
  the on-disk xattr item parser and the write transaction can preserve values.

The adapter keeps the block-device callback model. It performs no network I/O,
uses no process-global state, and leaves case-insensitive lookup to the Win32
compatibility layer rather than changing native btrfs byte-exact semantics.
