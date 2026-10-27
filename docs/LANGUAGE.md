# LoOS — language policy (Rust-first)

**Default: Rust.** New code in LoOS is written in Rust unless it falls under one
of the narrow exceptions in §C. C stays only where Rust is a worse fit
(interop-forced ABI, assembly, vendor C-only interfaces, or modules that would be
almost entirely `unsafe` — which defeats Rust's point). This applies to every
layer: kernel, drivers, filesystems, subsystem, services, package managers,
installer, tooling, and tests.

## Why Rust here

LoOS parses a lot of **untrusted input** — PE/COFF images, btrfs metadata, apt
`.deb` control files and winget/MSIX manifests, TLS records, network packets.
Those are exactly where memory-safety bugs are security bugs. Rust gives:

- no buffer overflows / use-after-free / double-free in the kernel and in every
  parser,
- `Result`-based error handling instead of ignored status codes,
- safe concurrency for the services and the network stack,
- ready, audited crates (RustCrypto, `serde`, `nom`, `ring`) instead of
  hand-rolled crypto and parsers.

## Rust targets

| Layer | Target | Mode |
|---|---|---|
| kernel, drivers | `x86_64-unknown-none` | `#![no_std]`, `panic = "abort"`, no unwinding, custom alloc |
| Win32 subsystem DLLs, apps | `x86_64-pc-windows-msvc` | `std` or `no_std`; `extern "C"` exports matching the NT/Win32 contract |
| host tests, CLI tools, build | `x86_64-unknown-linux-gnu` | `std` |

UEFI loader: `x86_64-unknown-uefi`. Boot assembly stays hand-written asm either
way (§C.2).

## Unsafe policy

- Crates `#![forbid(unsafe_code)]` unless they genuinely need it.
- `unsafe` is confined to **small leaf modules** with a `// SAFETY:` comment on
  every block, wrapped in a safe API. Types, invariants, and bounds are encoded
  in the type system so callers never write `unsafe`.
- `#![deny(unsafe_op_in_unsafe_fn)]`, `clippy` and `miri` in CI for host-tested
  crates.
- **The escape hatch:** if a module would be "mostly `unsafe`" — raw byte surgery
  with no invariants worth encapsulating (e.g. operating on an arbitrary program
  pointer from an image, syscall stubs, context switch) — that is a signal to use
  C, not to write a big `unsafe` crate. See §C.

## C is the right choice when (exhaustive)

1. **Vendor/kernel interfaces that are C-only and pointer/union-heavy.**
   WDDM `DXGK` display miniports, the Windows AMD/NVIDIA kernel drivers, CUDA
   driver API, Direct3D UMD. These are defined as C structs and expect C linkage.
2. **Assembly and boot.** UEFI entry, the transition to long mode, IDT/stub
   trampolines, `syscall`/`sysret` entry, context switch, `global_asm`. Written
   as `.S` + minimal C, or Rust `global_asm!` where it stays readable.
3. **Mostly-`unsafe` code by nature.** A module whose body is overwhelmingly
   pointer arithmetic over untrusted bytes with no safe abstraction to offer.
4. **Existing, proven C** that a rewrite would not make safer or faster (kept, not
   defended — see the migration policy).
5. **Toolchain/format constraints** where only a C path exists (e.g. an existing
   freestanding C header set, a CMake-only dependency).

FFI alone is **not** an exception: a Rust `extern "C"` function at the boundary is
fine, so "it has to interop" is never on its own a reason to write C.

## Rust crate layout

```
crates/                 Rust workspace (Cargo)
  loos-kernel/          no_std kernel core (sched, vm, syscalls)
  loos-pe/              PE/COFF parser + loader        (untrusted input)
  loos-btrfs/           btrfs read/write               (untrusted input)
  loos-crypto/          SHA/AES/RNG (RustCrypto)       (untrusted input)
  loos-pkg/             package service (winget+apt+MSIX+MSI)
  loos-manifest/        serde models for winget/deb/MSIX
  loos-net/             TCP/IP stack
  loos-services/        backend services
  loos-win32/           Win32 subsystem surface (extern "C" exports)
  loos-comp/            compositor core
  loos-tools/           CLI: loos-pkg, installer helpers, docs tooling
src/                    C that stays C (see §C)
```

## Supply chain & reproducibility

- Dependencies are **vendored** (`cargo vendor`) and built `--offline`; the
  lockfile is committed. No build script may reach the network.
- `cargo deny` + `cargo vet` in CI; licences checked against
  [SOURCE-POLICY.md](SOURCE-POLICY.md); no telemetry, ever.
- Release builds are reproducible (`--locked --offline`, pinned rustc).

## Migration policy

- **New code: Rust-first.** No new C unless it matches §C.
- **Existing C is rewritten opportunistically**, highest-value first: parsers of
  untrusted input (PE loader, btrfs, package formats), then the network stack,
  crypto, services, compositor core.
- Every module's language decision is recorded in the table below. Rewrites get a
  task (`rust-*`); "stays C" gets a one-line reason.

| Area | Language | Reason |
|---|---|---|
| kernel core, drivers, fs, net, pkg, services, compositor | **Rust** | §Why — untrusted input / safety |
| Win32 subsystem DLLs (ntdll/kernel32/…) | **Rust** (`extern "C"`) | big parsers + handles; C ABI at the edge |
| UCRT compatibility image (`src/win32/ucrt`) | **C** | the freestanding PE build has a C-only variadic `printf` ABI; fixed-width helpers are kept in the same image until the Rust Win32 target is wired |
| boot/entry, IDT/syscall stubs, context switch | **C + asm** | §C.2 |
| WDDM/DXGK, CUDA, D3D UMD host | **C** | §C.1 vendor interfaces |
| current PE loader / btrfs / crypto (until rewritten) | C → **Rust** | tracked by `rust-be`/`rust-btrfs`/`rust-crypto` |
| NT 5.2 study tree | C | external source, read-only |

## Tasks

Toolchain: `rust-toolchain`, `rust-workspace`, `rust-ffi`, `rust-vendor`.
Migration: `rust-kernel`, `rust-pe`, `rust-btrfs`, `rust-crypto`, `rust-net`,
`rust-services`, `rust-pkg`, `rust-comp`, `rust-win32`, `rust-apps`.
"stays C": documented in this file (`keep-c`).
