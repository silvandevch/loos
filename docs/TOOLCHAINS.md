# LoOS — compilers and language toolchains

How you build software *for* LoOS: the target triples, the base C library, and
the compilers we port. Cross toolchains let you build today on Linux; **native**
(self-hosted) toolchains come later so LoOS can build its own software.

## Targets

| Triple | Output | Runs on | Notes |
|---|---|---|---|
| `x86_64-loos` (ELF) | native LoOS program | LoOS | links LoOS libc + [LoOS API](API.md) |
| `x86_64-loos-kernel` | freestanding kernel/objects | LoOS kernel | `no_std`/freestanding, no libc |
| `x86_64-pc-windows-msvc` | PE32/PE32+ | LoOS Win32 layer | for the subsystem + Windows apps |
| `x86_64-unknown-linux-gnu` | ELF | LoOS via [Lin](LINUX-COMPAT.md) | stock Linux binaries |
| host | ELF | build machine | tests + tools |

## The base: LoOS libc

Everything above needs a C library (`libc-loos`): the POSIX-ish subset CPython,
GCC's runtime, and most ports expect — `malloc`, `stdio`, `pthread`, `dirent`,
`sys/stat`, `time`, `dlopen`, termios, locale stubs. It is implemented **on top
of the [LoOS API](API.md)**. Rust `std`, G++, and CPython all stand on it.

## Compilers (port order)

1. **binutils + GCC (`x86_64-loos-gcc`)** — the base C toolchain, built against
   `libc-loos`. Everything else can then be built *on* LoOS.
2. **G++ / libstdc++** — C++ support.
3. **clang + lld** — LLVM native, targeting `x86_64-loos`; also the compiler for
   the `no_std` Rust kernel path.
4. **rustc target `x86_64-loos`** — `no_std` already works for the kernel; `std`
   arrives once `libc-loos` is complete (Rust `std` needs libc/pthread/fs).
5. **Optional frontends** (deferred, only as needed): Zig, Go, Fortran, Ada.
6. **C#/.NET** — **deferred on purpose**; listed only as an example, not planned.

### Cross vs native

- **Cross** (build on Linux, run on LoOS): first, and enough for development and
  CI. Ships as `loos-<lang>` toolchains in the SDK.
- **Native/self-hosted** (build on LoOS for LoOS): follows from 1–4. The
  milestone is `gcc` compiling a LoOS program *while running on LoOS*.

## Linking

- Native LoOS apps are **ELF64** against `libc-loos` + LoOS API
  ([API.md](API.md)); dynamic by default with versioned symbols.
- Kernel/objects are freestanding and static.
- PE is produced only for the Win32 compat target.

## Scripting

- **Python — native, first-class** (see [PYTHON.md](PYTHON.md)): a real
  `python3` on LoOS, plus `pip`/`venv` through the package service.
- Shell: `LoCmd` (cmd-compatible) and, via Lin, `bash`.

## Tasks

`libc-loos`, `gcc-port`, `gpp-port`, `clang-port`, `rust-target`, `langs-optional`,
`csharp-deferred`, `python` (+ `python-stdlib`, `python-pip`, `python-capi`,
`python-embed`), `native-toolchain`.
