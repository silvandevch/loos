# Phase 2: fonts and text → Rust

Port `src/shell/raster.c` (glyph outline rasterizer, 114 lines), `src/shell/font.c` (glyph lookup,
advances, kerning/fit/index_at, bitmap cache, 154 lines), `src/shell/text.c` (`fb_text`, `fb_text_ex`,
`shell_text_width*`, `shell_ellipsize`, `shell_text_fit`, `shell_text_index_at`) and the font data
`src/shell/font_data.c` (generated tables) to `crates/loos-shell/src/{raster,font,text}.rs` + a data module.

- Read `src/shell/font.h` and `raster.h` for the public API; keep every exported name.
- `font_data.c` is generated: either port its generator to emit Rust, or convert the tables once with a
  small script under `crates/loos-shell/tools/` (commit the script, not a hand edit). Keep
  `loos_font_regular` / `loos_font_semibold` as `#[no_mangle] static` with the C layout of `loos_font`
  and `loos_glyph`, because `compositor.c`, `taskbar.c`, etc. still reference them.
- The glyph bitmap cache is static mutable state: wrap it in a small type with a single
  `// SAFETY:` explanation (the shell is single-threaded), or pass it explicitly.
- `text.c` used local copies of `dv`/`fb_clip_dev`; in Rust reuse `Canvas` from `draw.rs`.
- Tests: text width of "Hello" at size 16 matches the C value (capture it before deleting the C),
  ellipsize keeps valid UTF-8, `index_at` round-trips with `fit`.

Done when the C files above are gone, all verify steps pass, and text looks identical in `./loos qemu`.
