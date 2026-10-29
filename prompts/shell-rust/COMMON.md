# LoOS shell → Rust: rules for every phase

You are porting the LoOS desktop shell from C (`src/shell/`) to Rust (`crates/loos-shell/`),
one phase at a time. Repo root: `/run/media/silvan/DATA/LoOS`. Read `docs/LANGUAGE.md`,
`docs/RUST-WIRING.md`, `docs/UI-CUSTOMIZATION.md` and the C files you port before writing.

## How phase 1 did it (copy this pattern)
- Safe Rust core in a module (`crates/loos-shell/src/draw.rs`): types are `#[repr(C)]` and match
  the C structs byte for byte (`Rect` = `shell_rect`, `Theme` = `shell_theme`, `Damage` = `shell_damage`).
- `crates/loos-shell/src/ffi.rs`: `#[no_mangle] extern "C"` functions with the **exact C names and
  signatures** from `src/shell/shell.h`. Null-check pointers, build slices from (ptr, len), call the
  safe core. `unsafe` only here, every block with a short `// SAFETY:` comment.
- Delete the ported `.c` file (`git rm`), drop it from `src/shell/CMakeLists.txt` and from the
  `for f in ...` shell list in `boot/build-kernel.sh`. Headers stay; C callers do not change.
- The crate is `#![no_std]` (+ `alloc` if needed). It is linked twice: hosted via `crates/ffi`
  (`pub use loos_shell;`) into `libloos_ffi.a`, and into the boot kernel via `crates/kffi`
  (`x86_64-unknown-none`, soft-float: **no f32/f64**, use fixed point like the C code does).

## Rules
1. Same behavior, pixel for pixel where tests check pixels. Port logic faithfully; do not redesign.
2. No new dependencies. No `std`. No floats. No panics on valid input (kernel has panic = abort).
3. Keep comments short. Match the surrounding code style.
4. Structs that C code still reads or writes must stay `#[repr(C)]` with identical layout.
   When C code still owns a struct you need, mirror it in Rust with a layout test
   (`core::mem::size_of` / offsets) instead of changing it.
5. When a ported function calls C that is not ported yet, declare it in an `extern "C"` block.
6. Add Rust unit tests for the core logic (`cargo test -p loos-shell`).
7. Update `docs/RUST-WIRING.md` (table "Where Rust replaced C") and the status row in
   `prompts/shell-rust/README.md`.
8. One writer per file: claim the task on the bus first (`./loos bus task add/claim`), report when done.
   Don't burn tokens grepping everything — read the files listed in your phase, build, test, fix.

## Verify (all must pass before you report)
```bash
cd crates && cargo test -p loos-shell && cargo build --release --offline -p loos-kffi --target x86_64-unknown-none && cd ..
B=build-rust; cmake -S . -B $B -DCMAKE_C_COMPILER=clang && cmake --build $B -j8
ctest --test-dir $B -R "shell|rust.ffi$"          # 14 suites, all pass
bash boot/bios/boot_test.sh && bash boot/uefi/boot_test.sh
bash boot/bios/bugcheck_test.sh                  # stop screen still renders through the shell drawing code
./loos qemu                                      # look at it: desktop, Start (Super), Quick Settings (tray LED), calendar (clock)
```
Report: what moved, line counts C removed / Rust added, test results, anything left in C and why.
