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
