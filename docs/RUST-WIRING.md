# Rust ↔ C wiring

All Rust crates are reachable from the C code through **one static library with a C ABI**: `crates/ffi` (`loos-ffi`) → `libloos_ffi.a`, declared in `src/rust/loos_rust.h`, exposed to CMake as the imported target **`loos_rust`** (`src/rust/CMakeLists.txt` runs `cargo +1.97.1 build --release --offline -p loos-ffi`). Hosted builds only; the freestanding kernel preset (`boot/build-kernel.sh`, `CMAKE_SYSTEM_NAME=Generic`) has no cross-compiled Rust target (`x86_64-unknown-none` is not installed) and keeps the C implementations.

Toolchain: `rust-toolchain.toml` now pins **1.97.1** (1.63 cannot build the workspace). `LOOS_RUST_TOOLCHAIN=""` makes CMake use whatever `cargo` resolves.

## Where Rust replaced C (same symbols/headers, existing C tests pass unchanged)
| C module | Rust crate | notes |
|---|---|---|
| `src/security/crypto.c`, `kdf.c` (SHA-256, HMAC, PBKDF2) | `loos-security` | `loos_crypto` becomes an INTERFACE target over `loos_rust`; `crypto.h` uses an opaque 128-byte context when `LOOS_RUST_CRYPTO` is defined. Also adds AES-XTS/GCM entry points. Freestanding builds still compile the C files. |
| `src/fs/iso9660/iso9660.c` | `loos-fs` (ISO 9660 + Joliet + Rock Ridge) | same `loos_iso9660_*` API; `loos_iso9660_unmount()` added (frees Rust state). Two fixes went into the crate to match the C reader's contract: both-endian block-size check, and a damaged root directory no longer blocks mounting. Rust lists names without `;1`; the shim strips an explicit `;N` suffix from lookups. |
| `deb_version_compare` in `src/pkg/deb.c` | `loos-dpkg` (`compare_lenient`) | validated by the dpkg-generated case table in `tests/pkg`. |

## New C entry points for crates that had no C counterpart (tested in `tests/rust/ffi_test.c`, 96 checks)
`loos-kdump` crash dumps · `loos-tasksched` · `loos-timesvc` (SNTP, POSIX TZ) · `loos-kms` (EDID, CVT, mode pick) · `loos-winget` · `loos-msix` · `loos-msi` · `loos-pkgstate` (trust gate, ownership db) · `loos-vt` · `loos-kernel` (object manager + syscalls) · `loos-ksync` (dispatcher) · `loos-kmem` (buddy, VAD address space, working set) · `loos-registry` (hives) · `loos-khal` (IDT gate / GDT builders) · `loos-drivers` (PS/2 keyboard + mouse decoders) · `loos-vfs` + `loos-fs` (FAT/ISO/NTFS mount manager) · `loos-btrfs-ops` (read side, `tests/rust/ffi_btrfs_test.c` against a real `mkfs.btrfs -r` image).

## Deliberately left in C (and why)
- **`src/pkg/solver.c`, `apt.c`, `pgp.c`, `dpkgdb.c`**: broader than the Rust equivalents (multiarch, pins, holds, autoremove, upgrade-all, detached signatures). Rust `loos-dpkg`'s resolver is greedy; use it only through `loos_dpkg_plan`.
- **`src/fs/btrfs/*` (reader + writer + mkfs)**: the C code has the write path; `loos-btrfs-ops` is read/inspect/tools only, and cannot open the hand-made image from `tests/fs/btrfs/make_fixture.py` (it validates more strictly). Switch once Rust gains write + mkfs.
- **`src/fs/diskfmt` (GPT, streaming FAT32 writer)**: no-allocation callback API used by the kernel installer; `loos-fs` has no GPT and a different (path based) FAT API.
- **`src/kernel/core.c`, `mm.c`** (run queue, syscall table, section/view/COW): complementary to `kmem`/`ksync`/`loos-kernel`, not duplicates; the kernel C code does not yet call the Rust executive.
- Everything freestanding (the boot kernel) until a `x86_64-unknown-none` target + allocator hook exist; then add a `kernel` feature to `loos-ffi` instead of the std allocator.

## Adding an export
1. `#[no_mangle] pub unsafe extern "C" fn loos_…` in the right `crates/ffi/src/*.rs` (null-check, no panics on valid input, document the contract).
2. Declare it in `src/rust/loos_rust.h` (layout of any `#[repr(C)]` struct must match byte for byte).
3. Add a check to `tests/rust/ffi_test.c`. Verify: `cmake --build <dir> --target rust_ffi_test && ctest -R rust.`

## Verified
Hosted build with clang: `rust.ffi`, `rust.ffi_btrfs`, iso9660, kdf, crypto_block, all `shell.*`, all `pkg.*` pass. Known failures that are not caused by this work: `rust.workspace` (2 host-dependent btrfs-ops tests: kernel-header ABI drift, swapfile), `boot.install_uefi` ("selected disk cannot be used", installer/diskfmt in flux), `win32.*` PE image tests (the build script picks the old 1.63 toolchain's lld).
