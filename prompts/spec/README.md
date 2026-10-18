# spec/ — interface facts (prose, no code)

Written notes that agents implement from, so nobody has to re-derive the same
interface. You are free to **read the original source** while writing these
(see [`../../docs/SOURCE-POLICY.md`](../../docs/SOURCE-POLICY.md)); just record
the *fact*, not copied code, and compile the fact itself: what it does, its
inputs/outputs, its edge cases.

## What goes here

- API signatures as *interface*: params, return/status codes, semantics.
- Structures/ABI, enum values needed for interop, on-disk/format layouts.
- Behavioral contracts: "mapping a section with `SEC_IMAGE` validates the PE
  header and fails `STATUS_INVALID_IMAGE_FORMAT` on mismatch".

## What doesn't

- Large verbatim source dumps. A fact can be *learned* from the source, but write
  it in your own words. If you must quote a small value/table, cite the file and
  add a `docs/PROVENANCE.md` row.

## Suggested files (create as you go)

| Spec | Covers |
|---|---|
| `pe-coff.md` | image format, sections, imports/exports, relocations, TLS |
| `nt-syscalls.md` | the documented syscall contract (names, args, statuses) |
| `kernel32-file.md` | file/process/memory/sync core entry points |
| `apisets-core-file.md` | `api-ms-win-core-file-*` surface |
| `apisets-crt.md` | `api-ms-win-crt-*` / UCRT surface |
| `registry-hive.md` | hive format + key/value semantics |
| `btrfs-on-disk.md` | btrfs structures LoOS needs (see `../../docs/BTRFS.md`) |
| `compat-versions.md` | which reported version each app era expects |
| `win32-user-gdi.md` | window/message/GDI contracts |

Read the source, write the fact. That's the whole job.
