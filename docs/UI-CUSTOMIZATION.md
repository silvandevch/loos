# Shell customization, performance, KVM, display class

## Theme engine (`crates/loos-shell/src/theme.rs`, Settings → Personalize)
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

## Looks (`crates/loos-shell/src/look.rs`, Settings → Looks)
A **Look** is a named bundle of config keys: theme (`ui.base`, accent, tint, radius, title height, border, shadow, glow, opacity, icon shape, wallpaper), taskbar (`ui.taskbar.center|labels|top|size`) and window manager (`wm.tiling|gap|focus`, `ui.reduce_motion`, sometimes `ui.scale`). One click writes the keys, clears per-token colour overrides and records `ui.look=<id>`; everything stays editable afterwards, key by key. Shipped: **LoOS** (default), **Classic panel** (left bar with titles), **Floating dock** (big round icons, aurora), **Tiler** (top bar, no decoration, tiling + focus-follows-mouse), **Paper** (light), **Midnight** (OLED black), **Lean** (flat, no shadow/glow/motion: cheapest to draw), **Large print** (high contrast, 150%, big titles). Cards show a live miniature (wallpaper, window, bar position, icon alignment). Add a Look by adding one entry to `LOOKS`; a test checks ids are unique and values fit the config store.

## Behavior (Settings → Behavior)
| config key | range | meaning |
|---|---|---|
| `wm.tiling` | bool | master/stack tiling on (Super+T also flips it and the key follows) |
| `wm.gap` / `wm.master` | 0-24 / 30-70 | gap between tiles / main window width % |
| `wm.focus` | 0, 1 | click to focus / focus follows the mouse |
| `ui.night` | 0-100 | night light: scales green and blue down on the final image, once, for the dirty rectangle only (no per-window cost, 0 = no work at all) |
| `ui.reduce_motion` | bool | no animations |

The page also lists every keyboard shortcut. All `ui.*` and `wm.*` keys can be pre-seeded from the machine config (`desktop_apply_config`). Not done: user-defined key bindings, per-app Looks, clock format, auto-hide taskbar, icon packs, wallpaper images.

## Compositor performance
`paint()` now does **occlusion culling**: it starts at the topmost opaque window that fully covers the damaged rect (excluding rounded corners and NOCHROME windows), skipping the wallpaper gradient and every window below. Repainting inside a maximized window measured 38 ms → 2.7 ms (optimized build; 72 → 9 ms unoptimized) for a 1280x720 full repaint. The `no_cull` comparison tests in `compositor.rs` prove output is pixel-identical to the uncached path (`comp->no_cull`) across overlap, maximize, corner, minimize and taskbar cases. `ui.glow=0`/`Performance` preset drops the per-pixel glow math entirely.

## KVM
`scripts/qemu-accel.sh [args|name|status]`, `./loos kvm`. `LOOS_KVM=auto` (default for `boot/run-desktop.sh`) uses KVM when `/dev/kvm` is usable, `on` requires it, `off` forces TCG (default for the reproducible boot tests; `LOOS_KVM=on boot/bios/boot_test.sh` runs them under KVM — verified passing). Other scripts still accept `QEMU_ACCEL` directly.

## Display class core (`crates/kms`, task #228)
DRM/KMS-shaped model so Linux userspace and the compositor share one driver interface: EDID base-block parser (identity, DTDs, established/standard timings), CVT reduced-blanking generator (matches `cvt -r` for 1920x1080 and 1024x768), mode picking/sorting, atomic-commit validation (CRTC/connector routing, fb pitch/size/format, src/dst rects, scaling, bandwidth), connector→CRTC bipartite assignment, multi-monitor layout (overlap/isolation/primary checks, cursor clamping, DPI → scale suggestion). Not done: EDID extension blocks (CEA/DisplayID, HDR, audio), VRR, planes beyond primary, the miniport glue and actual scanout. Test: `cargo +1.97.1 test -p loos-kms`.

## Verify
`cargo +1.97.1 test -p loos-shell` and `cargo +1.97.1 test -p loos-kms`.

## Taskbar and Start (Windows 11 style, without the parts people dislike)

- Keys: `ui.taskbar.center` (default on), `ui.taskbar.labels` (window titles next to icons, off),
  `ui.taskbar.top` (bar at the top). All three are in Settings > Personalize > Taskbar and apply live.
- Start: search box, pinned app grid, local account footer. No web results, no feed, no ads,
  no "recommended" list, no online account.
- Windows: title bar blends with the window body, 46 px caption buttons, red close hover,
  monogram app icons (`shell_draw_app_tile`).
- Performance: the wallpaper is rendered once into a cache (`comp_set_wallpaper_cache`) and
  copied on repaint; it re-renders only when the theme or screen size changes.

## Command palette actions and QEMU
Start (Super / Ctrl+Space) now also finds **Files**, **Settings**, one `Look: <name>` entry per Look and toggles for tiling, focus-follows-mouse, night light, reduce motion and the hard network switch (`look:<id>` / `act:<name>` targets, `Desktop::run_action`). `./loos qemu [bios|uefi] --look tiler --night 40` (kernel words `loos.look=<id>`, `loos.night=<0-100>`) boots straight into a Look; verified headless with a QEMU screendump.
