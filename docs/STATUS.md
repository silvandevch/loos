# LoOS — Codebase Status (honest snapshot)

Plain-English report of what the code in this repo actually is right now: what it
can do, what it cannot do, what is built-and-tested vs. modelled vs. not started.

Last updated: 2026-10-18.

## The one-sentence version

LoOS today is a **set of real, host-tested C libraries** (a filesystem reader, a
PE loader, an API-set resolver, a version-compat engine, Win32 system DLLs, and
four Win32 desktop apps) plus a **working study lab that boots an NT 5.2
(Server 2003) ISO**. It is **not yet a bootable OS of its own** — there is no
kernel entry point, no bootloader output, and no LoOS ISO.

## What is real and tested

| Area | Code | Evidence |
|------|------|----------|
| Build system | `CMakeLists.txt`, `CMakePresets.json`, `cmake/` | 4 presets configure + build clean: `host-clang`, `kernel-clang`, `windows-msvc`, `windows-mingw` |
| Tests | `tests/` | `ctest --preset host-clang` → **15/16 pass** (only `crypto_block_test` fails) |
| btrfs read path | `src/fs/btrfs/btrfs.c` (1,822 lines) | superblock, chunk tree, root tree, inode/dir items, extents, path resolution, `read_file`, xattr get; tested against a deterministic fixture and a real `mkfs.btrfs` image |
| btrfs write path | `src/fs/btrfs/btrfs.c` | transaction begin / CoW tree / publish root / commit / abort, set_xattr; exercised by `test_btrfs.c`. Partial: no general data-extent allocation yet |
| block layer | `src/io/block.c` | aligned read/write/flush over a device abstraction; tested |
| crypto | `src/security/crypto.c` (179 lines) | real SHA-256; constant-time compare has a length-boundary bug (see below) |
| PE loader | `src/win32/pe/pe.c` (646 lines) | PE parser, relocator, import binder; tested against real Wine DLLs |
| ntdll loader | `src/win32/ldr/ldr.c` (470 lines) | maps + imports images; tested |
| API sets | `src/win32/apisets/apiset_table.c` (740 lines) | `api-ms-win-*` → host DLL resolver; tested |
| compat engine | `src/win32/compat/compat.c` (329 lines) | per-era version claims (Win7/10/11); tested |
| Win32 DLLs | `src/win32/{ntdll,kernel32}`, `scripts/win32/build-pe.sh` | builds real PE32+ `ntdll.dll` + `kernel32.dll` (+ `kernelbase.dll`) via clang + LLD |
| Desktop apps | `apps/` (19,669 lines) | `LoCalc.exe`, `LoNotepad.exe`, `LoTaskMgr.exe`, `LoCmd.exe` — real Win32 GUI/console apps; `LoCmd.exe` verified running under Wine |
| Executive object model | `crates/loos-kernel` | generation-safe handles, process/thread/event objects, named namespaces, security descriptors, and first syscall dispatcher; five host tests pass |
| Registry hive | `crates/loos-registry` | bounded `regf`/`hbin` load/save, key/value API, standard list-cell parsing, and three round-trip/corruption tests |

Counts: `src/` is ~10,800 lines of C; `apps/` is ~19,700 lines. Only **17**
stub/TODO markers exist and **3** real "not implemented" notes (ntdll
`ldr_api.c` PATH/SxS/KnownDlls; kernel32 `strings.c` locale collation; kernel32
`message.c` `FORMAT_MESSAGE_FROM_HMODULE`).

## What it CAN do right now

- Build every component on Linux with clang/CMake/Ninja, and build the four
  Windows apps with MinGW.
- Build real `ntdll.dll` / `kernel32.dll` PE32+ images exporting their function
  surface (via `lld-link`).
- Read a btrfs filesystem in userspace (host tests) — including a real
  `mkfs.btrfs` image.
- Parse and relocate real PE files (imports, relocations) — proven against Wine's
  own system DLLs.
- Resolve `api-ms-win-*` API-set names to host DLLs and apply
  version-compatibility claims per OS era.
- Run the LoOS desktop apps on Windows / Wine (they are ordinary Win32 apps).
- Boot the **NT 5.2 study tree** as an ISO in QEMU (`./loos iso`, `./loos qemu`).

## What it CANNOT do yet

- **Boot itself.** No kernel entry point, no linker script, no bootloader image,
  no LoOS ISO. `./loos source-build kernel-clang` says it plainly: *"freestanding
  kernel libraries built; no boot-image target exists yet."* `./loos qemu` boots
  the NT 5.2 study ISO, not LoOS.
- **Run a PE program on LoOS.** The loader is real and tested, but there is no
  running OS for it to load into.
- **Show any UI of its own.** There is no `user32`/`gdi32`/compositor code in
  `src/win32` — the apps link the *real* Windows user32/gdi32. LoOS's own shell,
  window manager, and compositor are not started.
- **Networking, drivers, full security, and Linux ("Lin") support.** Registry
  now has a host-testable hive layer, but it is not yet wired into btrfs or the
  Win32 `advapi32` service path.
- **A fully green test suite.** `security.crypto_block_test` fails on the
  constant-time length boundary (task #86).

## The two things called "kernel" — don't mix them up

1. **`src/kernel/` (LoOS kernel model):** `core.c` (153 lines) and `mm.c` (179
   lines) are **portable, host-testable C** — a run-queue/priority scheduler, a
   syscall table, a user-range validator, and a section/VM page-fault model.
   They pass host tests. They are **not an x86-64 kernel**: there is no paging
   setup, no IDT, no syscall instruction, no `_start`, and no arch code. The
   `kernel-clang` preset only proves these files compile freightanding-clean.
2. **The NT 5.2 study tree** (`$SRC_ROOT/srv03rtm`): a full, *external* Windows
   Server 2003 source tree that the prep/build/ISO/QEMU pipeline turns into a
   bootable ISO. That is the study lab, not LoOS.

## Virtue signals

- No fabricated capability: the code is genuinely implemented, not stubs.
- Tests are real and mostly pass; the one failure is a known, task-tracked bug.
- The gap is honest: the "OS" is a library suite + study lab, not yet an OS.

## Near-term critical path

1. **#86** fix the crypto constant-time boundary bug (make the suite green).
2. **#9 (done) → #5/#6/#10** stand up a real freestanding kernel: arch code,
   bootloader image, linker script, and a banner in QEMU.
3. **#21 → #31** wire the existing PE loader into a running environment so the
   first real `.exe` executes.
4. **#46/#34** first LoOS UI (compositor rectangle + window), which every other
   visual deliverable depends on.
5. **#88–#91 (Lin L0/L1/L2)** native ELF loader + Linux syscall table so an
   unmodified static Linux binary runs.
6. **Package managers** (`pkgd`, `pkgwinget`, `pkgapt`, `pkgcli`) — see
   [PACKAGING.md](PACKAGING.md) — then the installer/update path
   (`instcore`, `updatesvc`) — see [INSTALL.md](INSTALL.md).

## Scope note (full-OS backlog)

The board now also tracks the work to make LoOS a *complete* OS: HAL and full
virtual memory, real drivers (storage/USB/display/audio/NIC), filesystem drivers,
networking + firewall, security (accounts/logon/UAC/disk encryption/code
signing), the two native package managers, the installer + recovery + OTA
updates, and the long tail of Win32/system-app/accessibility/diagnostic work.
See docs/DEVELOPMENT.md §"Full-OS backlog (M11+)" and docs/AGENTS.md for the
category/owner routing.
