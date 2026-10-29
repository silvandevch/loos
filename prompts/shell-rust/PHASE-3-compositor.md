# Phase 3: compositor and window manager → Rust

Port `src/shell/compositor.c` (770 lines: windows, z-order, focus, hit testing, move/resize/snap
drag, damage, occlusion culling, wallpaper + cache, window chrome, cursor, overlays, metrics/scale)
and `src/shell/wm.c` (291 lines: monitors, snapping, virtual desktops, tiling) to
`crates/loos-shell/src/{compositor,wm}.rs`.

- `shell_comp` and `shell_window` are read/written directly by C code that is still C
  (taskbar, launcher, apps, desktop). Keep both structs `#[repr(C)]` and layout-identical (add a
  size/offset test against values printed from a tiny C program in `tests/shell/`).
- Overlays and `on_change` are C function pointers: store them as `Option<unsafe extern "C" fn(...)>`.
- Keep every `comp_*` and `wm_*` symbol from `shell.h` / `desktop.h`.
- Performance must not regress: run `perf_test` (in the ctest set) before and after; occlusion culling,
  wallpaper cache and damage merging must behave the same.
- Rust tests: hit testing of caption buttons (46 px each, close rightmost), snap rects, damage after move.

Done when `compositor.c` and `wm.c` are gone and all verify steps pass.
