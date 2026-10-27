# Shell customization, performance, KVM, display class

## Theme engine (`src/shell/theme.[ch]`, Settings → Personalize)
A theme is a `theme_spec`: base palette (`dark light hc amoled nord dracula solarized gruvbox paper`) + accent + surface **tint** + shape/effects + per-token color overrides. It resolves in order base → tint → overrides → **contrast guard**, then `theme_apply()` pushes palette and metrics to the compositor live.

| config key | range | meaning |
|---|---|---|
| `ui.base` | base name | palette (legacy `ui.theme` 0/1/2 still honored) |
| `ui.accent_hex` | `#RRGGBB` | accent (legacy `ui.accent` index is the fallback) |
| `ui.tint` | 0-100 | lean surfaces/borders toward the accent hue |
| `ui.radius` | -1..24 | window corner radius (-1 = default 8) |
| `ui.title_h` / `ui.border` | 20-48 / 1-4 | title bar height / border width (0 = default) |
| `ui.shadow` / `ui.glow` | 0-150 | % of drop shadow / wallpaper glows (0 glow = flat gradient, cheapest) |
| `ui.guard` | bool | keep text ≥4.5:1, dim text ≥3:1, accent text ≥3:1 against their backgrounds |
| `ui.c.<token>` | `#RRGGBB` | override one of `desktop surface surface_hi border text text_dim accent accent_text danger title_inactive` |

12 presets (LoOS Dark/Light, High contrast, AMOLED, Nord, Dracula, Solarized, Gruvbox, Paper, Glass, Compact, Performance). Themes export/import as one line: `base=nord;accent=#88C0D0;radius=10;c.surface=#101820` (`;` or newline separated; invalid entries are counted and skipped; numbers are clamped). The Personalize page has an HSL accent picker, sliders, a token swatch row with hex override, reset, and export/import. High contrast always wins over a custom theme. Not done: per-app themes, wallpaper images, icon packs, animation curves, fonts as a theme field.

## Compositor performance
`paint()` now does **occlusion culling**: it starts at the topmost opaque window that fully covers the damaged rect (excluding rounded corners and NOCHROME windows), skipping the wallpaper gradient and every window below. Repainting inside a maximized window measured 38 ms → 2.7 ms (optimized build; 72 → 9 ms unoptimized) for a 1280x720 full repaint. `tests/shell/perf_test.c` proves output is pixel-identical to the uncached path (`comp->no_cull`) across overlap, maximize, corner, minimize and taskbar cases. `ui.glow=0`/`Performance` preset drops the per-pixel glow math entirely.

## KVM
`scripts/qemu-accel.sh [args|name|status]`, `./loos kvm`. `LOOS_KVM=auto` (default for `boot/run-desktop.sh`) uses KVM when `/dev/kvm` is usable, `on` requires it, `off` forces TCG (default for the reproducible boot tests; `LOOS_KVM=on boot/bios/boot_test.sh` runs them under KVM — verified passing). Other scripts still accept `QEMU_ACCEL` directly.

## Display class core (`crates/kms`, task #228)
DRM/KMS-shaped model so Linux userspace and the compositor share one driver interface: EDID base-block parser (identity, DTDs, established/standard timings), CVT reduced-blanking generator (matches `cvt -r` for 1920x1080 and 1024x768), mode picking/sorting, atomic-commit validation (CRTC/connector routing, fb pitch/size/format, src/dst rects, scaling, bandwidth), connector→CRTC bipartite assignment, multi-monitor layout (overlap/isolation/primary checks, cursor clamping, DPI → scale suggestion). Not done: EDID extension blocks (CEA/DisplayID, HDR, audio), VRR, planes beyond primary, the miniport glue and actual scanout. Test: `cargo +1.97.1 test -p loos-kms`.

## Verify
`cmake --build <dir> --target theme_test perf_test settings_test` then `ctest -R shell.`; `cargo +1.97.1 test -p loos-kms`.
