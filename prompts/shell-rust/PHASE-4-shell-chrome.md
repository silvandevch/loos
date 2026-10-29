# Phase 4: taskbar, Start, notifications → Rust

Port `src/shell/taskbar.c` (taskbar layout centered/left/top, size 40/48/56, app tiles, Quick Settings
and calendar flyouts, Alt+Tab, Super), `src/shell/launcher.c` (Start: search, pinned grid, results,
fuzzy score), `src/shell/notify.c` and `src/shell/inventory.c` to Rust modules of the same names.

- Keep the public API in `shell.h` (`taskbar_*`, `launcher_*`, `fuzzy_score`, `shell_app_color`,
  `shell_draw_app_tile`, `TASKBAR_H` / `shell_taskbar_h`, `QS_*` actions, notify API in `notify.h`).
- `shell_taskbar`, `shell_launcher`, `shell_inventory` stay `#[repr(C)]` while `desktop.c` and
  `settings.c` are C.
- Callbacks (`on_quick`, `on_net_toggle`, `launch`) are C function pointers.
- Privacy rules from `docs/UI-CUSTOMIZATION.md` stay: Start has no web results, no feed, no ads.
- Rust tests: centered layout math, flyout hit areas, calendar weekday of 2026-10-01 (Thursday),
  fuzzy score ordering ("set" ranks Settings pages).

Done when the four C files are gone and `taskbar_test`, `notify_test`, `apps_test` pass unchanged.
