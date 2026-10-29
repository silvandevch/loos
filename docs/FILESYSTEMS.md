# LoOS — the non-btrfs filesystems (`crates/loos-fs`)

btrfs is the only filesystem a user *chooses* ([BTRFS.md](BTRFS.md)). LoOS still
has to speak a few others, and they live in one Rust crate: **`crates/loos-fs`**
(`#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`, every on-disk field treated as
hostile input). They all sit on a tiny byte-addressed `BlockDev` trait
(`read_at` / `write_at` / `flush` / `len`) with `MemDev` and a host `FileDev`.

| Module | Filesystem | Access | Used for |
|---|---|---|---|
| `fat` | FAT12 / FAT16 / FAT32 + VFAT long names | read/write | EFI System Partition, boot tooling (**not** a user choice) |
| `exfat` | exFAT (bitmap, upcase table, entry-set checksums, fragmented files) | read/write | removable-media interchange |
| `ntfs` | NTFS 1.2–3.1 | **read-only** | opening old Windows disks; never a system FS |
| `iso9660` | ISO 9660 + Joliet + Rock Ridge (+ El Torito lookup) | read-only | install media |
| `udf` | UDF 1.02–2.60 (physical, sparable, metadata partitions) | read-only | DVD/BD media |

## API shape (identical across modules)

```rust
let mut fs = Fat::mount(dev, /*byte offset of the partition*/ 0)?;   // Ntfs/ExFat same; Iso/Udf: mount(dev)
fs.read_dir("/EFI/BOOT")?;           // Vec<DirEntry>   (Iso/Udf/Ntfs return the shared `Entry`)
fs.stat("/a/b.txt")?;                // case-insensitive, `/` or `\`, `.`/`..` handled
fs.read_file(path, offset, &mut buf)?;  fs.read_all(path)?;
// FAT + exFAT only:
fs.write_file(path, data)?;  fs.write_at(path, off, data)?;  fs.truncate(path, n)?;
fs.mkdir(path)?;  fs.remove(path)?;  fs.rename(a, b)?;  fs.sync()?;
```

Errors are one `FsError` enum (`Io, NotRecognized, Corrupt, NotFound, NotDir, IsDir,
Exists, NotEmpty, NoSpace, InvalidName, Unsupported, ReadOnly`). Anything valid but not
implemented returns `Unsupported` — **never wrong bytes**: EFS-encrypted and
WOF-compressed NTFS data, UDF virtual (VAT) partitions.

## Behaviour worth knowing

- **Case**: lookups are case-insensitive (Windows semantics) with case preserved on
  write. NTFS uses the volume's own `$UpCase`; exFAT its upcase table.
- **FAT**: whole FAT cached in memory, written to every copy by `sync()`. Long names are
  generated with `~N` collision tails and NT lower-case flags; failed writes roll back.
  Tiny FAT32 volumes are detected by BPB layout, not cluster count (Linux/Windows disagree).
- **exFAT**: files are created contiguous (`NoFatChain`) and convert to a FAT chain only
  when growth is blocked. `ExFat::check()` is an offline bitmap/reachability audit.
- **NTFS**: MFT fixups, `$ATTRIBUTE_LIST` extension records, sparse runs, initialized
  size, **LZNT1 compression**, `$I30` B-tree directories (indexed lookup, linear fallback),
  alternate data streams (`file:stream`, `Ntfs::streams`), reparse symlinks/junctions,
  `Ntfs::attributes()` (DOS attribute bits), label/version/free clusters. DOS 8.3 aliases
  are hidden from listings.
- **ISO**: flavour chosen at mount (`Rock Ridge` > `Joliet` > plain); RR deep-directory
  relocation (`CL`), symlinks (`SL`), modes (`PX`), times (`TF`), multi-extent files,
  `boot_entries()` for El Torito BIOS/EFI images.
- **UDF**: short/long/extended/inline allocation descriptors with continuation extents,
  File Entry + Extended File Entry, symlinks, POSIX mode mapping.

## Why NTFS is read-only

The earlier task list had "NTFS read/write (compat)". That contradicts the standing policy
in [BTRFS.md](BTRFS.md) (*NTFS never writable, never a system FS*), and an in-OS NTFS writer
is the single most data-destroying thing we could ship. Windows apps get NTFS *behaviour*
from the Win32 layer over btrfs (case-insensitive layer, ADS sidecars, reparse metadata);
they do not need an NTFS driver. Task #128 is closed as superseded by the read-only reader.

## Tests

```bash
cd crates && cargo test -p loos-fs     # unit tests + image-based integration tests
```

Integration tests build real images with the host's own tools and use **independent
implementations as oracles**; each test skips cleanly if its tool is absent:

| Tool | Role |
|---|---|
| `mkfs.fat`, `mcopy`/`mmd`/`mtype`/`mdir`, `fsck.fat` | build FAT images; verify our writes (fsck must be clean) |
| `mkfs.exfat`, `fsck.exfat` | build/verify exFAT (our own `check()` covers the bitmap, which fsck.exfat does not) |
| `mkntfs`, `ntfscp`, `ntfscat`, `ntfsls` | build NTFS (plain, `-C` compressed, 512 B–64 KiB clusters, 500-name B-tree directory, ADS) and compare file bytes with `ntfscat` |
| `xorriso` | Rock Ridge + Joliet trees (12 levels deep, symlinks, 5 MB files), plain level-1, El Torito |
| `mkudffs` + `python3 -m pycdlib` | UDF partition layouts + a 2.60 tree |

The NTFS suite also flips random bytes in the boot sector and MFT region (300 mutated
images): mount/read must return errors, never panic.

## Not done / follow-ups

UDF VAT (virtual) partitions; NTFS `$Secure` security descriptors and `$LogFile` replay;
exFAT TexFAT/ACL entries; FAT `.` / `..` rename edge cases beyond directory moves.

## LoOS directory layout and Windows paths

LoOS uses a Linux-style tree with Windows-friendly folder names. Windows programs keep their
usual paths; `loos-vfs::winpath` translates them (C: `loos_winpath_to_loos` / `loos_winpath_to_windows`).

| LoOS | Windows programs see |
|---|---|
| `/home/<user>` (Desktop, Documents, Downloads, Music, Pictures, Videos) | `C:\Users\<user>` |
| `/home/<user>/.appdata` | `C:\Users\<user>\AppData` |
| `/home/public` | `C:\Users\Public` |
| `/apps`, `/apps/x86` | `C:\Program Files`, `C:\Program Files (x86)` |
| `/data` | `C:\ProgramData` |
| `/system/windows` | `C:\Windows` |
| `/tmp` | `C:\Windows\Temp` |
| `/mnt/c/<x>` | other `C:\<x>` |
| `/mnt/d` … | `D:\` … |
| anything else | `Z:\<path>` |

User names are lower-cased on the LoOS side. Relative, UNC and `..` paths are rejected
(callers resolve them first). The NT I/O manager (#77) calls this when opening files.
