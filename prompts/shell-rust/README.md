# Porting the LoOS shell/GUI from C to Rust — prompts for Sonnet

The desktop shell (`src/shell/*.c`, ~5,600 lines plus `font_data.c`) moves to the Rust crate
`crates/loos-shell` one phase at a time. Each phase replaces C modules **behind the same C API**
(`src/shell/*.h`), so the desktop keeps booting and the C tests keep passing after every phase.

Run the phases in order, one prompt per session. Paste `COMMON.md` first, then the phase file.

| Phase | File | Ports | Status |
|---|---|---|---|
| 1 | — | `draw.c` (rects, colors, themes, primitives, damage) | done (`crates/loos-shell/src/draw.rs`, `ffi.rs`) |
| 2 | `PHASE-2-fonts.md` | `raster.c`, `font.c`, `text.c`, `font_data.c` | done (`crates/loos-shell/src/{raster,font,text,font_data}.rs`) |
| 3 | `PHASE-3-compositor.md` | `compositor.c`, `wm.c` | done (`crates/loos-shell/src/{compositor,wm}.rs`) |
| 4 | `PHASE-4-shell-chrome.md` | `taskbar.c`, `launcher.c`, `notify.c`, `inventory.c` | done (`crates/loos-shell/src/{taskbar,launcher,notify,inventory}.rs`) |
| 5 | `PHASE-5-toolkit-theme.md` | `ui.c`, `theme.c`, `util.c`, `perms.c` | done (`crates/loos-shell/src/{ui,theme,util,perms}.rs`) |
| 6 | `PHASE-6-apps.md` | `settings.c`, `filemgr.c`, `taskmon.c`, `session.c`, `installer.c` | in progress (`session`, `taskmon`, `filemgr` done) |
| 7 | `PHASE-7-desktop.md` | `desktop.c`, `apphost.c`, `backend_mem.c`, `compat_console.c`; delete the C shell | todo |
