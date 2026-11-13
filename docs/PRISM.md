# LoOS Prism — macOS compatibility

Prism runs Mach-O programs on LoOS. A macOS process is an ordinary process of the Linux
subsystem (`crates/loos-lin`), so it uses the same address spaces, file descriptors, VFS,
scheduler, signals and windows. The code is in `crates/loos-prism`.

| Piece | Where | Status |
|---|---|---|
| Mach-O parsing: thin x86_64/arm64, universal (fat/fat64), segments, sections, `LC_MAIN`/`LC_UNIXTHREAD`, dylibs, rpaths | `macho.rs` | working |
| Fixups: `LC_DYLD_INFO` rebase/bind/lazy-bind opcodes, `LC_DYLD_CHAINED_FIXUPS` (`PTR_64`, `PTR_64_OFFSET`), export trie with re-exports | `macho.rs` | working, including arm64e (`PTR_ARM64E`, `USERLAND`, `USERLAND24`) |
| Loader: bundled dylibs via `@executable_path`/`@loader_path`/`@rpath`, initializers (`__mod_init_func`, `__init_offsets`), TLV templates | `loader.rs` | working |
| libSystem in Rust: stdio/printf family, malloc, string/memory, POSIX file I/O, env, time, atexit, Mach port names | `libsys.rs` | 136 functions; anything else stops the program with "X is not implemented by Prism's libSystem" |
| Darwin syscalls (BSD class 2, Mach traps, `thread_set_tsd_base`), errno/flags/`stat64` translation, carry-flag convention | `darwin.rs` | common calls; Mach IPC returns `MACH_SEND_INVALID_DEST` |
| Apple Silicon: AArch64 interpreter (integer, loads/stores, LSE atomics, exclusives, scalar FP, common SIMD), arm64e pointer authentication | `arm64.rs` | runs Apple's own arm64e tools |
| `.app` bundles (`Info.plist` XML, `CFBundleExecutable`) | `bundle.rs` | working (binary plists not yet) |

## How it fits in

- The kernel's Linux boot path (`crates/kffi/src/lin.rs`) hands any Mach-O file or `.app` given
  as `loos.lin=` to `Prism::spawn`.
- Imports from `/usr/lib/*` and `/System/Library/*` bind to 16-byte stubs at `0x7000_0000_0000`
  (`mov r10, rcx; mov eax, 0x4C50_0000+i; syscall; ret`). Intel code runs natively in ring 3 and
  traps into `Prism::syscall`. The ARM64 interpreter treats a branch into that range as the same
  call.
- Process start goes through the `__prism_start` stub: it runs initializers, then calls
  `main(argc, argv, envp, apple)`. `main` returns to `__prism_main_ret`, which runs the atexit
  handlers and exits.

## arm64e

Apple's own binaries ship as x86_64 + arm64e. Prism loads arm64e code without PAC keys:
signed pointers in chained fixups are written unsigned, `PAC*` leave a pointer unchanged,
`AUT*`/`XPAC*` are no-ops, and `BRAA`/`BLRAA`/`RETAA`/`LDRAA` branch or load without
checking. A program cannot tell the difference unless it inspects pointer bits itself.

## Not done yet

- Threads: one thread per Prism process. pthread mutexes succeed immediately, and
  `pthread_create` is not provided.
- Objective-C/Swift runtimes, CoreFoundation, Foundation, AppKit and CoreGraphics. Programs that
  link them load, but stop at the first unimplemented import.
- GUI. The plan is AppKit windows as LoOS windows through the existing win32k/compositor path
  that Linux Wayland windows use.
- The ARM64 interpreter has no JIT. QEMU TCG and Dynarmic are C/C++ and do not fit the no_std
  kernel crate.

## Tests

- `cargo +1.97.1 test -p loos-prism`: unit tests, plus real binaries built from
  `tests/prism/*.c` with clang and rustup's `ld64.lld`. The arm64 program runs end to end on the
  interpreter.
- `PRISM_MAC_BIN=<dir> cargo +1.97.1 test -p loos-prism`: Apple's arm64e `pwd`, `printenv`,
  `dirname`, `cksum`, `sum`, `xxd` and `getopt` (copied from macOS, not stored here) on the
  interpreter, with output compared against macOS.
- `boot/prism_macos_test.sh`: the same Apple tools natively (x86_64 slice) on LoOS under QEMU,
  copied from a Mac over ssh (`PRISM_MAC=user@host`).
- `boot/prism_test.sh`: boots LoOS in QEMU and runs the Intel build natively, the arm64 build on
  the interpreter, a universal `.app`, and an app with a bundled dylib.
