# LoOS registry hive

The registry service stores settings in the classic Windows hive format while
keeping the implementation independent from Win32. `crates/loos-registry` is a
`no_std` + `alloc` parser/writer for the `regf` base block and `hbin` cells.

## On-disk contract

- The base block is 4096 bytes and begins with `regf`.
- Hive bins begin at file offset `0x1000`; every bin is a 4096-byte multiple
  beginning with `hbin`.
- Cell offsets are relative to the beginning of the bin region, and allocated
  cells have a negative signed size. Key (`nk`), value (`vk`), and subkey-list
  (`li`, `lf`, `lh`, `ri`) cells are parsed with bounds checks.
- Key and value names may use the registry's compressed single-byte form or
  UTF-16LE. Unknown value type numbers are retained as `Other(u32)` rather than
  discarded.

The loader caps images at 64 MiB, limits key depth to 64 and list cardinality
to one million, rejects cycles and out-of-range cells, and verifies a non-zero
base-block checksum. The writer emits a canonical single-bin image with valid
checksum and frees named objects when the last in-memory reference disappears.

## API semantics

`Hive::key`, `Hive::value`, `Hive::create_key`, `Hive::set_value`, and
`Hive::delete_value` use case-insensitive path components separated by `\\` or
`/`. Empty path addresses the root; `.` and `..` are rejected. `save` rebuilds
the cell layout from the in-memory tree, so deleted values cannot remain as
reachable stale cells.

The Win32 `advapi32` layer will map `HKEY_*` roots and `REG_*` types onto this
model. Hive transaction logging and registry security descriptors are separate
follow-up work; the base hive remains parseable and writable without them.


## Registry service and btrfs persistence (`service.rs`, `vfs_store.rs`)

`Registry<S: HiveStorage>` unifies hives into one namespace: `HKLM`→`MACHINE\{SYSTEM,SOFTWARE,SAM,SECURITY}`,
`HKU`→`USER\<sid>`, `HKCU`→`USER\<current_user>`, `HKCR`→`MACHINE\SOFTWARE\Classes`.
Longest-mount routing, `create/delete_key`, typed value get/set, enumeration, `load_hive`/`unload_hive`
(per-user `NTUSER.DAT`). A corrupt hive fails `open` (never silently replaced).

Persistence (`--features vfs`): `VfsStorage` writes `<name>.new`, flushes, then `rename`s over the
old file (CoW-atomic on btrfs; `loos-vfs` gained `FileSystem::create_file`/`rename` and
`MountManager::{create_file,rename,flush_path}`, default `Unsupported`). Shrinking hives leave no
stale tail. Tests: `cargo test -p loos-registry --features vfs`.

`create_file`/`rename` are implemented for the FAT adapter (`FatAdapter`, tested against `mkfs.fat`; rename replaces the destination). There is no btrfs `FileSystem` adapter yet (`loos-btrfs*` expose a reader and planners only), so hive persistence on btrfs needs that adapter to implement these two methods.
