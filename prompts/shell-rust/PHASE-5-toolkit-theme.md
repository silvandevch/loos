# Phase 5: UI toolkit and theme engine → Rust

Port `src/shell/ui.c` (immediate-mode widgets: buttons, toggles, radios, sliders, text fields,
lists, paragraphs, a11y registration, clip stack), `src/shell/theme.c` (theme specs, presets,
HSL, contrast guard, export/import text format, ui.* persistence), `src/shell/util.c` and
`src/shell/perms.c`.

- Keep `ui.h`, `theme.h`, `perms.h` APIs and the `ui_ctx` / `theme_spec` layouts.
- Theme export/import format must stay compatible (existing exported themes import unchanged);
  test round-trips including `opacity`, `icons`, `wallpaper`.
- The contrast guard (WCAG ratio) uses integer math; port it exactly, test against known pairs.
- `ui_paragraph` wraps at spaces and must never split UTF-8 sequences.

Done when the four C files are gone and `ui_test`, `theme_test`, `a11y_test` pass unchanged.
